pub mod control;
pub mod page;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

pub const DEFAULT_PORT: u16 = 8881;
const HANDSHAKE: Duration = Duration::from_secs(20);
const CONNECT: Duration = Duration::from_secs(10);
const HEAD_LIMIT: usize = 16 * 1024;
const UDP_PEERS: usize = 4096;
const UDP_NAMES: usize = 256;

#[derive(Default)]
pub struct Stats {
    pub sharing: std::sync::atomic::AtomicBool,
    pub active: AtomicUsize,
    pub total: AtomicUsize,
    pub failed: AtomicUsize,
    pub up: AtomicU64,
    pub down: AtomicU64,
    pub clients: Mutex<HashSet<IpAddr>>,
    pub last_error: Mutex<String>,
}

impl Stats {
    fn error(&self, e: impl Into<String>) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().unwrap() = e.into();
    }
}

pub struct Handle {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub stats: Arc<Stats>,
    pub port: u16,
}

impl Handle {
    pub fn stop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn is_lan(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local(),
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return is_lan(&IpAddr::V4(m));
            }
            let s = v.segments()[0];
            v.is_loopback() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
        }
    }
}

pub fn public(ip: &IpAddr) -> bool {
    if is_lan(ip) || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(o[0] == 0 || (o[0] == 100 && (o[1] & 0xc0) == 64) || v.is_broadcast())
        }
        IpAddr::V6(v) => v.to_ipv4_mapped().is_none_or(|m| public(&IpAddr::V4(m))),
    }
}

fn routed_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:53").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (is_lan(&ip) && !ip.is_loopback()).then_some(ip)
}

fn home_adapters() -> Vec<IpAddr> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GAA_FLAG_INCLUDE_GATEWAYS, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH};
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    let mut size: u32 = 32 * 1024;
    let mut buf: Vec<u64> = Vec::new();
    let mut out = Vec::new();
    unsafe {
        let mut rc = 0;
        for _ in 0..3 {
            buf = vec![0u64; size as usize / 8 + 1];
            rc = GetAdaptersAddresses(AF_INET as u32, GAA_FLAG_INCLUDE_GATEWAYS, std::ptr::null(), buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH, &mut size);
            if rc != 111 {
                break;
            }
        }
        if rc != 0 {
            return out;
        }
        let mut a = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !a.is_null() {
            let ad = &*a;
            if ad.OperStatus == IfOperStatusUp && matches!(ad.IfType, 6 | 71) && !ad.FirstGatewayAddress.is_null() {
                let mut u = ad.FirstUnicastAddress;
                while !u.is_null() {
                    let sa = (*u).Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = &*(sa as *const SOCKADDR_IN);
                        let ip = IpAddr::from(sin.sin_addr.S_un.S_addr.to_ne_bytes());
                        if is_lan(&ip) && !ip.is_loopback() && !matches!(ip, IpAddr::V4(v) if v.is_link_local()) {
                            out.push(ip);
                        }
                    }
                    u = (*u).Next;
                }
            }
            a = ad.Next;
        }
    }
    out
}

pub fn lan_ip() -> Option<IpAddr> {
    let homes = home_adapters();
    let routed = routed_ip();
    match routed {
        Some(r) if homes.is_empty() || homes.contains(&r) => Some(r),
        _ => homes.first().copied().or(routed),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Socks { host: String, port: u16 },
    Connect { host: String, port: u16 },
    Http { host: String, port: u16, head: Vec<u8> },
    Local { path: String },
    Udp,
}

pub fn parse_http(head: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host(target, 443)?;
        return Some(Request::Connect { host, port });
    }
    if target.starts_with('/') {
        return Some(Request::Local { path: target.to_string() });
    }
    let rest = target.strip_prefix("http://").or_else(|| target.strip_prefix("HTTP://"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = split_host(authority, 80)?;
    let mut out = format!("{method} {path} {version}\r\n");
    for l in lines {
        if l.is_empty() {
            break;
        }
        let name = l.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if matches!(name.as_str(), "proxy-connection" | "proxy-authorization" | "connection" | "keep-alive") {
            continue;
        }
        out.push_str(l);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    Some(Request::Http { host, port, head: out.into_bytes() })
}

fn split_host(s: &str, default: u16) -> Option<(String, u16)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(default))?;
        return Some((h.to_string(), port));
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => Some((h.to_string(), p.parse().ok()?)),
        _ => (!s.is_empty()).then(|| (s.to_string(), default)),
    }
}

