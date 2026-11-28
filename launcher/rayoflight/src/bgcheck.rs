use crate::flows::Engine;
use crate::{desync, sys, tls};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const LONG: usize = 1800;
const CONNECT: Duration = Duration::from_secs(5);
const ANSWER: Duration = Duration::from_secs(6);
const TICK: Duration = Duration::from_secs(2);
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

pub fn hello(host: &str, long: bool) -> Option<Vec<u8>> {
    let mut h = tls::with_sni(desync::FAKE_DEFAULT, host)?;
    sys::random(&mut h[11..43]);
    let sid = h[43] as usize;
    sys::random(&mut h[44..44 + sid]);
    if long {
        h = tls::with_padding(&h, LONG)?;
    }
    Some(h)
}

fn check<A: Clone + Send + 'static>(engine: &Arc<Mutex<Engine<A>>>, host: &str, addr: SocketAddr, long: bool) {
    let Some(h) = hello(host, long) else { return };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, CONNECT) else { return };
    let Ok(port) = s.local_addr().map(|a| a.port()) else { return };
    if let Ok(mut e) = engine.lock() {
        e.add_bg_port(port);
    }
    let _ = s.set_nodelay(true);
    let _ = s.set_read_timeout(Some(ANSWER));
    if s.write_all(&h).is_ok() {
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf);
    }
    drop(s);
    if let Ok(mut e) = engine.lock() {
        e.remove_bg_port(port);
    }
}

pub fn spawn<A: Clone + Send + 'static>(engine: Arc<Mutex<Engine<A>>>, ended: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !ended.load(Ordering::SeqCst) {
            std::thread::sleep(TICK);
            let targets = engine.lock().map(|mut e| e.bg_targets(Instant::now())).unwrap_or_default();
            for (host, addr, parts) in targets {
                let engine = engine.clone();
                std::thread::spawn(move || check(&engine, &host, addr, parts >= 2));
            }
            let hosts = engine.lock().map(|mut e| e.freeze_targets(Instant::now())).unwrap_or_default();
            for host in hosts {
                let engine = engine.clone();
                std::thread::spawn(move || {
                    let (got, ended) = crate::dns::download(&host, crate::flows::FETCH_WANT, FETCH_TIMEOUT);
                    if let Ok(mut e) = engine.lock() {
                        e.freeze_verdict(&host, crate::flows::fetch_verdict(got, ended));
                    }
                });
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_hello_keeps_name_and_lengths() {
        let short = hello("community.akamai.steamstatic.com", false).unwrap();
        let long = hello("community.akamai.steamstatic.com", true).unwrap();
        assert_eq!(long.len(), LONG);
        for h in [&short, &long] {
            assert!(tls::is_client_hello(h));
            assert_eq!(tls::record_len(h), Some(h.len()));
            assert_eq!(tls::sni(h).as_deref(), Some("community.akamai.steamstatic.com"));
            assert_eq!(u32::from_be_bytes([0, h[6], h[7], h[8]]) as usize, h.len() - 9);
        }
        assert_ne!(short[11..43], hello("community.akamai.steamstatic.com", false).unwrap()[11..43]);
    }

    #[test]
    #[ignore]
    fn servers_answer_long_hello_live() {
        use std::net::ToSocketAddrs;
        for host in ["www.google.com", "discord.com", "community.akamai.steamstatic.com", "store.steampowered.com"] {
            for long in [false, true] {
                let addr = (host, 443).to_socket_addrs().unwrap().find(|a| a.is_ipv4()).unwrap();
                let mut s = TcpStream::connect_timeout(&addr, CONNECT).unwrap();
                s.set_read_timeout(Some(ANSWER)).unwrap();
                s.write_all(&hello(host, long).unwrap()).unwrap();
                let mut buf = [0u8; 6];
                s.read_exact(&mut buf).unwrap();
                assert_eq!((buf[0], buf[5]), (0x16, 0x02), "{host} long={long}");
            }
        }
    }
}
