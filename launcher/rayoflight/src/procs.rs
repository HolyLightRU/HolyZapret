use crate::windivert::Address;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

pub const EVENT_CONNECT: u32 = 4;
pub const EVENT_CLOSE: u32 = 7;
const REFRESH: Duration = Duration::from_millis(250);
const MISS_REFRESH: Duration = Duration::from_millis(20);
const SYSTEM_PIDS: [u32; 2] = [0, 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Event,
    Table,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Event => "по событию",
            Source::Table => "по таблице TCP",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    pub name: Option<String>,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Conn {
    pub local_port: u16,
    pub remote: IpAddr,
    pub remote_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketEvent {
    pub event: u32,
    pub pid: u32,
    pub conn: Conn,
    pub tcp: bool,
}

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

pub fn decode_socket(a: &Address) -> Option<SocketEvent> {
    let d = &a.data;
    let tcp = match d[56] {
        6 => true,
        17 => false,
        _ => return None,
    };
    let remote = decode_addr([u32_at(d, 36), u32_at(d, 40), u32_at(d, 44), u32_at(d, 48)], a.ipv6());
    Some(SocketEvent {
        event: a.event(),
        pid: u32_at(d, 16),
        conn: Conn { local_port: u16::from_le_bytes([d[52], d[53]]), remote, remote_port: u16::from_le_bytes([d[54], d[55]]) },
        tcp,
    })
}

pub fn tcp_table() -> Vec<(Conn, u32)> {
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
                let (lport, ip, rport, pid) = if family == AF_INET {
                    (u16::from_be_bytes([b[8], b[9]]), IpAddr::V4(Ipv4Addr::new(b[12], b[13], b[14], b[15])), u16::from_be_bytes([b[16], b[17]]), u32_at(b, 20))
                } else {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[24..40]);
                    let v6 = Ipv6Addr::from(o);
                    let ip = v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6));
                    (u16::from_be_bytes([b[20], b[21]]), ip, u16::from_be_bytes([b[44], b[45]]), u32_at(b, 52))
                };
                out.push((Conn { local_port: lport, remote: ip, remote_port: rport }, pid));
            }
        }
    }
    out
}

pub fn process_names() -> HashMap<u32, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    let mut out = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            out.insert(e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase());
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

pub struct ProcMap {
    conns: HashMap<Conn, (u32, Source)>,
    pub system_events: u64,
    names: HashMap<u32, String>,
    table_at: Option<Instant>,
    names_at: Option<Instant>,
    table: Box<dyn FnMut() -> Vec<(Conn, u32)> + Send>,
    snapshot: Box<dyn FnMut() -> HashMap<u32, String> + Send>,
}

impl ProcMap {
    pub fn new(table: Box<dyn FnMut() -> Vec<(Conn, u32)> + Send>, snapshot: Box<dyn FnMut() -> HashMap<u32, String> + Send>) -> Self {
        let mut m = ProcMap { conns: HashMap::new(), system_events: 0, names: HashMap::new(), table_at: None, names_at: None, table, snapshot };
        m.reload_table(Instant::now());
        m
    }

    pub fn system() -> Self {
        ProcMap::new(Box::new(tcp_table), Box::new(process_names))
    }

    fn reload_table(&mut self, now: Instant) {
        self.table_at = Some(now);
        for (c, pid) in (self.table)() {
            if !SYSTEM_PIDS.contains(&pid) {
                self.conns.insert(c, (pid, Source::Table));
            }
        }
    }

    pub fn on_event(&mut self, ev: &SocketEvent) {
        if !ev.tcp {
            return;
        }
        match ev.event {
            EVENT_CONNECT if SYSTEM_PIDS.contains(&ev.pid) => self.system_events += 1,
            EVENT_CONNECT => {
                self.conns.insert(ev.conn, (ev.pid, Source::Event));
            }
            EVENT_CLOSE => {
                self.conns.remove(&ev.conn);
            }
            _ => {}
        }
    }

    pub fn pid(&mut self, conn: &Conn, now: Instant) -> Option<(u32, Source)> {
        if let Some(p) = self.conns.get(conn) {
            return Some(*p);
        }
        if self.table_at.is_none_or(|t| now.duration_since(t) >= MISS_REFRESH) {
            self.reload_table(now);
        }
        self.conns.get(conn).copied()
    }

    pub fn name(&mut self, pid: u32, now: Instant) -> Option<String> {
        if let Some(n) = self.names.get(&pid) {
            return Some(n.clone());
        }
        if self.names_at.is_none_or(|t| now.duration_since(t) >= REFRESH) {
            self.names_at = Some(now);
            self.names = (self.snapshot)();
        }
        self.names.get(&pid).cloned()
    }

    pub fn owner(&mut self, conn: &Conn, now: Instant) -> Option<Owner> {
        let (pid, source) = self.pid(conn, now)?;
        Some(Owner { pid, name: self.name(pid, now), source })
    }

    pub fn app(&mut self, conn: &Conn, now: Instant) -> Option<String> {
        self.owner(conn, now)?.name
    }

    pub fn tracked(&self) -> usize {
        self.conns.len()
    }

