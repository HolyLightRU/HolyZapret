use crate::packet;
use crate::run::Log;
use crate::windivert::{self, Address, Handle, Lib};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SERVERS: [&str; 4] = ["8.8.8.8", "1.1.1.1", "8.8.4.4", "1.0.0.1"];
pub const FILTER: &str = "inbound and !loopback and udp.SrcPort == 53 and udp.PayloadLength >= 12";
const TIMEOUT: Duration = Duration::from_millis(2500);
const PAUSE_AFTER: u32 = 3;
const PAUSE: Duration = Duration::from_secs(600);
const MIN_TTL: u32 = 30;
const MAX_TTL: u32 = 600;
const NOTE_EVERY: Duration = Duration::from_secs(600);
const CONFIRMED_FOR: Duration = Duration::from_secs(120);
const TYPE_A: u16 = 1;
const TYPE_CNAME: u16 = 5;
const TYPE_AAAA: u16 = 28;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: u16,
    pub name: String,
    pub qtype: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Fine,
    Missing,
    Refused,
    Bogus,
}

impl Verdict {
    pub fn text(self) -> &'static str {
        match self {
            Verdict::Fine => "ответ в порядке",
            Verdict::Missing => "провайдер ответил, что такого сайта нет",
            Verdict::Refused => "провайдер отказался отвечать",
            Verdict::Bogus => "провайдер подсунул ненастоящий адрес",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    pub rcode: u8,
    pub addrs: Vec<IpAddr>,
    pub cname: bool,
    pub min_ttl: Option<u32>,
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(at)?, *d.get(at + 1)?, *d.get(at + 2)?, *d.get(at + 3)?]))
}

