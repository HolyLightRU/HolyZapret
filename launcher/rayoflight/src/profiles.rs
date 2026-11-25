pub const CF_PORTS: [u16; 5] = [2053, 2083, 2087, 2096, 8443];
pub const CF_HOST: &str = "discord.media";
pub const MYIP_TCP_PORTS: [u16; 3] = [80, 443, 8443];
pub const VOICE: [(u16, u16); 2] = [(19294, 19344), (50000, 50100)];
pub const VOICE_REPEATS: usize = 6;
pub const GAME_UDP_REPEATS: usize = 10;
pub const GAME_UDP_PACKETS: u32 = 2;
pub const NAMES: [&str; 7] = ["http", "google", "cf", "myip-tcp", "myip-quic", "game-tcp", "game-udp"];
pub const VOICE_NAME: &str = "voice";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Own {
    pub http: bool,
    pub google: bool,
    pub cf: bool,
    pub myip_tcp: bool,
    pub myip_quic: bool,
    pub game_tcp: bool,
    pub game_udp: bool,
    pub voice: bool,
}

impl Own {
    pub fn parse(text: &str) -> Result<Own, String> {
        let mut o = Own::default();
        for name in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match name.to_ascii_lowercase().as_str() {
                "http" => o.http = true,
                "google" => o.google = true,
                "cf" => o.cf = true,
                "myip-tcp" => o.myip_tcp = true,
                "myip-quic" => o.myip_quic = true,
                "game-tcp" => o.game_tcp = true,
                "game-udp" => o.game_udp = true,
                "voice" => o.voice = true,
                other => return Err(format!("неизвестный профиль {other}: {}, {VOICE_NAME}", NAMES.join(", "))),
            }
        }
        Ok(o)
    }

    pub fn names(&self) -> Vec<&'static str> {
        let all = [
            (self.http, "http"),
            (self.google, "google"),
            (self.cf, "cf"),
            (self.myip_tcp, "myip-tcp"),
            (self.myip_quic, "myip-quic"),
            (self.game_tcp, "game-tcp"),
            (self.game_udp, "game-udp"),
            (self.voice, VOICE_NAME),
        ];
        all.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ports(Vec<(u16, u16)>);

impl Ports {
    pub fn parse(text: &str) -> Result<Ports, String> {
        let mut v = Vec::new();
        for part in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let bad = || format!("неверные порты: {part}");
            let (a, b) = match part.split_once('-') {
                Some((a, b)) => (a.trim().parse::<u16>().map_err(|_| bad())?, b.trim().parse::<u16>().map_err(|_| bad())?),
                None => {
                    let p = part.parse::<u16>().map_err(|_| bad())?;
                    (p, p)
                }
            };
            if a > b {
                return Err(bad());
            }
            v.push((a, b));
        }
        Ok(Ports(v))
    }

    pub fn contains(&self, port: u16) -> bool {
        self.0.iter().any(|(a, b)| (*a..=*b).contains(&port))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn ranges(&self) -> &[(u16, u16)] {
        &self.0
    }
}

pub fn in_voice(port: u16) -> bool {
    VOICE.iter().any(|(a, b)| (*a..=*b).contains(&port))
}

pub fn is_discord_or_stun(d: &[u8]) -> bool {
    let discovery = d.len() == 74 && d[0] == 0 && d[1] == 1 && d[2] == 0 && d[3] == 70;
    let stun = d.len() >= 20 && d[0] & 0xC0 == 0 && d[4..8] == [0x21, 0x12, 0xA4, 0x42];
    discovery || stun
}

const METHODS: [&str; 8] = ["GET ", "POST ", "HEAD ", "PUT ", "OPTIONS ", "DELETE ", "PATCH ", "CONNECT "];

pub fn is_http_request(d: &[u8]) -> bool {
    METHODS.iter().any(|m| d.starts_with(m.as_bytes()))
}

pub fn http_host(d: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&d[..d.len().min(4096)]);
    for line in text.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("host") {
                let v = v.trim();
                let host = if v.starts_with('[') { v.split(']').next().map(|h| h.trim_start_matches('[')).unwrap_or(v) } else { v.split(':').next().unwrap_or(v) };
                let host = host.trim_end_matches('.').to_ascii_lowercase();
                return (!host.is_empty()).then_some(host);
            }
        }
    }
    None
}

pub fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_and_ports() {
        let o = Own::parse("http, google,voice").unwrap();
        assert!(o.http && o.google && o.voice && !o.cf);
        assert_eq!(o.names(), vec!["http", "google", "voice"]);
        assert!(Own::parse("nope").is_err());
        let p = Ports::parse("1024-65535").unwrap();
        assert!(p.contains(1024) && p.contains(65535) && !p.contains(443));
        let q = Ports::parse("12, 80,1000-2000").unwrap();
        assert!(q.contains(12) && q.contains(1500) && !q.contains(81));
        assert!(Ports::parse("9-1").is_err() && Ports::parse("x").is_err());
        assert!(in_voice(19300) && in_voice(50050) && !in_voice(443));
    }

    #[test]
    fn recognizes_voice_and_http() {
        let mut disc = vec![0u8; 74];
        disc[1] = 1;
        disc[3] = 70;
        assert!(is_discord_or_stun(&disc));
        let mut stun = vec![0u8; 20];
        stun[1] = 1;
        stun[4..8].copy_from_slice(&[0x21, 0x12, 0xA4, 0x42]);
        assert!(is_discord_or_stun(&stun));
        assert!(!is_discord_or_stun(&[0x80u8; 74]));
        let req = b"GET /x HTTP/1.1\r\nUser-Agent: a\r\nHost: Rutracker.ORG:80\r\n\r\n";
        assert!(is_http_request(req) && !is_http_request(b"\x16\x03\x01"));
        assert_eq!(http_host(req).as_deref(), Some("rutracker.org"));
        assert_eq!(http_host(b"GET / HTTP/1.0\r\n\r\nHost: no\r\n"), None);
        assert!(host_is("c-fra08.discord.media", CF_HOST) && host_is("discord.media", CF_HOST) && !host_is("notdiscord.media", CF_HOST));
    }
}
