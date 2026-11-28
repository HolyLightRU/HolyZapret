use crate::flows::{Engine, Outcome};
use crate::packet::{self, Tcp};
use crate::run::Log;
use crate::windivert::{self, Address, Handle, Lib};
use crate::tls;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 40000;
pub const RULE: &str = "HolyZapret RayOfLight WARP";
pub const WHY: &str = "через WARP";
const IP_KEEP: Duration = Duration::from_secs(30 * 60);
const CONN_KEEP: Duration = Duration::from_secs(15 * 60);
const WANT_HOLD: Duration = Duration::from_secs(5 * 60);
const MAX_IPS: usize = 256;
const HELLO_WAIT: Duration = Duration::from_secs(3);
const SOCKS_WAIT: Duration = Duration::from_secs(5);
const ANSWER_WAIT: Duration = Duration::from_secs(8);

pub struct Shared {
    pub proxy: u16,
    listen: AtomicU16,
    ips: Mutex<HashMap<IpAddr, Instant>>,
    version: AtomicU64,
    conns: Mutex<HashMap<u16, (IpAddr, u16, Instant)>>,
    ready: AtomicBool,
    wanted: Mutex<Option<Instant>>,
    pub active: AtomicUsize,
    pub done: AtomicU64,
    pub answered: AtomicU64,
    force: Vec<String>,
}

impl Shared {
    pub fn new(proxy: u16, force: Vec<String>) -> Arc<Shared> {
        Arc::new(Shared {
            proxy,
            listen: AtomicU16::new(0),
            ips: Mutex::new(HashMap::new()),
            version: AtomicU64::new(0),
            conns: Mutex::new(HashMap::new()),
            ready: AtomicBool::new(false),
            wanted: Mutex::new(None),
            active: AtomicUsize::new(0),
            done: AtomicU64::new(0),
            answered: AtomicU64::new(0),
            force: force.into_iter().map(|h| h.to_ascii_lowercase()).collect(),
        })
    }

