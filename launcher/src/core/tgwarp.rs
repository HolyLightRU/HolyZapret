use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const KEY: &str = r"Software\HolyZapret";
const VALUE: &str = "TgWarp";
const HANDSHAKE: Duration = Duration::from_secs(6);

static ON: AtomicBool = AtomicBool::new(false);
static PORT: AtomicU16 = AtomicU16::new(super::warp::PROXY_PORT);

const V4: [(Ipv4Addr, u8); 10] = [
    (Ipv4Addr::new(91, 105, 192, 0), 23),
    (Ipv4Addr::new(91, 108, 4, 0), 22),
    (Ipv4Addr::new(91, 108, 8, 0), 22),
    (Ipv4Addr::new(91, 108, 12, 0), 22),
    (Ipv4Addr::new(91, 108, 16, 0), 22),
    (Ipv4Addr::new(91, 108, 20, 0), 22),
    (Ipv4Addr::new(91, 108, 56, 0), 22),
    (Ipv4Addr::new(95, 161, 64, 0), 20),
    (Ipv4Addr::new(149, 154, 160, 0), 20),
    (Ipv4Addr::new(185, 76, 151, 0), 24),
];

const V6: [(Ipv6Addr, u8); 5] = [
    (Ipv6Addr::new(0x2001, 0x0b28, 0xf23c, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x2001, 0x0b28, 0xf23d, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x2001, 0x0b28, 0xf23f, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x2001, 0x067c, 0x04e8, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x2a0a, 0xf280, 0, 0, 0, 0, 0, 0), 32),
];

pub fn telegram(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let a = u32::from(*v);
            V4.iter().any(|(net, bits)| a >> (32 - bits) == u32::from(*net) >> (32 - bits))
        }
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return telegram(&IpAddr::V4(m));
            }
            let a = u128::from(*v);
            V6.iter().any(|(net, bits)| a >> (128 - bits) == u128::from(*net) >> (128 - bits))
        }
    }
}

fn load() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .and_then(|k| k.get_value::<u32, _>(VALUE))
        .is_ok_and(|v| v != 0)
}

pub fn init() {
    ON.store(load(), Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value(VALUE, &(on as u32));
    }
}

static PROBE: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);

pub fn proxy_up() -> bool {
    let mut p = PROBE.lock().unwrap();
    if let Some((at, up)) = *p {
        if at.elapsed() < Duration::from_secs(5) {
            return up;
        }
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT.load(Ordering::Relaxed)));
    let up = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok();
    *p = Some((std::time::Instant::now(), up));
    up
}

pub fn via() -> Option<SocketAddr> {
    enabled().then(|| SocketAddr::from((Ipv4Addr::LOCALHOST, PORT.load(Ordering::Relaxed))))
}

fn refused(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionRefused, format!("WARP: {what}"))
}

async fn handshake(s: &mut TcpStream, target: SocketAddr) -> io::Result<()> {
    s.write_all(&[5, 1, 0]).await?;
    let mut hello = [0u8; 2];
    s.read_exact(&mut hello).await?;
    if hello != [5, 0] {
        return Err(refused("прокси требует пароль"));
    }
    let mut req = vec![5, 1, 0];
    match target.ip() {
        IpAddr::V4(v) => {
            req.push(1);
            req.extend_from_slice(&v.octets());
        }
        IpAddr::V6(v) => {
            req.push(4);
            req.extend_from_slice(&v.octets());
        }
    }
    req.extend_from_slice(&target.port().to_be_bytes());
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0 {
        return Err(refused(&format!("прокси не соединил с {target}, код {}", head[1])));
    }
    let rest = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            l[0] as usize
        }
        _ => return Err(refused("непонятный ответ прокси")),
    };
    let mut skip = vec![0u8; rest + 2];
    s.read_exact(&mut skip).await?;
    Ok(())
}

pub async fn connect(proxy: SocketAddr, target: SocketAddr) -> io::Result<TcpStream> {
    timeout(HANDSHAKE, async {
        let mut s = TcpStream::connect(proxy).await?;
        let _ = s.set_nodelay(true);
        handshake(&mut s, target).await?;
        Ok(s)
    })
    .await
    .map_err(|_| refused("прокси не ответил"))?
}

#[cfg(test)]
pub mod fake {
    use super::*;

    pub fn socks() -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for s in l.incoming().flatten() {
                let h = h.clone();
                std::thread::spawn(move || {
                    let mut c = s;
                    let mut b = [0u8; 3];
                    if c.read_exact(&mut b).is_err() {
                        return;
                    }
                    let _ = c.write_all(&[5, 0]);
                    let mut head = [0u8; 4];
                    if c.read_exact(&mut head).is_err() {
                        return;
                    }
                    let mut addr = vec![0u8; if head[3] == 4 { 16 } else { 4 }];
                    let mut port = [0u8; 2];
                    if c.read_exact(&mut addr).is_err() || c.read_exact(&mut port).is_err() {
                        return;
                    }
                    h.fetch_add(1, Ordering::Relaxed);
                    let _ = c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]);
                    let mut buf = [0u8; 1024];
                    loop {
                        match c.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let _ = c.write_all(&buf[..n]);
                            }
                        }
                    }
                });
            }
        });
        (port, hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_ranges() {
        for ip in ["149.154.167.51", "149.154.175.100", "149.154.171.5", "91.108.56.130", "91.105.192.100", "95.161.76.100", "2001:b28:f23d:f001::a", "2001:67c:4e8:f004::9", "::ffff:149.154.167.220"] {
            assert!(telegram(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["149.154.176.1", "91.108.24.1", "8.8.8.8", "2001:b28:f23e::1", "2a00:1450::1"] {
            assert!(!telegram(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn connects_through_socks() {
        let (port, hits) = fake::socks();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut s = connect(SocketAddr::from(([127, 0, 0, 1], port)), "149.154.167.51:443".parse().unwrap()).await.unwrap();
            s.write_all(b"mtproto").await.unwrap();
            let mut back = [0u8; 7];
            s.read_exact(&mut back).await.unwrap();
            assert_eq!(&back, b"mtproto");
            let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
            assert!(connect(closed, "149.154.167.51:443".parse().unwrap()).await.is_err());
        });
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }
}