async fn read_head(s: &mut TcpStream, mut buf: Vec<u8>) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Ok((buf, rest));
        }
        if buf.len() > HEAD_LIMIT {
            anyhow::bail!("слишком длинный запрос");
        }
        let n = s.read(&mut chunk).await?;
        if n == 0 {
            anyhow::bail!("клиент закрыл соединение");
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn socks(s: &mut TcpStream) -> anyhow::Result<Request> {
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).await?;
    let mut methods = vec![0u8; hdr[1] as usize];
    s.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        s.write_all(&[5, 0xff]).await?;
        anyhow::bail!("SOCKS5 без пароля не поддержан клиентом");
    }
    s.write_all(&[5, 0]).await?;
    let mut req = [0u8; 4];
    s.read_exact(&mut req).await?;
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            s.read_exact(&mut a).await?;
            IpAddr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            s.read_exact(&mut name).await?;
            String::from_utf8(name)?
        }
        4 => {
            let mut a = [0u8; 16];
            s.read_exact(&mut a).await?;
            IpAddr::from(a).to_string()
        }
        _ => anyhow::bail!("неизвестный тип адреса SOCKS5"),
    };
    let mut p = [0u8; 2];
    s.read_exact(&mut p).await?;
    match req[1] {
        1 => Ok(Request::Socks { host, port: u16::from_be_bytes(p) }),
        3 => Ok(Request::Udp),
        _ => {
            s.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("SOCKS5: неизвестная команда {}", req[1])
        }
    }
}

async fn udp_target<'a>(pkt: &'a [u8], names: &mut HashMap<(String, u16), SocketAddr>, local: bool) -> Option<(SocketAddr, &'a [u8])> {
    if pkt.len() < 4 || pkt[2] != 0 {
        return None;
    }
    let (target, rest) = match pkt[3] {
        1 if pkt.len() >= 10 => {
            let ip = Ipv4Addr::new(pkt[4], pkt[5], pkt[6], pkt[7]);
            (SocketAddr::from((ip, u16::from_be_bytes([pkt[8], pkt[9]]))), &pkt[10..])
        }
        3 if pkt.len() >= 5 => {
            let l = pkt[4] as usize;
            if pkt.len() < 7 + l {
                return None;
            }
            let host = std::str::from_utf8(&pkt[5..5 + l]).ok()?.to_string();
            let port = u16::from_be_bytes([pkt[5 + l], pkt[6 + l]]);
            let key = (host, port);
            let addr = match names.get(&key) {
                Some(a) => *a,
                None => {
                    let a = timeout(CONNECT, tokio::net::lookup_host((key.0.as_str(), port))).await.ok()?.ok()?.find(SocketAddr::is_ipv4)?;
                    if names.len() < UDP_NAMES {
                        names.insert(key, a);
                    }
                    a
                }
            };
            (addr, &pkt[7 + l..])
        }
        _ => return None,
    };
    let ip = target.ip();
    ((local && !ip.is_unspecified()) || public(&ip)).then_some((target, rest))
}

