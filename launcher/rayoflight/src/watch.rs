use crate::run::Log;
use crate::{packet, quic, sys, tls, windivert};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const FORWARD_FILTER: &str = "true";
pub const REDIRECT_FILTER: &str = "outbound and ((icmp.Type == 5) or (icmpv6.Type == 137))";
const SUMMARY_EVERY: Duration = Duration::from_secs(10);
const MAX_NAMES: usize = 2000;

#[derive(Default)]
pub struct Seen {
    pub packets: HashMap<IpAddr, (u64, u64)>,
    pub names: HashSet<(IpAddr, String, &'static str)>,
    pub redirects: HashMap<IpAddr, u64>,
}

fn source(raw: &[u8]) -> Option<IpAddr> {
    match raw.first()? >> 4 {
        4 if raw.len() >= 20 => Some(IpAddr::V4(Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15]))),
        6 if raw.len() >= 40 => Some(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&raw[8..24]).ok()?))),
        _ => None,
    }
}

fn destination(raw: &[u8]) -> Option<IpAddr> {
    match raw.first()? >> 4 {
        4 if raw.len() >= 20 => Some(IpAddr::V4(Ipv4Addr::new(raw[16], raw[17], raw[18], raw[19]))),
        6 if raw.len() >= 40 => Some(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&raw[24..40]).ok()?))),
        _ => None,
    }
}

impl Seen {
    pub fn forwarded(&mut self, raw: &[u8]) -> Option<String> {
        let src = source(raw)?;
        let e = self.packets.entry(src).or_default();
        e.0 += 1;
        e.1 += raw.len() as u64;
        let (name, kind, dst) = if packet::is_udp(raw) {
            let u = packet::parse_udp(raw)?;
            if u.dport != 443 || !quic::is_initial(u.payload()) {
                return None;
            }
            let name = quic::hello_of(u.payload()).and_then(|(_, h)| quic::sni(&h)).unwrap_or_else(|| "без имени".into());
            (name, "QUIC", u.dst)
        } else {
            let t = packet::parse(raw)?;
            let d = t.payload();
            if t.dport != 443 || !tls::is_client_hello(d) {
                return None;
            }
            (tls::sni(d).unwrap_or_else(|| "без имени".into()), "TLS", t.dst)
        };
        if self.names.len() >= MAX_NAMES || !self.names.insert((src, name.clone(), kind)) {
            return None;
        }
        Some(format!("вижу пересылаемое соединение: {src} → {name} ({kind}, адрес {dst})"))
    }

    pub fn redirect(&mut self, raw: &[u8]) -> Option<String> {
        let to = destination(raw)?;
        let n = self.redirects.entry(to).or_default();
        *n += 1;
        (*n == 1).then(|| format!("Windows отправила устройству {to} сообщение «иди напрямую к роутеру» (ICMP redirect)"))
    }

    pub fn summary(&self) -> String {
        if self.packets.is_empty() {
            return "пересылаемых пакетов пока нет: проверьте, что на устройстве шлюзом указан этот компьютер и что пересылка включена".into();
        }
        let mut v: Vec<(&IpAddr, &(u64, u64))> = self.packets.iter().collect();
        v.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        let parts: Vec<String> = v.iter().take(8).map(|(ip, (n, b))| format!("{ip}: {n} пакетов, {} КБ", b / 1024)).collect();
        let named = self.names.len();
        let redirects: u64 = self.redirects.values().sum();
        format!("пересылается: {}; соединений с именем сайта {named}; перенаправлений от Windows {redirects}", parts.join(", "))
    }
}

pub fn run(lib: &windivert::Lib, priority: i16, log: &Arc<Log>) -> Result<(), String> {
    let fwd = Arc::new(lib.open(FORWARD_FILTER, windivert::LAYER_NETWORK_FORWARD, priority, windivert::FLAG_SNIFF | windivert::FLAG_RECV_ONLY)?);
    let icmp = lib.open(REDIRECT_FILTER, windivert::LAYER_NETWORK, priority, windivert::FLAG_SNIFF | windivert::FLAG_RECV_ONLY).map(Arc::new);
    if let Err(e) = &icmp {
        log.line(&format!("перенаправления Windows не отслеживаются: {e}"));
    }
    let icmp = icmp.ok();
    let shutters = Mutex::new(vec![fwd.shutter()]);
    if let Some(i) = &icmp {
        shutters.lock().unwrap().push(i.shutter());
    }
    sys::on_stop(move || {
        for s in shutters.lock().unwrap().iter() {
            s.shutdown();
        }
    });
    log.line("режим наблюдения: смотрю пересылаемый трафик, ничего не меняю. Остановить: Ctrl+C");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let icmp_thread = icmp.map(|i| {
        let (seen, log) = (seen.clone(), log.clone());
        std::thread::spawn(move || {
            let mut buf = vec![0u8; windivert::MAX_PACKET];
            while let Some((n, _)) = i.recv(&mut buf) {
                if let Some(l) = seen.lock().unwrap().redirect(&buf[..n]) {
                    log.line(&l);
                }
            }
        })
    });
    let mut buf = vec![0u8; windivert::MAX_PACKET];
    let mut last = Instant::now();
    while let Some((n, _)) = fwd.recv(&mut buf) {
        let line = seen.lock().unwrap().forwarded(&buf[..n]);
        if let Some(l) = line {
            log.line(&l);
        }
        if last.elapsed() >= SUMMARY_EVERY {
            last = Instant::now();
            log.line(&seen.lock().unwrap().summary());
        }
    }
    if let Some(t) = icmp_thread {
        let _ = t.join();
    }
    log.line(&seen.lock().unwrap().summary());
    log.line("наблюдение остановлено");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_forwarded_hello_once_and_counts_packets() {
        let hello = tls::with_sni(crate::desync::FAKE_DEFAULT, "www.youtube.com").unwrap();
        let pkt = packet::tests::sample_v4(&hello, true);
        let mut s = Seen::default();
        let first = s.forwarded(&pkt).unwrap();
        assert!(first.contains("www.youtube.com (TLS"), "{first}");
        assert!(s.forwarded(&pkt).is_none());
        let src = source(&pkt).unwrap();
        assert_eq!(s.packets[&src].0, 2);
        assert!(s.summary().contains("2 пакетов"));
    }

    #[test]
    fn quic_initial_named() {
        let pkt = packet::tests::sample_udp(&quic::tests::google());
        let mut s = Seen::default();
        let l = s.forwarded(&pkt).unwrap();
        assert!(l.contains("QUIC"), "{l}");
        assert!(l.contains("google"), "{l}");
    }

    #[test]
    fn redirect_logged_once_per_device() {
        let mut p = vec![0x45, 0, 0, 56, 0, 0, 0, 0, 64, 1, 0, 0, 192, 168, 1, 10, 192, 168, 1, 50, 5, 1];
        p.resize(56, 0);
        let mut s = Seen::default();
        assert!(s.redirect(&p).unwrap().contains("192.168.1.50"));
        assert!(s.redirect(&p).is_none());
        assert!(s.summary().contains("пересылаемых пакетов пока нет"));
    }
}
