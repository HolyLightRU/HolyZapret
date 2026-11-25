use super::coverage::Verdict;
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Flow {
    pub udp: bool,
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub pid: u32,
    pub flow: Flow,
}

use rayoflight::windivert::{self, Address};

const EVENT_ESTABLISHED: u32 = 1;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn decode_addr(words: [u32; 4], v6: bool) -> IpAddr {
    if !v6 || (words[1] == 0xFFFF && words[2] == 0 && words[3] == 0) {
        return IpAddr::V4(Ipv4Addr::from(words[0]));
    }
    let n = (words[3] as u128) << 96 | (words[2] as u128) << 64 | (words[1] as u128) << 32 | words[0] as u128;
    let v6 = Ipv6Addr::from(n);
    match v6.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(v6),
    }
}

fn decode(a: &Address) -> Option<Seen> {
    if a.event() != EVENT_ESTABLISHED || a.loopback() {
        return None;
    }
    let d = &a.data;
    let pid = u32_at(d, 16);
    let remote = [u32_at(d, 36), u32_at(d, 40), u32_at(d, 44), u32_at(d, 48)];
    let port = u16::from_le_bytes([d[54], d[55]]);
    let udp = match d[56] {
        6 => false,
        17 => true,
        _ => return None,
    };
    let ip = decode_addr(remote, a.ipv6());
    Some(Seen { pid, flow: Flow { udp, ip, port } })
}

pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_multicast() || v.is_broadcast() || v.is_unspecified() || v.octets()[0] == 100 && (v.octets()[1] & 0xC0) == 64),
        IpAddr::V6(v) => !(v.is_loopback() || v.is_multicast() || v.is_unspecified() || (v.segments()[0] & 0xFE00) == 0xFC00 || (v.segments()[0] & 0xFFC0) == 0xFE80),
    }
}

pub fn watch(bin: &Path, stop: Arc<AtomicBool>, mut sink: impl FnMut(Seen)) -> anyhow::Result<()> {
    let lib = windivert::Lib::load(bin).map_err(anyhow::Error::msg)?;
    let h = lib.open("true", windivert::LAYER_FLOW, 0, windivert::FLAG_SNIFF | windivert::FLAG_RECV_ONLY).map_err(anyhow::Error::msg)?;
    let shutter = h.shutter();
    let done = Arc::new(AtomicBool::new(false));
    let waker = {
        let (stop, done) = (stop.clone(), done.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) && !done.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            shutter.shutdown();
        })
    };
    while !stop.load(Ordering::Relaxed) {
        let Some(a) = h.recv_event() else { break };
        if let Some(s) = decode(&a) {
            sink(s);
        }
    }
    done.store(true, Ordering::Relaxed);
    let _ = waker.join();
    Ok(())
}

pub fn tcp_now() -> Vec<Seen> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_CONNECTIONS};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    let mut out = Vec::new();
    for (family, row) in [(AF_INET, 24usize), (AF_INET6, 56usize)] {
        let mut size = 0u32;
        unsafe {
            GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, family as u32, TCP_TABLE_OWNER_PID_CONNECTIONS, 0);
            if size == 0 {
                continue;
            }
            let mut buf = vec![0u8; size as usize + 4096];
            size = buf.len() as u32;
            if GetExtendedTcpTable(buf.as_mut_ptr() as _, &mut size, 0, family as u32, TCP_TABLE_OWNER_PID_CONNECTIONS, 0) != 0 {
                continue;
            }
            let n = u32_at(&buf, 0) as usize;
            for i in 0..n {
                let r = 4 + i * row;
                if r + row > buf.len() {
                    break;
                }
                let b = &buf[r..r + row];
                let (state, ip, port, pid) = if family == AF_INET {
                    (u32_at(b, 0), IpAddr::V4(Ipv4Addr::new(b[12], b[13], b[14], b[15])), u16::from_be_bytes([b[16], b[17]]), u32_at(b, 20))
                } else {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[24..40]);
                    let v6 = Ipv6Addr::from(o);
                    let ip = v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6));
                    (u32_at(b, 48), ip, u16::from_be_bytes([b[44], b[45]]), u32_at(b, 52))
                };
                if state == 5 && is_public(&ip) {
                    out.push(Seen { pid, flow: Flow { udp: false, ip, port } });
                }
            }
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct App {
    pub name: String,
    pub flows: BTreeMap<Flow, u32>,
}