async fn udp_relay(mut s: TcpStream, stats: Arc<Stats>, local: bool) -> anyhow::Result<()> {
    let peer = s.peer_addr()?.ip();
    let here = match s.local_addr()?.ip() {
        IpAddr::V4(v) => v,
        IpAddr::V6(v) => v.to_ipv4_mapped().unwrap_or(Ipv4Addr::LOCALHOST),
    };
    let relay = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let mut reply = vec![5, 0, 0, 1];
    reply.extend_from_slice(&here.octets());
    reply.extend_from_slice(&relay.local_addr()?.port().to_be_bytes());
    s.write_all(&reply).await?;
    let mut client: Option<SocketAddr> = None;
    let mut sent: HashSet<SocketAddr> = HashSet::new();
    let mut names = HashMap::new();
    let mut buf = vec![0u8; 65535];
    let mut ctl = [0u8; 64];
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            r = s.read(&mut ctl) => {
                if matches!(r, Ok(0) | Err(_)) {
                    break;
                }
            }
            _ = tick.tick() => {
                if !stats.sharing.load(Ordering::Relaxed) {
                    let _ = s.set_zero_linger();
                    break;
                }
            }
            r = relay.recv_from(&mut buf) => {
                let Ok((n, from)) = r else { continue };
                if client == Some(from) || (client.is_none() && from.ip() == peer) {
                    client = Some(from);
                    let Some((target, data)) = udp_target(&buf[..n], &mut names, local).await else { continue };
                    if relay.send_to(data, target).await.is_ok() {
                        if sent.len() >= UDP_PEERS {
                            sent.clear();
                        }
                        sent.insert(target);
                        stats.up.fetch_add(data.len() as u64, Ordering::Relaxed);
                    }
                } else if let (Some(c), SocketAddr::V4(f)) = (client, from) {
                    if !sent.contains(&from) {
                        continue;
                    }
                    let mut out = Vec::with_capacity(n + 10);
                    out.extend_from_slice(&[0, 0, 0, 1]);
                    out.extend_from_slice(&f.ip().octets());
                    out.extend_from_slice(&f.port().to_be_bytes());
                    out.extend_from_slice(&buf[..n]);
                    if relay.send_to(&out, c).await.is_ok() {
                        stats.down.fetch_add(n as u64, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn dial(host: &str, port: u16, local: bool) -> anyhow::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = timeout(CONNECT, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| anyhow::anyhow!("{host}: DNS не ответил"))??
        .filter(|a| (local && !a.ip().is_unspecified()) || public(&a.ip()))
        .collect();
    if addrs.is_empty() {
        anyhow::bail!("{host}: адрес недоступен через раздачу");
    }
    let mut last = None;
    for a in addrs {
        match timeout(CONNECT, TcpStream::connect(a)).await {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => last = Some(e.to_string()),
            Err(_) => last = Some("таймаут".into()),
        }
    }
    anyhow::bail!("{host}:{port}: {}", last.unwrap_or_default())
}

async fn request(s: &mut TcpStream, first: u8) -> anyhow::Result<(Request, Vec<u8>)> {
    if first == 5 {
        return Ok((socks(s).await?, Vec::new()));
    }
    let (head, rest) = read_head(s, Vec::new()).await?;
    Ok((parse_http(&head).ok_or_else(|| anyhow::anyhow!("непонятный запрос"))?, rest))
}

async fn handle(mut s: TcpStream, stats: Arc<Stats>, port: u16, local: bool) -> anyhow::Result<()> {
    let mut first = [0u8; 1];
    let (req, extra) = timeout(HANDSHAKE, async {
        s.peek(&mut first).await?;
        request(&mut s, first[0]).await
    })
    .await
    .map_err(|_| anyhow::anyhow!("клиент не прислал запрос"))??;
    let (host, port, preface, reply): (String, u16, Vec<u8>, &[u8]) = match req {
        Request::Local { path } => {
            let addr = s.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| format!("127.0.0.1:{port}"));
            let body = page::respond(&path, &addr, stats.sharing.load(Ordering::Relaxed));
            s.write_all(&body).await?;
            return Ok(());
        }
        _ if !stats.sharing.load(Ordering::Relaxed) => {
            let _ = s.set_zero_linger();
            return Ok(());
        }
        Request::Udp => return udp_relay(s, stats, local).await,
        Request::Socks { host, port } => (host, port, extra, &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]),
        Request::Connect { host, port } => (host, port, extra, b"HTTP/1.1 200 Connection established\r\n\r\n"),
        Request::Http { host, port, mut head } => {
            head.extend_from_slice(&extra);
            (host, port, head, b"")
        }
    };
    let mut up = match dial(&host, port, local).await {
        Ok(u) => u,
        Err(e) => {
            let fail: &[u8] = if first[0] == 5 { &[5, 4, 0, 1, 0, 0, 0, 0, 0, 0] } else { b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n" };
            let _ = s.write_all(fail).await;
            return Err(e);
        }
    };
    if !reply.is_empty() {
        s.write_all(reply).await?;
    }
    if !preface.is_empty() {
        up.write_all(&preface).await?;
        stats.up.fetch_add(preface.len() as u64, Ordering::Relaxed);
    }
    let off = async {
        while stats.sharing.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    tokio::select! {
        r = tokio::io::copy_bidirectional(&mut s, &mut up) => {
            let (a, b) = r.unwrap_or((0, 0));
            stats.up.fetch_add(a, Ordering::Relaxed);
            stats.down.fetch_add(b, Ordering::Relaxed);
        }
        _ = off => {
            let _ = s.set_zero_linger();
        }
    }
    Ok(())
}

async fn serve(listener: TcpListener, stats: Arc<Stats>, port: u16, local: bool, mut stop: oneshot::Receiver<()>) {
    loop {
        tokio::select! {
            _ = &mut stop => break,
            r = listener.accept() => {
                let Ok((s, peer)) = r else { continue };
                if !is_lan(&peer.ip()) {
                    continue;
                }
                let stats = stats.clone();
                tokio::spawn(async move {
                    let counted = stats.sharing.load(Ordering::Relaxed);
                    if counted {
                        stats.active.fetch_add(1, Ordering::Relaxed);
                        stats.total.fetch_add(1, Ordering::Relaxed);
                        stats.clients.lock().unwrap().insert(peer.ip());
                    }
                    if let Err(e) = handle(s, stats.clone(), port, local).await {
                        if counted {
                            stats.error(e.to_string());
                        }
                    }
                    if counted {
                        stats.active.fetch_sub(1, Ordering::Relaxed);
                    }
                });
            }
        }
    }
}

pub fn start(port: u16) -> anyhow::Result<Handle> {
    start_on("0.0.0.0", port, false)
}

fn start_on(bind: &str, port: u16, local: bool) -> anyhow::Result<Handle> {
    let listener = std::net::TcpListener::bind((bind, port)).map_err(|e| anyhow::anyhow!("порт {port} занят или недоступен: {e}"))?;
    listener.set_nonblocking(true)?;
    let stats = Arc::new(Stats::default());
    stats.sharing.store(true, Ordering::Relaxed);
    let st = stats.clone();
    let (tx, rx) = oneshot::channel();
    let thread = std::thread::Builder::new().name("lan-proxy".into()).spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        rt.block_on(async move {
            let Ok(listener) = TcpListener::from_std(listener) else { return };
            serve(listener, st, port, local, rx).await;
        });
    })?;
    Ok(Handle { stop: Some(tx), thread: Some(thread), stats, port })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn lan_addresses() {
        for ip in ["192.168.1.5", "10.0.0.2", "172.20.1.1", "127.0.0.1", "169.254.3.3", "fe80::1", "fd00::5", "::ffff:192.168.0.9"] {
            assert!(is_lan(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "2a00:1450::1", "::ffff:1.1.1.1"] {
            assert!(!is_lan(&ip.parse().unwrap()), "{ip}");
            assert!(public(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["192.168.1.1", "10.8.0.1", "172.17.0.1", "100.64.1.1", "0.0.0.0", "0.1.2.3", "255.255.255.255", "224.0.0.1", "::", "::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(!public(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn http_requests_parsed() {
        assert_eq!(parse_http(b"CONNECT www.youtube.com:443 HTTP/1.1\r\nHost: x\r\n\r\n"), Some(Request::Connect { host: "www.youtube.com".into(), port: 443 }));
        assert_eq!(parse_http(b"CONNECT [2a00::1]:8443 HTTP/1.1\r\n\r\n"), Some(Request::Connect { host: "2a00::1".into(), port: 8443 }));
        assert_eq!(parse_http(b"GET /proxy.pac HTTP/1.1\r\nHost: 192.168.1.5:8881\r\n\r\n"), Some(Request::Local { path: "/proxy.pac".into() }));
        let r = parse_http(b"GET http://example.com:8080/a?b=1 HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n").unwrap();
        assert_eq!(
            r,
            Request::Http {
                host: "example.com".into(),
                port: 8080,
                head: b"GET /a?b=1 HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\nConnection: close\r\n\r\n".to_vec()
            }
        );
        assert!(parse_http(b"garbage").is_none());
    }

    fn echo() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut s = s;
                    let mut buf = [0u8; 1024];
                    loop {
                        match s.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let _ = s.write_all(&buf[..n]);
                            }
                        }
                    }
                });
            }
        });
        p
    }

    #[test]
    fn socks_and_connect_forward() {
        let p = echo();
        let mut handle = start_on("127.0.0.1", 18892, true).unwrap();
        let mut c = std::net::TcpStream::connect(("127.0.0.1", 18892)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut r = [0u8; 2];
        c.read_exact(&mut r).unwrap();
        assert_eq!(r, [5, 0]);
        let mut req = vec![5, 1, 0, 3, 9];
        req.extend_from_slice(b"localhost");
        req.extend_from_slice(&p.to_be_bytes());
        c.write_all(&req).unwrap();
        let mut ok = [0u8; 10];
        c.read_exact(&mut ok).unwrap();
        assert_eq!(&ok[..2], &[5, 0]);
        c.write_all(b"ping").unwrap();
        let mut back = [0u8; 4];
        c.read_exact(&mut back).unwrap();
        assert_eq!(&back, b"ping");
        let mut h = std::net::TcpStream::connect(("127.0.0.1", 18892)).unwrap();
        h.write_all(format!("CONNECT 127.0.0.1:{p} HTTP/1.1\r\n\r\nhello").as_bytes()).unwrap();
        let mut got = vec![0u8; 44];
        h.read_exact(&mut got).unwrap();
        assert_eq!(String::from_utf8_lossy(&got), "HTTP/1.1 200 Connection established\r\n\r\nhello");
        drop(c);
        drop(h);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(handle.stats.total.load(Ordering::Relaxed), 2);
        assert!(handle.stats.up.load(Ordering::Relaxed) >= 9);
        handle.stop();
    }

    #[test]
    fn socks_udp_associate_relays() {
        let echo = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let ep = echo.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut b = [0u8; 2048];
            while let Ok((n, from)) = echo.recv_from(&mut b) {
                let _ = echo.send_to(&b[..n], from);
            }
        });
        let mut handle = start_on("127.0.0.1", 18896, true).unwrap();
        let mut c = std::net::TcpStream::connect(("127.0.0.1", 18896)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut r = [0u8; 2];
        c.read_exact(&mut r).unwrap();
        c.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        let mut rep = [0u8; 10];
        c.read_exact(&mut rep).unwrap();
        assert_eq!(&rep[..4], &[5, 0, 0, 1]);
        let relay = SocketAddr::from((Ipv4Addr::new(rep[4], rep[5], rep[6], rep[7]), u16::from_be_bytes([rep[8], rep[9]])));
        let u = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        u.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut pkt = vec![0, 0, 0, 1, 127, 0, 0, 1];
        pkt.extend_from_slice(&ep.to_be_bytes());
        pkt.extend_from_slice(b"voice");
        u.send_to(&pkt, relay).unwrap();
        let mut back = [0u8; 64];
        let n = u.recv(&mut back).unwrap();
        assert_eq!(&back[..4], &[0, 0, 0, 1]);
        assert_eq!(&back[4..8], &[127, 0, 0, 1]);
        assert_eq!(u16::from_be_bytes([back[8], back[9]]), ep);
        assert_eq!(&back[10..n], b"voice");
        let mut named = vec![0, 0, 0, 3, 9];
        named.extend_from_slice(b"localhost");
        named.extend_from_slice(&ep.to_be_bytes());
        named.extend_from_slice(b"call");
        u.send_to(&named, relay).unwrap();
        let n = u.recv(&mut back).unwrap();
        assert_eq!(&back[10..n], b"call");
        let stranger = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let _ = stranger.send_to(&pkt, relay);
        u.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        assert!(u.recv(&mut back).is_err());
        drop(c);
        handle.stop();
    }

    #[test]
    fn udp_to_local_refused_without_flag() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut names = HashMap::new();
        let mut p = vec![0, 0, 0, 1, 192, 168, 1, 1, 0, 53];
        p.extend_from_slice(b"x");
        assert!(rt.block_on(udp_target(&p, &mut names, false)).is_none());
        let mut q = vec![0, 0, 0, 1, 8, 8, 8, 8, 0, 53];
        q.extend_from_slice(b"dns");
        let (t, d) = rt.block_on(udp_target(&q, &mut names, false)).unwrap();
        assert_eq!((t, d), (SocketAddr::from(([8, 8, 8, 8], 53)), &b"dns"[..]));
        assert!(rt.block_on(udp_target(&[0, 0, 1, 1, 8, 8, 8, 8, 0, 53], &mut names, false)).is_none());
    }

    #[test]
    #[ignore]
    fn live_through_lan_address() {
        let ip = lan_ip().expect("нет локального адреса");
        let mut handle = start(18893).unwrap();
        let proxy = format!("{ip}:18893");
        let curl = |args: &[&str]| {
            let out = std::process::Command::new("curl.exe").args(["-s", "-o", "NUL", "-m", "15", "-w", "%{http_code}"]).args(args).output().unwrap();
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        let http_proxy = format!("http://{proxy}");
        let socks = format!("socks5h://{proxy}");
        let results = [
            ("CONNECT https", curl(&["-x", &http_proxy, "https://www.google.com/"])),
            ("SOCKS5 https", curl(&["-x", &socks, "https://www.cloudflare.com/"])),
            ("plain http", curl(&["-x", &http_proxy, "http://example.com/"])),
            ("page", curl(&[&format!("http://{proxy}/")])),
            ("pac", curl(&[&format!("http://{proxy}/proxy.pac")])),
        ];
        for (name, code) in &results {
            println!("{name}: {code}");
        }
        std::thread::sleep(Duration::from_millis(300));
        println!(
            "total={} failed={} up={} down={} clients={:?} err={}",
            handle.stats.total.load(Ordering::Relaxed),
            handle.stats.failed.load(Ordering::Relaxed),
            handle.stats.up.load(Ordering::Relaxed),
            handle.stats.down.load(Ordering::Relaxed),
            handle.stats.clients.lock().unwrap(),
            handle.stats.last_error.lock().unwrap()
        );
        assert!(results.iter().all(|(_, c)| c.starts_with('2') || c.starts_with('3')));
        handle.stop();
    }

    #[test]
    #[ignore]
    fn live_udp_dns_through_lan_address() {
        let ip = lan_ip().expect("нет локального адреса");
        let mut handle = start(18897).unwrap();
        let mut c = std::net::TcpStream::connect((ip, 18897)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut r = [0u8; 2];
        c.read_exact(&mut r).unwrap();
        c.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        let mut rep = [0u8; 10];
        c.read_exact(&mut rep).unwrap();
        let relay = SocketAddr::from((Ipv4Addr::new(rep[4], rep[5], rep[6], rep[7]), u16::from_be_bytes([rep[8], rep[9]])));
        println!("relay {relay}");
        let u = std::net::UdpSocket::bind((ip, 0)).unwrap();
        u.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut pkt = vec![0, 0, 0, 1, 8, 8, 8, 8, 0, 53];
        pkt.extend_from_slice(&[0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 7]);
        pkt.extend_from_slice(b"example");
        pkt.extend_from_slice(&[3]);
        pkt.extend_from_slice(b"com");
        pkt.extend_from_slice(&[0, 0, 1, 0, 1]);
        u.send_to(&pkt, relay).unwrap();
        let mut back = [0u8; 512];
        let n = u.recv(&mut back).unwrap();
        println!("answer {} bytes from {}.{}.{}.{}", n - 10, back[4], back[5], back[6], back[7]);
        assert_eq!(&back[4..8], &[8, 8, 8, 8]);
        assert_eq!(&back[10..12], &[0x12, 0x34]);
        println!("up={} down={}", handle.stats.up.load(Ordering::Relaxed), handle.stats.down.load(Ordering::Relaxed));
        drop(c);
        handle.stop();
    }

    #[test]
    #[ignore]
    fn serve_for_browser() {
        let mut handle = start(18894).unwrap();
        std::thread::sleep(Duration::from_secs(90));
        handle.stop();
    }

    #[test]
    fn soft_off_keeps_pac_direct() {
        let p = echo();
        let mut handle = start_on("127.0.0.1", 18895, true).unwrap();
        handle.stats.sharing.store(false, Ordering::Relaxed);
        let mut c = std::net::TcpStream::connect(("127.0.0.1", 18895)).unwrap();
        c.write_all(b"GET /proxy.pac HTTP/1.1\r\n\r\n").unwrap();
        let mut body = String::new();
        c.read_to_string(&mut body).unwrap();
        assert!(body.contains("return \"DIRECT\";") && !body.contains("PROXY"));
        let mut c = std::net::TcpStream::connect(("127.0.0.1", 18895)).unwrap();
        c.write_all(format!("CONNECT 127.0.0.1:{p} HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
        let mut resp = Vec::new();
        let _ = c.read_to_end(&mut resp);
        assert!(resp.is_empty());
        assert_eq!(handle.stats.total.load(Ordering::Relaxed), 0);
        handle.stop();
    }

    #[test]
    fn local_page_and_loopback_refused() {
        let mut handle = start_on("127.0.0.1", 18891, false).unwrap();
        let mut c = std::net::TcpStream::connect(("127.0.0.1", 18891)).unwrap();
        c.write_all(b"GET /proxy.pac HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut body = String::new();
        c.read_to_string(&mut body).unwrap();
        assert!(body.contains("FindProxyForURL") && body.contains(":18891"));
        let p = echo();
        let mut c = std::net::TcpStream::connect(("127.0.0.1", 18891)).unwrap();
        c.write_all(format!("CONNECT 127.0.0.1:{p} HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
        let mut resp = String::new();
        c.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 502"));
        handle.stop();
    }
}