    pub fn forced(&self, host: &str) -> bool {
        self.force.iter().any(|f| host == f || host.ends_with(&format!(".{f}")))
    }

    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed) && self.listen.load(Ordering::Relaxed) != 0
    }

    pub fn set_listen(&self, port: u16) {
        self.listen.store(port, Ordering::Relaxed);
    }

    pub fn set_ready(&self, v: bool) {
        self.ready.store(v, Ordering::Relaxed);
    }

    pub fn want(&self, now: Instant) {
        if let Ok(mut w) = self.wanted.lock() {
            *w = Some(now);
        }
    }

    pub fn wanted(&self, now: Instant) -> bool {
        self.wanted.lock().ok().and_then(|w| *w).is_some_and(|t| now.saturating_duration_since(t) < WANT_HOLD)
    }

    pub fn add_ip(&self, ip: IpAddr, now: Instant) {
        if let Ok(mut m) = self.ips.lock() {
            if m.len() >= MAX_IPS && !m.contains_key(&ip) {
                return;
            }
            if m.insert(ip, now).is_none() {
                self.version.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    pub fn has_ip(&self, ip: &IpAddr) -> bool {
        self.ips.lock().is_ok_and(|m| m.contains_key(ip))
    }

    pub fn ip_count(&self) -> usize {
        self.ips.lock().map(|m| m.len()).unwrap_or(0)
    }

    fn snapshot(&self) -> (u64, Vec<IpAddr>) {
        let v = self.version.load(Ordering::SeqCst);
        let mut ips: Vec<IpAddr> = self.ips.lock().map(|m| m.keys().copied().collect()).unwrap_or_default();
        ips.sort();
        (v, ips)
    }

    pub fn prune(&self, now: Instant) {
        if let Ok(mut m) = self.ips.lock() {
            let before = m.len();
            m.retain(|_, t| now.saturating_duration_since(*t) < IP_KEEP);
            if m.len() != before {
                self.version.fetch_add(1, Ordering::SeqCst);
            }
        }
        if let Ok(mut c) = self.conns.lock() {
            c.retain(|_, v| now.saturating_duration_since(v.2) < CONN_KEEP);
        }
    }

    fn touch(&self, ip: IpAddr, now: Instant) {
        if let Ok(mut m) = self.ips.lock() {
            if let Some(t) = m.get_mut(&ip) {
                *t = now;
            }
        }
    }

    fn conn(&self, local_port: u16) -> Option<(IpAddr, u16)> {
        self.conns.lock().ok()?.get(&local_port).map(|v| (v.0, v.1))
    }

    pub fn json(&self, now: Instant) -> String {
        format!(
            "\"warp\":{{\"ready\":{},\"wanted\":{},\"active\":{},\"ips\":{},\"done\":{},\"answered\":{}}},",
            self.ready(),
            self.wanted(now),
            self.active.load(Ordering::Relaxed),
            self.ip_count(),
            self.done.load(Ordering::Relaxed),
            self.answered.load(Ordering::Relaxed)
        )
    }
}

fn v4_tcp(p: &[u8]) -> Option<usize> {
    let ihl = (*p.first()? & 0x0f) as usize * 4;
    (p.len() >= ihl + 20 && p[0] >> 4 == 4 && p[9] == 6).then_some(ihl)
}

fn swap_addrs(p: &mut [u8]) {
    let src: [u8; 4] = p[12..16].try_into().unwrap();
    let dst: [u8; 4] = p[16..20].try_into().unwrap();
    p[12..16].copy_from_slice(&dst);
    p[16..20].copy_from_slice(&src);
}

pub fn to_listener(p: &mut [u8], listen: u16) -> bool {
    let Some(ihl) = v4_tcp(p) else { return false };
    swap_addrs(p);
    p[ihl + 2..ihl + 4].copy_from_slice(&listen.to_be_bytes());
    packet::fix_checksums(p);
    true
}

pub fn to_app(p: &mut [u8], remote_port: u16) -> bool {
    let Some(ihl) = v4_tcp(p) else { return false };
    swap_addrs(p);
    p[ihl..ihl + 2].copy_from_slice(&remote_port.to_be_bytes());
    packet::fix_checksums(p);
    true
}

pub fn reset_for(t: &Tcp) -> Option<Vec<u8>> {
    let (IpAddr::V4(src), IpAddr::V4(dst)) = (t.src, t.dst) else { return None };
    let mut p = vec![0u8; 40];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&40u16.to_be_bytes());
    p[6] = 0x40;
    p[8] = 64;
    p[9] = 6;
    p[12..16].copy_from_slice(&dst.octets());
    p[16..20].copy_from_slice(&src.octets());
    p[20..22].copy_from_slice(&t.dport.to_be_bytes());
    p[22..24].copy_from_slice(&t.sport.to_be_bytes());
    p[24..28].copy_from_slice(&t.ack.to_be_bytes());
    p[28..32].copy_from_slice(&t.seq.wrapping_add(t.payload().len() as u32).to_be_bytes());
    p[32] = 5 << 4;
    p[33] = 0x14;
    packet::fix_checksums(&mut p);
    Some(p)
}

pub fn inbound(addr: &Address) -> Address {
    let mut a = *addr;
    a.bits &= !(1 << 17);
    a
}

pub fn redirect_filter(listen: u16, ips: &[IpAddr]) -> Option<String> {
    let v4: Vec<String> = ips.iter().filter_map(|ip| if let IpAddr::V4(v) = ip { Some(format!("ip.DstAddr == {v}")) } else { None }).collect();
    if v4.is_empty() || listen == 0 {
        return None;
    }
    Some(format!(
        "outbound and !loopback and ip and tcp and (tcp.SrcPort == {listen} or ((tcp.DstPort == 443 or tcp.DstPort == 80) and ({})))",
        v4.join(" or ")
    ))
}

fn redirect_new(shared: &Shared, dst: &IpAddr, now: Instant) -> bool {
    if !shared.has_ip(dst) {
        return false;
    }
    if shared.ready() {
        return true;
    }
    shared.want(now);
    false
}

fn handle_packet(shared: &Shared, h: &Handle, buf: &mut [u8], addr: &Address) {
    let listen = shared.listen.load(Ordering::Relaxed);
    let Some(ihl) = v4_tcp(buf) else {
        let _ = h.send(buf, addr);
        return;
    };
    let sport = u16::from_be_bytes([buf[ihl], buf[ihl + 1]]);
    let dport = u16::from_be_bytes([buf[ihl + 2], buf[ihl + 3]]);
    let flags = buf[ihl + 13];
    if sport == listen {
        if let Some((_, rport)) = shared.conn(dport) {
            if to_app(buf, rport) {
                let _ = h.send_built(buf, &inbound(addr));
            }
        }
        return;
    }
    let dst = IpAddr::V4(Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]));
    let syn = flags & 0x02 != 0 && flags & 0x10 == 0;
    let now = Instant::now();
    if syn && redirect_new(shared, &dst, now) {
        if let Ok(mut c) = shared.conns.lock() {
            c.insert(sport, (dst, dport, now));
        }
    }
    match shared.conn(sport) {
        Some((ip, port)) if ip == dst && port == dport => {
            if to_listener(buf, listen) {
                let _ = h.send_built(buf, &inbound(addr));
            }
        }
        _ => {
            let _ = h.send(buf, addr);
        }
    }
}

