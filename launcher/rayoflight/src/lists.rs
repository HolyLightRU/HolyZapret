use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;

#[derive(Default, Debug)]
pub struct Hosts {
    suffix: HashSet<String>,
    exact: HashSet<String>,
}

impl Hosts {
    pub fn add(&mut self, line: &str) {
        let l = line.trim().trim_end_matches('.').to_lowercase();
        if l.is_empty() || l.starts_with('#') {
            return;
        }
        match l.strip_prefix('^') {
            Some(e) => self.exact.insert(e.to_string()),
            None => self.suffix.insert(l),
        };
    }

    pub fn add_text(&mut self, text: &str) {
        for l in text.lines() {
            self.add(l);
        }
    }

    pub fn load(files: &[&Path]) -> Hosts {
        let mut h = Hosts::default();
        for f in files {
            h.add_text(&std::fs::read_to_string(f).unwrap_or_default());
        }
        h
    }

    pub fn len(&self) -> usize {
        self.suffix.len() + self.exact.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn matches(&self, host: &str) -> bool {
        if self.exact.contains(host) {
            return true;
        }
        let mut h = host;
        loop {
            if self.suffix.contains(h) {
                return true;
            }
            match h.find('.') {
                Some(i) => h = &h[i + 1..],
                None => return false,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Net {
    addr: IpAddr,
    prefix: u8,
}

impl Net {
    pub fn parse(s: &str) -> Option<Net> {
        let s = s.trim();
        if s.is_empty() || s.starts_with('#') {
            return None;
        }
        let (a, p) = s.split_once('/').unwrap_or((s, ""));
        let addr: IpAddr = a.trim().parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = if p.is_empty() { max } else { p.trim().parse().ok().filter(|v| *v <= max)? };
        Some(Net { addr, prefix })
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(n), IpAddr::V4(i)) => {
                let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - self.prefix) };
                u32::from(n) & mask == u32::from(*i) & mask
            }
            (IpAddr::V6(n), IpAddr::V6(i)) => {
                let mask = if self.prefix == 0 { 0 } else { u128::MAX << (128 - self.prefix) };
                u128::from(n) & mask == u128::from(*i) & mask
            }
            _ => false,
        }
    }
}

#[derive(Default, Debug)]
pub struct Nets(Vec<Net>);

impl Nets {
    pub fn parse_text(text: &str) -> Nets {
        Nets(text.lines().filter_map(Net::parse).collect())
    }

    pub fn load(files: &[&Path]) -> Nets {
        let mut v = Vec::new();
        for f in files {
            v.extend(Nets::parse_text(&std::fs::read_to_string(f).unwrap_or_default()).0);
        }
        Nets(v)
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        self.0.iter().any(|n| n.contains(ip))
    }

    pub fn as_slice(&self) -> &[Net] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_nets() {
        let mut h = Hosts::default();
        h.add_text("# c\nyoutube.com\n^dns.google\nDiscord.GG.\n");
        assert!(h.matches("www.youtube.com") && h.matches("youtube.com") && h.matches("discord.gg"));
        assert!(h.matches("dns.google") && !h.matches("x.dns.google") && !h.matches("google"));
        assert_eq!(h.len(), 3);
        let n = Nets::parse_text("10.0.0.0/8\n# x\n2606:4700::/32\nbad\n");
        assert_eq!(n.as_slice().len(), 2);
        assert!(n.contains(&"10.1.2.3".parse().unwrap()) && !n.contains(&"11.0.0.1".parse().unwrap()));
        assert!(n.contains(&"2606:4700:1::1".parse().unwrap()));
        assert!(Net::parse("1.2.3.4").unwrap().contains(&"1.2.3.4".parse().unwrap()));
        assert!(Net::parse("1.2.3.4/33").is_none());
    }
}