fn read_name(msg: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *msg.get(at)? as usize;
        if len == 0 {
            return Some((labels.join(".").to_ascii_lowercase(), end.unwrap_or(at + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = (be16(msg, at)? & 0x3FFF) as usize;
            end.get_or_insert(at + 2);
            jumps += 1;
            if jumps > 16 || ptr >= msg.len() {
                return None;
            }
            at = ptr;
            continue;
        }
        if len > 63 {
            return None;
        }
        let label = msg.get(at + 1..at + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        at += 1 + len;
    }
}

pub fn question(msg: &[u8]) -> Option<Question> {
    if msg.len() < 12 || be16(msg, 4)? != 1 {
        return None;
    }
    let (name, at) = read_name(msg, 12)?;
    let qtype = be16(msg, at)?;
    be16(msg, at + 2)?;
    (!name.is_empty()).then_some(Question { id: be16(msg, 0)?, name, qtype })
}

pub fn answers(msg: &[u8]) -> Option<Answers> {
    if msg.len() < 12 || msg[2] & 0x80 == 0 {
        return None;
    }
    let rcode = msg[3] & 0x0F;
    let mut at = 12;
    for _ in 0..be16(msg, 4)? {
        at = read_name(msg, at)?.1 + 4;
    }
    let mut out = Answers { rcode, ..Default::default() };
    for _ in 0..be16(msg, 6)? {
        at = read_name(msg, at)?.1;
        let rtype = be16(msg, at)?;
        let ttl = be32(msg, at + 4)?;
        let len = be16(msg, at + 8)? as usize;
        let data = msg.get(at + 10..at + 10 + len)?;
        match (rtype, len) {
            (TYPE_A, 4) => out.addrs.push(IpAddr::V4(Ipv4Addr::new(data[0], data[1], data[2], data[3]))),
            (TYPE_AAAA, 16) => {
                let mut b = [0u8; 16];
                b.copy_from_slice(data);
                out.addrs.push(IpAddr::V6(Ipv6Addr::from(b)));
            }
            (TYPE_CNAME, _) => out.cname = true,
            _ => {}
        }
        out.min_ttl = Some(out.min_ttl.map_or(ttl, |m| m.min(ttl)));
        at += 10 + len;
    }
    Some(out)
}

pub fn bogus(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            o[0] == 0 || o[0] == 127 || o[0] == 10 || (o[0] == 172 && (16..32).contains(&o[1])) || (o[0] == 192 && o[1] == 168) || (o[0] == 100 && (64..128).contains(&o[1])) || (o[0] == 169 && o[1] == 254)
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            v.is_unspecified() || v.is_loopback() || s[0] & 0xFE00 == 0xFC00 || s[0] & 0xFFC0 == 0xFE80
        }
    }
}

pub fn verdict(msg: &[u8], qtype: u16) -> Verdict {
    let Some(a) = answers(msg) else { return Verdict::Fine };
    match a.rcode {
        3 => Verdict::Missing,
        2 | 5 => Verdict::Refused,
        0 if a.addrs.iter().any(bogus) => Verdict::Bogus,
        0 if qtype == TYPE_A && !a.addrs.iter().any(IpAddr::is_ipv4) => Verdict::Missing,
        _ => Verdict::Fine,
    }
}

pub fn query(name: &str, qtype: u16) -> Vec<u8> {
    let mut q = vec![0, 0, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.').filter(|l| !l.is_empty()) {
        let b = label.as_bytes();
        q.push(b.len().min(63) as u8);
        q.extend_from_slice(&b[..b.len().min(63)]);
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes());
    q
}

pub fn with_id(msg: &[u8], id: u16) -> Vec<u8> {
    let mut m = msg.to_vec();
    if m.len() >= 2 {
        m[..2].copy_from_slice(&id.to_be_bytes());
    }
    m
}

fn matches(resp: &[u8], name: &str, qtype: u16) -> bool {
    question(resp).is_some_and(|q| q.name == name && q.qtype == qtype) && answers(resp).is_some()
}

mod http {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::time::Duration;
    use windows_sys::Win32::Networking::WinHttp::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn session() -> Option<*mut c_void> {
        static SESSION: OnceLock<usize> = OnceLock::new();
        let h = *SESSION.get_or_init(|| unsafe {
            WinHttpOpen(wide("HolyZapret RayOfLight").as_ptr(), WINHTTP_ACCESS_TYPE_NO_PROXY, std::ptr::null(), std::ptr::null(), 0) as usize
        });
        (h != 0).then_some(h as *mut c_void)
    }

    struct Closing(*mut c_void);

    impl Drop for Closing {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    pub fn post(host: &str, body: &[u8], timeout: Duration) -> Option<Vec<u8>> {
        let s = session()?;
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        unsafe {
            let conn = Closing(WinHttpConnect(s, wide(host).as_ptr(), 443, 0));
            if conn.0.is_null() {
                return None;
            }
            let req = Closing(WinHttpOpenRequest(conn.0, wide("POST").as_ptr(), wide("/dns-query").as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
            if req.0.is_null() {
                return None;
            }
            WinHttpSetTimeouts(req.0, ms, ms, ms, ms);
            let headers = wide("Content-Type: application/dns-message\r\nAccept: application/dns-message\r\n");
            if WinHttpSendRequest(req.0, headers.as_ptr(), u32::MAX, body.as_ptr() as *const c_void, body.len() as u32, body.len() as u32, 0) == 0 {
                return None;
            }
            if WinHttpReceiveResponse(req.0, std::ptr::null_mut()) == 0 {
                return None;
            }
            let mut code = 0u32;
            let mut size = 4u32;
            let ok = WinHttpQueryHeaders(
                req.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                &mut code as *mut u32 as *mut c_void,
                &mut size,
                std::ptr::null_mut(),
            );
            if ok == 0 || code != 200 {
                return None;
            }
            let mut out = Vec::new();
            let mut buf = vec![0u8; 4096];
            loop {
                let mut read = 0u32;
                if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read) == 0 {
                    return None;
                }
                if read == 0 || out.len() > 65535 {
                    break;
                }
                out.extend_from_slice(&buf[..read as usize]);
            }
            Some(out)
        }
    }
}

pub enum Fix {
    Replaced(Vec<u8>, Question, Verdict),
    Confirmed,
    Failed,
}

pub struct Resolver {
    cache: Mutex<HashMap<(String, u16), (Vec<u8>, Instant)>>,
    confirmed: Mutex<HashMap<(String, u16), Instant>>,
    fails: AtomicU32,
    paused: Mutex<Option<Instant>>,
    pub fixed: AtomicU64,
    pub failed: AtomicU64,
    noted: Mutex<HashMap<String, Instant>>,
    ask: Box<dyn Fn(&str, &[u8]) -> Option<Vec<u8>> + Send + Sync>,
}

impl Resolver {
    pub fn new() -> Resolver {
        Resolver::with(Box::new(|server, body| http::post(server, body, TIMEOUT)))
    }

    pub fn with(ask: Box<dyn Fn(&str, &[u8]) -> Option<Vec<u8>> + Send + Sync>) -> Resolver {
        Resolver {
            cache: Mutex::new(HashMap::new()),
            confirmed: Mutex::new(HashMap::new()),
            fails: AtomicU32::new(0),
            paused: Mutex::new(None),
            fixed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            noted: Mutex::new(HashMap::new()),
            ask,
        }
    }

    pub fn resolve(&self, name: &str, qtype: u16, now: Instant) -> Result<Vec<u8>, bool> {
        let key = (name.to_string(), qtype);
        if let Some((m, until)) = self.cache.lock().map_err(|_| false)?.get(&key) {
            if *until > now {
                return Ok(m.clone());
            }
        }
        if self.confirmed.lock().map_err(|_| false)?.get(&key).is_some_and(|t| *t > now) {
            return Err(true);
        }
        if self.paused.lock().map_err(|_| false)?.is_some_and(|t| t > now) {
            return Err(false);
        }
        let body = query(name, qtype);
        let mut reached = false;
        for server in SERVERS {
            let Some(resp) = (self.ask)(server, &body) else { continue };
            if !matches(&resp, name, qtype) {
                continue;
            }
            if verdict(&resp, qtype) != Verdict::Fine {
                reached = true;
                break;
            }
            let ttl = answers(&resp).and_then(|a| a.min_ttl).unwrap_or(MIN_TTL).clamp(MIN_TTL, MAX_TTL);
            if let Ok(mut c) = self.cache.lock() {
                if c.len() > 4096 {
                    c.retain(|_, (_, t)| *t > now);
                }
                c.insert(key, (resp.clone(), now + Duration::from_secs(ttl as u64)));
            }
            self.fails.store(0, Ordering::Relaxed);
            return Ok(resp);
        }
        if reached {
            self.fails.store(0, Ordering::Relaxed);
            if let Ok(mut c) = self.confirmed.lock() {
                if c.len() > 4096 {
                    c.retain(|_, t| *t > now);
                }
                c.insert(key, now + CONFIRMED_FOR);
            }
            return Err(true);
        }
        if self.fails.fetch_add(1, Ordering::Relaxed) + 1 >= PAUSE_AFTER {
            self.fails.store(0, Ordering::Relaxed);
            if let Ok(mut p) = self.paused.lock() {
                *p = Some(now + PAUSE);
            }
        }
        Err(false)
    }

    pub fn fix(&self, resp: &[u8], now: Instant) -> Option<Fix> {
        let q = question(resp)?;
        let v = verdict(resp, q.qtype);
        if v == Verdict::Fine {
            return None;
        }
        Some(match self.resolve(&q.name, q.qtype, now) {
            Ok(good) => {
                self.fixed.fetch_add(1, Ordering::Relaxed);
                Fix::Replaced(with_id(&good, q.id), q, v)
            }
            Err(true) => Fix::Confirmed,
            Err(false) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                Fix::Failed
            }
        })
    }

    pub fn avoiding(&self, name: &str, qtype: u16, avoid: &[IpAddr]) -> Option<Vec<u8>> {
        let body = query(name, qtype);
        SERVERS.iter().find_map(|server| {
            let resp = (self.ask)(server, &body)?;
            let a = answers(&resp)?;
            let clean = matches(&resp, name, qtype) && verdict(&resp, qtype) == Verdict::Fine && !a.addrs.is_empty() && a.addrs.iter().all(|ip| !avoid.contains(ip));
            clean.then_some(resp)
        })
    }

    fn should_note(&self, name: &str, now: Instant) -> bool {
        let Ok(mut n) = self.noted.lock() else { return false };
        match n.get(name) {
            Some(t) if now.duration_since(*t) < NOTE_EVERY => false,
            _ => {
                n.insert(name.to_string(), now);
                true
            }
        }
    }
}

pub type Wanted = Arc<dyn Fn(&str) -> bool + Send + Sync>;
pub type Avoid = Arc<dyn Fn(&str) -> Vec<IpAddr> + Send + Sync>;

pub fn start(lib: Lib, priority: i16, wanted: Wanted, avoid: Avoid, ended: Arc<AtomicBool>, log: Arc<Log>) -> Result<Arc<Resolver>, String> {
    let handle = Arc::new(lib.open(FILTER, windivert::LAYER_NETWORK, priority, 0).map_err(|e| format!("DNS: перехват не включился: {e}"))?);
    let resolver = Arc::new(Resolver::new());
    log.line(&format!("DNS: чиню ответы провайдера для сайтов из списков через {}", SERVERS.join(", ")));
    flush_cache();
    {
        let h = handle.clone();
        let ended = ended.clone();
        std::thread::spawn(move || {
            while !ended.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(300));
            }
            h.shutter().shutdown();
        });
    }
    let r = resolver.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; windivert::MAX_PACKET];
        while let Some((len, addr)) = handle.recv(&mut buf) {
            let pkt = buf[..len].to_vec();
            handle_packet(&handle, &r, &wanted, &avoid, &log, pkt, addr);
        }
    });
    Ok(resolver)
}