fn socks_connect(proxy: u16, host: Option<&str>, ip: IpAddr, port: u16) -> std::io::Result<TcpStream> {
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, proxy)), SOCKS_WAIT)?;
    s.set_read_timeout(Some(SOCKS_WAIT))?;
    s.set_nodelay(true)?;
    s.write_all(&[5, 1, 0])?;
    let mut r = [0u8; 2];
    s.read_exact(&mut r)?;
    if r != [5, 0] {
        return Err(std::io::Error::other("прокси WARP не принял приветствие"));
    }
    let mut req = vec![5, 1, 0];
    match (host, ip) {
        (Some(h), _) if !h.is_empty() && h.len() < 256 => {
            req.push(3);
            req.push(h.len() as u8);
            req.extend_from_slice(h.as_bytes());
        }
        (_, IpAddr::V4(v)) => {
            req.push(1);
            req.extend_from_slice(&v.octets());
        }
        (_, IpAddr::V6(v)) => {
            req.push(4);
            req.extend_from_slice(&v.octets());
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[1] != 0 {
        return Err(std::io::Error::other(format!("прокси WARP отказал, код {}", head[1])));
    }
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n)?;
            n[0] as usize + 2
        }
        _ => return Err(std::io::Error::other("прокси WARP ответил непонятно")),
    };
    let mut skip = vec![0u8; rest];
    s.read_exact(&mut skip)?;
    Ok(s)
}

fn host_of(first: &[u8], port: u16) -> Option<String> {
    if port == 443 {
        return tls::sni(first);
    }
    crate::profiles::http_host(first)
}

fn pump(mut from: TcpStream, mut to: TcpStream) {
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Write);
}

fn serve<A: Clone + Send + 'static>(shared: Arc<Shared>, engine: Arc<Mutex<Engine<A>>>, mut app: TcpStream, ip: IpAddr, local_port: u16, port: u16, log: Arc<Log>) {
    shared.active.fetch_add(1, Ordering::Relaxed);
    let _ = app.set_nodelay(true);
    let _ = app.set_read_timeout(Some(HELLO_WAIT));
    let mut first = vec![0u8; 16 * 1024];
    let n = app.read(&mut first).unwrap_or(0);
    first.truncate(n);
    let host = host_of(&first, port);
    let result = socks_connect(shared.proxy, host.as_deref(), ip, port).and_then(|mut s| {
        s.write_all(&first)?;
        s.set_read_timeout(Some(ANSWER_WAIT))?;
        let mut answer = vec![0u8; 16 * 1024];
        let n = s.read(&mut answer)?;
        answer.truncate(n);
        Ok((s, answer))
    });
    let ok = matches!(&result, Ok((_, a)) if !a.is_empty());
    shared.done.fetch_add(1, Ordering::Relaxed);
    if ok {
        shared.answered.fetch_add(1, Ordering::Relaxed);
        shared.touch(ip, Instant::now());
    }
    if let Err(e) = &result {
        log.line(&format!("WARP: {} — {e}", host.as_deref().unwrap_or("без имени")));
    }
    if let Ok(mut e) = engine.lock() {
        e.warp_settle(host.clone(), local_port, ip, port, if ok { Outcome::Answered } else { Outcome::Silent }, Instant::now());
    }
    if let Ok((s, answer)) = result {
        if !answer.is_empty() && app.write_all(&answer).is_ok() {
            let _ = app.set_read_timeout(None);
            let _ = s.set_read_timeout(None);
            if let (Ok(a2), Ok(s2)) = (app.try_clone(), s.try_clone()) {
                let up = std::thread::spawn(move || pump(a2, s2));
                pump(s, app);
                let _ = up.join();
            }
        }
    }
    shared.active.fetch_sub(1, Ordering::Relaxed);
}