pub fn group(seen: &[Seen], names: &HashMap<u32, String>, skip: &[&str]) -> Vec<App> {
    let mut by: HashMap<String, App> = HashMap::new();
    for s in seen {
        if !is_public(&s.flow.ip) {
            continue;
        }
        let name = names.get(&s.pid).cloned().unwrap_or_else(|| if s.pid == 4 { "System".into() } else { format!("Процесс {}", s.pid) });
        if skip.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let a = by.entry(name.to_lowercase()).or_insert_with(|| App { name: name.clone(), ..Default::default() });
        *a.flows.entry(s.flow).or_default() += 1;
    }
    let mut v: Vec<App> = by.into_values().collect();
    v.sort_by(|a, b| b.flows.len().cmp(&a.flows.len()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Judged {
    pub flow: Flow,
    pub now: Verdict,
    pub with_games: Verdict,
}

pub fn ports(flows: &[Flow]) -> String {
    let mut out = Vec::new();
    for udp in [false, true] {
        let mut ps: Vec<u16> = flows.iter().filter(|f| f.udp == udp).map(|f| f.port).collect();
        ps.sort();
        ps.dedup();
        if ps.is_empty() {
            continue;
        }
        let mut parts = Vec::new();
        let mut i = 0;
        while i < ps.len() {
            let mut j = i;
            while j + 1 < ps.len() && ps[j + 1] == ps[j] + 1 {
                j += 1;
            }
            parts.push(if j > i { format!("{}-{}", ps[i], ps[j]) } else { ps[i].to_string() });
            i = j + 1;
        }
        out.push(format!("{} {}", if udp { "UDP" } else { "TCP" }, parts.join(",")));
    }
    out.join(" · ")
}

pub fn addresses(judged: &[Judged], pick: impl Fn(&Judged) -> bool) -> Vec<String> {
    let mut v: Vec<String> = judged
        .iter()
        .filter(|j| pick(j))
        .map(|j| match j.flow.ip {
            IpAddr::V4(a) => format!("{a}/32"),
            IpAddr::V6(a) => format!("{a}/128"),
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_windivert_flow() {
        let mut a = Address { bits: 2 | (1 << 8), ..Default::default() };
        a.data[16..20].copy_from_slice(&1234u32.to_le_bytes());
        a.data[36..40].copy_from_slice(&u32::from(Ipv4Addr::new(5, 6, 7, 8)).to_le_bytes());
        a.data[40..44].copy_from_slice(&0xFFFFu32.to_le_bytes());
        a.data[54..56].copy_from_slice(&27015u16.to_le_bytes());
        a.data[56] = 17;
        assert_eq!(decode(&a), Some(Seen { pid: 1234, flow: Flow { udp: true, ip: "5.6.7.8".parse().unwrap(), port: 27015 } }));
        a.bits |= 1 << 18;
        assert_eq!(decode(&a), None);
        a.bits = 2 | (2 << 8);
        assert_eq!(decode(&a), None);
        let v6: Ipv6Addr = "2606:4700::1111".parse().unwrap();
        let n = u128::from(v6);
        let w = [n as u32, (n >> 32) as u32, (n >> 64) as u32, (n >> 96) as u32];
        assert_eq!(decode_addr(w, true), IpAddr::V6(v6));
    }

    #[test]
    fn groups_and_summarises() {
        let f = |udp, ip: &str, port| Flow { udp, ip: ip.parse().unwrap(), port };
        let seen = vec![
            Seen { pid: 10, flow: f(true, "5.6.7.8", 27015) },
            Seen { pid: 10, flow: f(true, "5.6.7.8", 27015) },
            Seen { pid: 11, flow: f(true, "5.6.7.9", 27016) },
            Seen { pid: 11, flow: f(false, "5.6.7.9", 443) },
            Seen { pid: 10, flow: f(true, "192.168.1.1", 53) },
            Seen { pid: 12, flow: f(false, "1.1.1.1", 443) },
        ];
        let names = HashMap::from([(10, "cs2.exe".to_string()), (11, "CS2.exe".to_string()), (12, "HolyZapret.exe".to_string())]);
        let apps = group(&seen, &names, &["holyzapret.exe"]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].flows.len(), 3);
        assert_eq!(apps[0].flows[&f(true, "5.6.7.8", 27015)], 2);
        let flows: Vec<Flow> = apps[0].flows.keys().copied().collect();
        assert_eq!(ports(&flows), "TCP 443 · UDP 27015-27016");
        assert!(!is_public(&"100.64.0.1".parse().unwrap()) && is_public(&"100.128.0.1".parse().unwrap()));
        assert!(!is_public(&"fe80::1".parse().unwrap()) && is_public(&"2a00::1".parse().unwrap()));
    }
}