    pub fn forget_dead(&mut self, alive: &HashSet<u32>) {
        self.conns.retain(|_, (p, _)| alive.contains(p));
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Apps {
    pub all: HashSet<String>,
    pub skip: HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRule {
    Skip,
    All,
    Lists,
}

impl Apps {
    pub fn parse(all: &[String], skip: &[String]) -> Apps {
        let norm = |v: &[String]| -> HashSet<String> {
            v.iter()
                .flat_map(|s| s.split([',', ';']))
                .map(|s| s.trim().trim_matches('"').to_lowercase())
                .filter(|s| !s.is_empty())
                .map(|s| if s.ends_with(".exe") { s } else { format!("{s}.exe") })
                .collect()
        };
        Apps { all: norm(all), skip: norm(skip) }
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty() && self.skip.is_empty()
    }

    pub fn rule(&self, app: Option<&str>) -> AppRule {
        match app {
            Some(a) if self.skip.contains(a) => AppRule::Skip,
            Some(a) if self.all.contains(a) => AppRule::All,
            _ => AppRule::Lists,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn conn(port: u16) -> Conn {
        Conn { local_port: port, remote: "1.2.3.4".parse().unwrap(), remote_port: 443 }
    }

    #[test]
    fn decodes_socket_connect() {
        let mut a = Address { bits: 3 | (EVENT_CONNECT << 8), ..Default::default() };
        a.data[16..20].copy_from_slice(&4242u32.to_le_bytes());
        a.data[36..40].copy_from_slice(&u32::from(Ipv4Addr::new(1, 2, 3, 4)).to_le_bytes());
        a.data[40..44].copy_from_slice(&0xFFFFu32.to_le_bytes());
        a.data[52..54].copy_from_slice(&50000u16.to_le_bytes());
        a.data[54..56].copy_from_slice(&443u16.to_le_bytes());
        a.data[56] = 6;
        assert_eq!(decode_socket(&a), Some(SocketEvent { event: EVENT_CONNECT, pid: 4242, conn: conn(50000), tcp: true }));
        a.data[56] = 1;
        assert_eq!(decode_socket(&a), None);
    }

    #[test]
    fn map_uses_events_then_table() {
        let table = Arc::new(Mutex::new(vec![(conn(1000), 7u32)]));
        let t2 = table.clone();
        let mut m = ProcMap::new(
            Box::new(move || t2.lock().unwrap().clone()),
            Box::new(|| HashMap::from([(7, "chrome.exe".to_string()), (9, "curl.exe".to_string())])),
        );
        let now = Instant::now();
        assert_eq!(m.app(&conn(1000), now).as_deref(), Some("chrome.exe"));
        m.on_event(&SocketEvent { event: EVENT_CONNECT, pid: 9, conn: conn(2000), tcp: true });
        assert_eq!(m.app(&conn(2000), now).as_deref(), Some("curl.exe"));
        assert_eq!(m.owner(&conn(2000), now).unwrap().source, Source::Event);
        m.on_event(&SocketEvent { event: EVENT_CONNECT, pid: 4, conn: conn(2000), tcp: true });
        assert_eq!(m.app(&conn(2000), now).as_deref(), Some("curl.exe"));
        assert_eq!(m.system_events, 1);
        m.on_event(&SocketEvent { event: EVENT_CLOSE, pid: 9, conn: conn(2000), tcp: true });
        assert_eq!(m.pid(&conn(2000), now), None);
        table.lock().unwrap().push((conn(3000), 9));
        table.lock().unwrap().push((conn(3100), 4));
        assert_eq!(m.pid(&conn(3000), now), None);
        assert_eq!(m.pid(&conn(3000), now + MISS_REFRESH), Some((9, Source::Table)));
        assert_eq!(m.pid(&conn(3100), now + REFRESH), None);
        m.on_event(&SocketEvent { event: EVENT_CONNECT, pid: 5, conn: conn(4000), tcp: false });
        assert_eq!(m.pid(&conn(4000), now), None);
        assert_eq!(m.tracked(), 2);
        m.forget_dead(&HashSet::from([9]));
        assert_eq!(m.tracked(), 1);
    }

    #[test]
    fn app_rules() {
        let a = Apps::parse(&["Discord, chrome.exe".into()], &["steam.exe;\"Bank.exe\"".into()]);
        assert_eq!(a.all, HashSet::from(["discord.exe".to_string(), "chrome.exe".to_string()]));
        assert_eq!(a.rule(Some("steam.exe")), AppRule::Skip);
        assert_eq!(a.rule(Some("bank.exe")), AppRule::Skip);
        assert_eq!(a.rule(Some("discord.exe")), AppRule::All);
        assert_eq!(a.rule(Some("firefox.exe")), AppRule::Lists);
        assert_eq!(a.rule(None), AppRule::Lists);
        let both = Apps::parse(&["x.exe".into()], &["x.exe".into()]);
        assert_eq!(both.rule(Some("x.exe")), AppRule::Skip);
        assert!(Apps::default().is_empty());
    }

    #[test]
    fn real_system_tables_work() {
        let names = process_names();
        assert!(names.values().any(|n| n == "system" || n.ends_with(".exe")));
        let _ = tcp_table();
    }
}