fn netsh(args: &[&str]) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("netsh").args(args).creation_flags(0x0800_0000).output();
}

fn firewall(on: bool) {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    netsh(&["advfirewall", "firewall", "delete", "rule", &format!("name={RULE}")]);
    if on && !exe.is_empty() {
        netsh(&["advfirewall", "firewall", "add", "rule", &format!("name={RULE}"), "dir=in", "action=allow", &format!("program={exe}"), "protocol=TCP", "profile=any"]);
    }
}

pub fn start<A: Clone + Send + 'static>(lib: Lib, priority: i16, shared: Arc<Shared>, engine: Arc<Mutex<Engine<A>>>, ended: Arc<AtomicBool>, log: Arc<Log>) -> Result<(), String> {
    let listener = TcpListener::bind(("0.0.0.0", 0)).map_err(|e| format!("WARP: не открылся порт для перенаправления: {e}"))?;
    let listen = listener.local_addr().map_err(|e| e.to_string())?.port();
    shared.listen.store(listen, Ordering::Relaxed);
    firewall(true);
    log.line(&format!("WARP: последний шаг самоподстройки, прокси 127.0.0.1:{}, перенаправление на порт {listen}", shared.proxy));
    {
        let (shared, ended) = (shared.clone(), ended.clone());
        std::thread::spawn(move || {
            let mut was = None;
            while !ended.load(Ordering::SeqCst) {
                let up = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, shared.proxy)), Duration::from_millis(500)).is_ok();
                shared.set_ready(up);
                if was != Some(up) {
                    was = Some(up);
                }
                shared.prune(Instant::now());
                for _ in 0..25 {
                    if ended.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            firewall(false);
        });
    }
    {
        let (shared, engine, log) = (shared.clone(), engine.clone(), log.clone());
        std::thread::spawn(move || {
            for app in listener.incoming() {
                let Ok(app) = app else { continue };
                let Ok(peer) = app.peer_addr() else { continue };
                let Some((ip, port)) = shared.conn(peer.port()).filter(|(ip, _)| *ip == peer.ip()) else {
                    drop(app);
                    continue;
                };
                let (shared, engine, log) = (shared.clone(), engine.clone(), log.clone());
                let local_port = peer.port();
                std::thread::spawn(move || serve(shared, engine, app, ip, local_port, port, log));
            }
        });
    }
    std::thread::spawn(move || {
        let mut current: Option<(u64, Option<Arc<Handle>>)> = None;
        while !ended.load(Ordering::SeqCst) {
            let (version, ips) = shared.snapshot();
            if current.as_ref().map(|c| c.0) != Some(version) {
                let next = redirect_filter(listen, &ips).and_then(|f| match lib.open(&f, windivert::LAYER_NETWORK, priority, 0) {
                    Ok(h) => Some(Arc::new(h)),
                    Err(e) => {
                        log.line(&format!("WARP: перехват не включился: {e}"));
                        None
                    }
                });
                if let Some(h) = &next {
                    let (h, shared) = (h.clone(), shared.clone());
                    std::thread::spawn(move || {
                        let mut buf = vec![0u8; windivert::MAX_PACKET];
                        while let Some((len, addr)) = h.recv(&mut buf) {
                            handle_packet(&shared, &h, &mut buf[..len], &addr);
                        }
                    });
                }
                if let Some((_, Some(old))) = current.take() {
                    old.shutter().shutdown();
                }
                current = Some((version, next));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        if let Some((_, Some(h))) = current {
            h.shutter().shutdown();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syn(src: [u8; 4], sport: u16, dst: [u8; 4], dport: u16) -> Vec<u8> {
        let mut p = vec![0u8; 40];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&40u16.to_be_bytes());
        p[8] = 128;
        p[9] = 6;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p[20..22].copy_from_slice(&sport.to_be_bytes());
        p[22..24].copy_from_slice(&dport.to_be_bytes());
        p[24..28].copy_from_slice(&1000u32.to_be_bytes());
        p[32] = 5 << 4;
        p[33] = 0x02;
        packet::fix_checksums(&mut p);
        p
    }

    #[test]
    fn rewrites_round_trip_through_listener() {
        let mut out = syn([192, 168, 1, 10], 50000, [104, 16, 1, 1], 443);
        assert!(to_listener(&mut out, 40400));
        let t = packet::parse(&out).unwrap();
        assert_eq!((t.src, t.sport, t.dst, t.dport), ("104.16.1.1".parse().unwrap(), 50000, "192.168.1.10".parse().unwrap(), 40400));
        let mut reply = syn([192, 168, 1, 10], 40400, [104, 16, 1, 1], 50000);
        assert!(to_app(&mut reply, 443));
        let r = packet::parse(&reply).unwrap();
        assert_eq!((r.src, r.sport, r.dst, r.dport), ("104.16.1.1".parse().unwrap(), 443, "192.168.1.10".parse().unwrap(), 50000));
        let mut copy = reply.clone();
        packet::fix_checksums(&mut copy);
        assert_eq!(copy, reply);
    }

    #[test]
    fn reset_goes_back_to_the_app() {
        let p = syn([192, 168, 1, 10], 50000, [104, 16, 1, 1], 443);
        let t = packet::parse(&p).unwrap();
        let r = reset_for(&t).unwrap();
        let rt = packet::parse(&r).unwrap();
        assert_eq!((rt.src, rt.sport, rt.dst, rt.dport, rt.flags), (t.dst, 443, t.src, 50000, 0x14));
        assert_eq!(rt.seq, t.ack);
    }

    #[test]
    fn filter_lists_only_ipv4_targets() {
        assert_eq!(redirect_filter(40400, &[]), None);
        let f = redirect_filter(40400, &["104.16.1.1".parse().unwrap(), "2606:4700::1".parse().unwrap()]).unwrap();
        assert!(f.contains("tcp.SrcPort == 40400") && f.contains("ip.DstAddr == 104.16.1.1") && !f.contains("2606"));
        assert_eq!(f.matches('(').count(), f.matches(')').count());
        let s = Shared::new(DEFAULT_PORT, vec!["Example.org".into()]);
        assert!(s.forced("www.example.org") && s.forced("example.org") && !s.forced("notexample.org"));
        let now = Instant::now();
        s.add_ip("104.16.1.1".parse().unwrap(), now);
        assert!(s.has_ip(&"104.16.1.1".parse().unwrap()));
        assert!(!s.wanted(now));
        s.want(now);
        assert!(s.wanted(now) && !s.wanted(now + WANT_HOLD));
        s.prune(now + IP_KEEP);
        assert_eq!(s.ip_count(), 0);
    }

    #[test]
    fn new_connections_go_direct_and_ask_for_warp_while_proxy_is_down() {
        let s = Shared::new(40000, vec![]);
        let ip: IpAddr = "104.16.1.1".parse().unwrap();
        let other: IpAddr = "104.16.1.2".parse().unwrap();
        let now = Instant::now();
        s.add_ip(ip, now);
        s.set_listen(40400);
        assert!(!redirect_new(&s, &other, now));
        assert!(!s.wanted(now));
        s.set_ready(false);
        assert!(!redirect_new(&s, &ip, now));
        assert!(s.wanted(now));
        s.set_ready(true);
        assert!(redirect_new(&s, &ip, now));
    }
}