pub fn flush_cache() {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("ipconfig").arg("/flushdns").creation_flags(0x0800_0000).output();
}

pub fn frozen_in(msg: &[u8], avoid: &[IpAddr]) -> bool {
    !avoid.is_empty() && answers(msg).is_some_and(|a| a.addrs.iter().any(|ip| avoid.contains(ip)))
}

fn handle_packet(handle: &Arc<Handle>, r: &Arc<Resolver>, wanted: &Wanted, avoid: &Avoid, log: &Arc<Log>, pkt: Vec<u8>, addr: Address) {
    let Some(u) = packet::parse_udp(&pkt) else {
        handle.send(&pkt, &addr);
        return;
    };
    let payload = u.payload();
    let Some(q) = question(payload).filter(|q| matches!(q.qtype, TYPE_A | TYPE_AAAA)) else {
        handle.send(&pkt, &addr);
        return;
    };
    if verdict(payload, q.qtype) == Verdict::Fine {
        let skip = avoid(&q.name);
        if !frozen_in(payload, &skip) {
            handle.send(&pkt, &addr);
            return;
        }
        let (handle, r, log) = (handle.clone(), r.clone(), log.clone());
        std::thread::spawn(move || {
            let Some(u) = packet::parse_udp(&pkt) else { return };
            match r.avoiding(&q.name, q.qtype, &skip) {
                Some(good) => {
                    handle.send(&packet::udp_like(&u, &with_id(&good, q.id)), &addr);
                    if r.should_note(&q.name, Instant::now()) {
                        let ips: Vec<String> = answers(&good).map(|a| a.addrs.iter().take(2).map(|ip| ip.to_string()).collect()).unwrap_or_default();
                        log.line(&format!("DNS: {} — сервер обрывал загрузку, взят другой адрес {}", q.name, ips.join(", ")));
                    }
                }
                None => {
                    handle.send(&pkt, &addr);
                    if r.should_note(&q.name, Instant::now()) {
                        log.line(&format!("DNS: {} — сервер обрывал загрузку, другого адреса шифрованный DNS не дал", q.name));
                    }
                }
            }
        });
        return;
    }
    if !wanted(&q.name) {
        handle.send(&pkt, &addr);
        return;
    }
    let (handle, r, log) = (handle.clone(), r.clone(), log.clone());
    std::thread::spawn(move || {
        let Some(u) = packet::parse_udp(&pkt) else { return };
        match r.fix(u.payload(), Instant::now()) {
            Some(Fix::Replaced(good, q, v)) => {
                handle.send(&packet::udp_like(&u, &good), &addr);
                if r.should_note(&q.name, Instant::now()) {
                    log.line(&format!("DNS: {} — {}, адрес взят у шифрованного DNS", q.name, v.text()));
                }
            }
            Some(Fix::Confirmed) | None => {
                handle.send(&pkt, &addr);
            }
            Some(Fix::Failed) => {
                handle.send(&pkt, &addr);
                if let Some(q) = question(u.payload()).filter(|q| r.should_note(&q.name, Instant::now())) {
                    log.line(&format!("DNS: {} — {}, шифрованный DNS не ответил", q.name, verdict(u.payload(), q.qtype).text()));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(name: &str, qtype: u16, rcode: u8, addrs: &[IpAddr], ttl: u32) -> Vec<u8> {
        let mut m = query(name, qtype);
        m[0..2].copy_from_slice(&0x1234u16.to_be_bytes());
        m[2] = 0x81;
        m[3] = 0x80 | rcode;
        m[6..8].copy_from_slice(&(addrs.len() as u16).to_be_bytes());
        for a in addrs {
            m.extend_from_slice(&[0xC0, 0x0C]);
            let (t, data): (u16, Vec<u8>) = match a {
                IpAddr::V4(v) => (TYPE_A, v.octets().to_vec()),
                IpAddr::V6(v) => (TYPE_AAAA, v.octets().to_vec()),
            };
            m.extend_from_slice(&t.to_be_bytes());
            m.extend_from_slice(&1u16.to_be_bytes());
            m.extend_from_slice(&ttl.to_be_bytes());
            m.extend_from_slice(&(data.len() as u16).to_be_bytes());
            m.extend_from_slice(&data);
        }
        m
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn reads_question_and_answers_with_pointers() {
        let m = response("www.YouTube.com", TYPE_A, 0, &[ip("142.250.74.110"), ip("142.250.74.142")], 120);
        assert_eq!(question(&m), Some(Question { id: 0x1234, name: "www.youtube.com".into(), qtype: TYPE_A }));
        let a = answers(&m).unwrap();
        assert_eq!((a.rcode, a.addrs.len(), a.min_ttl), (0, 2, Some(120)));
        assert_eq!(verdict(&m, TYPE_A), Verdict::Fine);
        assert_eq!(question(&query("x.com", TYPE_AAAA)).unwrap().qtype, TYPE_AAAA);
        assert!(answers(&query("x.com", TYPE_A)).is_none());
        let mut loop_ptr = response("a.com", TYPE_A, 0, &[ip("1.2.3.4")], 60);
        let at = loop_ptr.len() - 16;
        loop_ptr[at..at + 2].copy_from_slice(&[0xC0, at as u8]);
        assert!(answers(&loop_ptr).is_none());
    }

    #[test]
    fn spots_blocked_answers() {
        assert_eq!(verdict(&response("www.youtube.com", TYPE_A, 3, &[], 0), TYPE_A), Verdict::Missing);
        assert_eq!(verdict(&response("www.youtube.com", TYPE_A, 5, &[], 0), TYPE_A), Verdict::Refused);
        assert_eq!(verdict(&response("www.youtube.com", TYPE_A, 0, &[], 0), TYPE_A), Verdict::Missing);
        assert_eq!(verdict(&response("www.youtube.com", TYPE_AAAA, 0, &[], 0), TYPE_AAAA), Verdict::Fine);
        for stub in ["0.0.0.0", "127.0.0.1", "10.10.10.10", "192.168.1.1", "100.64.0.1", "::", "::1", "fd00::1"] {
            assert_eq!(verdict(&response("www.youtube.com", TYPE_A, 0, &[ip(stub)], 60), TYPE_A), Verdict::Bogus, "{stub}");
        }
        assert!(!bogus(&ip("8.8.8.8")) && !bogus(&ip("172.32.0.1")) && !bogus(&ip("2a00:1450::1")));
    }

    #[test]
    fn resolver_replaces_bad_answer_keeps_id_caches_and_pauses() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let r = Resolver::with(Box::new(move |server, body| {
            c.fetch_add(1, Ordering::Relaxed);
            let q = question(body)?;
            (server == "1.1.1.1").then(|| response(&q.name, q.qtype, 0, &[ip("142.250.74.110")], 300))
        }));
        let now = Instant::now();
        let bad = response("www.youtube.com", TYPE_A, 3, &[], 0);
        let Some(Fix::Replaced(good, q, v)) = r.fix(&bad, now) else { panic!("не заменён") };
        assert_eq!((q.name.as_str(), v, question(&good).unwrap().id), ("www.youtube.com", Verdict::Missing, 0x1234));
        assert_eq!(answers(&good).unwrap().addrs, vec![ip("142.250.74.110")]);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(matches!(r.fix(&bad, now + Duration::from_secs(10)), Some(Fix::Replaced(..))));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(r.fix(&response("ok.com", TYPE_A, 0, &[ip("1.2.3.4")], 60), now).is_none());

        let dead = Resolver::with(Box::new(|_, _| None));
        for i in 0..PAUSE_AFTER {
            assert!(matches!(dead.fix(&bad, now + Duration::from_secs(i as u64)), Some(Fix::Failed)));
        }
        assert!(dead.paused.lock().unwrap().is_some());
        assert_eq!(dead.failed.load(Ordering::Relaxed), PAUSE_AFTER as u64);
    }

    #[test]
    fn real_missing_site_does_not_pause_the_fixer() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let r = Resolver::with(Box::new(move |_, body| {
            c.fetch_add(1, Ordering::Relaxed);
            let q = question(body)?;
            Some(response(&q.name, q.qtype, 3, &[], 0))
        }));
        let now = Instant::now();
        for i in 0..PAUSE_AFTER * 2 {
            let name = format!("nothing{i}.discord.media");
            assert!(matches!(r.fix(&response(&name, TYPE_A, 3, &[], 0), now), Some(Fix::Confirmed)));
        }
        assert!(r.paused.lock().unwrap().is_none());
        assert_eq!(r.failed.load(Ordering::Relaxed), 0);
        let before = calls.load(Ordering::Relaxed);
        assert!(matches!(r.fix(&response("nothing0.discord.media", TYPE_A, 3, &[], 0), now + Duration::from_secs(30)), Some(Fix::Confirmed)));
        assert_eq!(calls.load(Ordering::Relaxed), before);
    }

    #[test]
    fn frozen_server_is_swapped_for_another_address() {
        let r = Resolver::with(Box::new(|server, body| {
            let q = question(body)?;
            let ip = if server == "8.8.8.8" { "18.239.255.86" } else { "18.64.211.103" };
            Some(response(&q.name, q.qtype, 0, &[ip.parse().unwrap()], 60))
        }));
        let bad: Vec<IpAddr> = vec!["18.239.255.86".parse().unwrap()];
        let isp = response("tr.rbxcdn.com", TYPE_A, 0, &[bad[0]], 60);
        assert!(frozen_in(&isp, &bad));
        assert!(!frozen_in(&isp, &[]));
        let good = r.avoiding("tr.rbxcdn.com", TYPE_A, &bad).unwrap();
        assert_eq!(answers(&good).unwrap().addrs, vec!["18.64.211.103".parse::<IpAddr>().unwrap()]);
        let all: Vec<IpAddr> = vec![bad[0], "18.64.211.103".parse().unwrap()];
        assert!(r.avoiding("tr.rbxcdn.com", TYPE_A, &all).is_none());
    }

    #[test]
    fn cname_chain_without_address_is_missing() {
        let mut m = response("tr.rbxcdn.com", TYPE_A, 0, &[], 0);
        m[6..8].copy_from_slice(&1u16.to_be_bytes());
        m.extend_from_slice(&[0xC0, 0x0C, 0, 5, 0, 1, 0, 0, 0, 60, 0, 2, 0xC0, 0x0C]);
        let a = answers(&m).unwrap();
        assert!(a.cname && a.addrs.is_empty());
        assert_eq!(verdict(&m, TYPE_A), Verdict::Missing);
        assert_eq!(verdict(&m, TYPE_AAAA), Verdict::Fine);
    }

    #[test]
    fn replaced_answer_fits_the_original_datagram() {
        let mut p = vec![0u8; 28];
        p[0] = 0x45;
        p[8] = 64;
        p[9] = 17;
        p[12..16].copy_from_slice(&[192, 168, 1, 1]);
        p[16..20].copy_from_slice(&[192, 168, 1, 10]);
        p[20..22].copy_from_slice(&53u16.to_be_bytes());
        p[22..24].copy_from_slice(&50000u16.to_be_bytes());
        let bad = response("www.youtube.com", TYPE_A, 3, &[], 0);
        p.extend_from_slice(&bad);
        let total = p.len() as u16;
        p[2..4].copy_from_slice(&total.to_be_bytes());
        p[24..26].copy_from_slice(&((8 + bad.len()) as u16).to_be_bytes());
        packet::fix_checksums(&mut p);
        let u = packet::parse_udp(&p).unwrap();
        let good = with_id(&response("www.youtube.com", TYPE_A, 0, &[ip("142.250.74.110")], 300), 0x1234);
        let out = packet::udp_like(&u, &good);
        let o = packet::parse_udp(&out).unwrap();
        assert_eq!((o.sport, o.dport, o.payload()), (53, 50000, &good[..]));
        assert!(packet::checksums_ok(&out));
    }

    #[test]
    #[ignore]
    fn encrypted_dns_answers_live() {
        let r = Resolver::new();
        let now = Instant::now();
        let resp = r.resolve("www.youtube.com", TYPE_A, now).expect("шифрованный DNS не ответил");
        assert_eq!(r.resolve("nothing-here-at-all.discord.media", TYPE_A, now), Err(true));
        assert!(!answers(&resp).unwrap().addrs.is_empty());
        for server in SERVERS {
            let ok = http::post(server, &query("discord.com", TYPE_A), TIMEOUT).is_some_and(|r| matches(&r, "discord.com", TYPE_A));
            println!("{server}: {ok}");
        }
    }
}
