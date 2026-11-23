use std::path::PathBuf;

pub const CATEGORY: &str = "Голос Discord";
const MAX: usize = 4;
const FALLBACK: [(&str, u16); 3] = [("finland14129.discord.media", 443), ("c-ams15-0ef6c291.discord.media", 2096), ("c-ams08-2e46f389.discord.media", 2096)];

pub fn parse_log(text: &str) -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = Vec::new();
    for line in text.lines().rev() {
        let Some(i) = line.find("wss://") else { continue };
        let rest = &line[i + 6..];
        let end = rest.find(['/', ',', ' ', '"']).unwrap_or(rest.len());
        let (host, port) = match rest[..end].split_once(':') {
            Some((h, p)) => (h, p.parse().unwrap_or(443)),
            None => (&rest[..end], 443),
        };
        let host = host.to_lowercase();
        if host.ends_with(".discord.media") && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') && !out.iter().any(|(h, _)| *h == host) {
            out.push((host, port));
        }
        if out.len() >= MAX {
            break;
        }
    }
    out
}

fn logs() -> Vec<PathBuf> {
    let appdata = PathBuf::from(std::env::var("APPDATA").unwrap_or_default());
    ["discord", "discordptb", "discordcanary"].iter().map(|d| appdata.join(d).join("logs").join("renderer_js.log")).filter(|p| p.is_file()).collect()
}

pub fn hosts() -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = Vec::new();
    for p in logs() {
        let bytes = std::fs::read(&p).unwrap_or_default();
        let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4 * 1024 * 1024)..]);
        for h in parse_log(&tail) {
            if !out.iter().any(|(x, _)| *x == h.0) {
                out.push(h);
            }
        }
    }
    for (h, p) in FALLBACK {
        if out.len() >= MAX {
            break;
        }
        if !out.iter().any(|(x, _)| x == h) {
            out.push((h.to_string(), p));
        }
    }
    out.truncate(MAX);
    out
}

pub fn label(host: &str, port: u16) -> String {
    let name = host.trim_end_matches(".discord.media");
    let region = match name.strip_prefix("c-") {
        Some(r) => r.split('-').next().unwrap_or(r).to_string(),
        None => name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string(),
    };
    format!("{region}:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_hosts_from_log() {
        let log = "\
[info] Connecting to RTC server wss://finland14129.discord.media:443/, rtc-connection-id: x\n\
[info] [CONNECT] wss://c-ams15-0ef6c291.discord.media:2096/\n\
[info] gateway wss://gateway.discord.gg/?v=9\n\
[info] Connecting to RTC server wss://c-ams15-0ef6c291.discord.media:2096/, x\n\
[info] [CONNECT] wss://c-fra08-11aa22bb.discord.media:8443/\n";
        assert_eq!(
            parse_log(log),
            vec![
                ("c-fra08-11aa22bb.discord.media".to_string(), 8443),
                ("c-ams15-0ef6c291.discord.media".to_string(), 2096),
                ("finland14129.discord.media".to_string(), 443),
            ]
        );
        assert_eq!(label("finland14129.discord.media", 443), "finland:443");
        assert_eq!(label("c-ams15-0ef6c291.discord.media", 2096), "ams15:2096");
    }

    #[test]
    #[ignore]
    fn hosts_from_this_machine() {
        let h = hosts();
        println!("{h:?}");
        assert!(!h.is_empty() && h.len() <= MAX);
    }
}
