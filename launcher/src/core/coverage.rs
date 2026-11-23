use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
struct Hosts {
    suffix: HashSet<String>,
    exact: HashSet<String>,
}

impl Hosts {
    fn add(&mut self, line: &str) {
        let l = line.trim().trim_end_matches('.').to_lowercase();
        if l.is_empty() || l.starts_with('#') {
            return;
        }
        match l.strip_prefix('^') {
            Some(e) => self.exact.insert(e.to_string()),
            None => self.suffix.insert(l),
        };
    }

    fn matches(&self, host: &str) -> bool {
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

#[derive(Clone, Copy)]
struct Net {
    addr: IpAddr,
    prefix: u8,
}

impl Net {
    fn parse(s: &str) -> Option<Net> {
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

    fn contains(&self, ip: &IpAddr) -> bool {
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

#[derive(Default)]
pub struct Cache {
    hosts: HashMap<PathBuf, Arc<Hosts>>,
    nets: HashMap<PathBuf, Arc<Vec<Net>>>,
}

impl Cache {
    fn hosts(&mut self, p: &PathBuf) -> Arc<Hosts> {
        self.hosts
            .entry(p.clone())
            .or_insert_with(|| {
                let mut h = Hosts::default();
                for l in std::fs::read_to_string(p).unwrap_or_default().lines() {
                    h.add(l);
                }
                Arc::new(h)
            })
            .clone()
    }

    fn nets(&mut self, p: &PathBuf) -> Arc<Vec<Net>> {
        self.nets
            .entry(p.clone())
            .or_insert_with(|| Arc::new(std::fs::read_to_string(p).unwrap_or_default().lines().filter_map(Net::parse).collect()))
            .clone()
    }
}

fn tokens(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in args.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn ports_include(spec: &str, port: u16) -> bool {
    spec.split(',').any(|p| match p.split_once('-') {
        Some((a, b)) => a.trim().parse::<u16>().is_ok_and(|a| a <= port) && b.trim().parse::<u16>().is_ok_and(|b| port <= b),
        None => p.trim().parse::<u16>().is_ok_and(|v| v == port),
    })
}

#[derive(Default)]
struct Profile {
    tcp: Option<String>,
    udp: Option<String>,
    rest: Vec<String>,
    l7: Option<String>,
    desync: bool,
    hostlists: Vec<PathBuf>,
    domains: Vec<String>,
    host_excludes: Vec<PathBuf>,
    ipsets: Vec<PathBuf>,
    ips: Vec<Net>,
    ip_excludes: Vec<PathBuf>,
}

fn profiles(args: &str) -> Vec<Profile> {
    let mut out = vec![Profile::default()];
    for t in tokens(args) {
        if t == "--new" {
            out.push(Profile::default());
            continue;
        }
        let p = out.last_mut().unwrap();
        let (k, v) = t.split_once('=').unwrap_or((t.as_str(), ""));
        match k {
            "--filter-tcp" => p.tcp = Some(v.to_string()),
            "--filter-udp" => p.udp = Some(v.to_string()),
            "--filter-l7" => p.l7 = Some(v.to_lowercase()),
            "--dpi-desync" => {
                p.desync = true;
                p.rest.push(t.clone());
            }
            "--hostlist" => p.hostlists.push(PathBuf::from(v)),
            "--hostlist-domains" => p.domains.extend(v.split(',').map(|d| d.trim().to_lowercase())),
            "--hostlist-exclude" => p.host_excludes.push(PathBuf::from(v)),
            "--ipset" => p.ipsets.push(PathBuf::from(v)),
            "--ipset-ip" => p.ips.extend(v.split(',').filter_map(Net::parse)),
            "--ipset-exclude" => p.ip_excludes.push(PathBuf::from(v)),
            _ if k.starts_with("--filter-") || k.starts_with("--hostlist") || k.starts_with("--ipset") || k.starts_with("--wf-") => {}
            "--comment" | "--port" | "--no-gui" | "--debug" => {}
            _ => p.rest.push(t.clone()),
        }
    }
    out
}

fn first(list: Vec<Profile>, host: &str, ips: &[IpAddr], udp: bool, cache: &mut Cache) -> Option<Profile> {
    let host = host.to_lowercase();
    for p in list {
        let port_ok = if udp {
            p.udp.as_ref().is_some_and(|spec| ports_include(spec, 443)) && p.tcp.is_none()
        } else {
            match &p.tcp {
                Some(spec) => ports_include(spec, 443),
                None => p.udp.is_none(),
            }
        };
        let l7_ok = p.l7.as_ref().is_none_or(|l| l.split(',').any(|x| if udp { x == "quic" } else { x == "tls" || x == "http" }));
        if !port_ok || !l7_ok {
            continue;
        }
        let has_hosts = !p.hostlists.is_empty() || !p.domains.is_empty();
        if has_hosts {
            let listed = p.domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
                || p.hostlists.iter().any(|f| cache.hosts(f).matches(&host));
            if !listed {
                continue;
            }
        }
        let has_ips = !p.ipsets.is_empty() || !p.ips.is_empty();
        if has_ips {
            let in_set = ips.iter().any(|ip| p.ips.iter().any(|n| n.contains(ip)) || p.ipsets.iter().any(|f| cache.nets(f).iter().any(|n| n.contains(ip))));
            if !in_set {
                continue;
            }
        }
        if p.host_excludes.iter().any(|f| cache.hosts(f).matches(&host)) {
            continue;
        }
        if p.ip_excludes.iter().any(|f| ips.iter().any(|ip| cache.nets(f).iter().any(|n| n.contains(ip)))) {
            continue;
        }
        return Some(p);
    }
    None
}

pub fn touches(args: &str, host: &str, ips: &[IpAddr], cache: &mut Cache) -> bool {
    first(profiles(args), host, ips, false, cache).is_some_and(|p| p.desync)
}

pub fn borrow(args: &str, host: &str, ips: &[IpAddr], cache: &mut Cache) -> [Option<Vec<String>>; 2] {
    let pick = |udp: bool, cache: &mut Cache| first(profiles(args), host, ips, udp, cache).filter(|p| p.desync).map(|p| p.rest);
    [pick(false, cache), pick(true, cache)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrow_takes_first_matching_profile() {
        let args = "--wf-tcp=443 --wf-udp=443 --filter-udp=443 --hostlist-domains=a.com --dpi-desync=fake --dpi-desync-repeats=11 --new --filter-tcp=443 --hostlist-domains=a.com --ipset-exclude=\"x\" --dpi-desync=multisplit --dpi-desync-split-pos=1 --comment=test --new --filter-tcp=443 --dpi-desync=fake";
        let mut c = Cache::default();
        let v = |s: &[&str]| Some(s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(borrow(args, "www.a.com", &[], &mut c), [v(&["--dpi-desync=multisplit", "--dpi-desync-split-pos=1"]), v(&["--dpi-desync=fake", "--dpi-desync-repeats=11"])]);
        assert_eq!(borrow(args, "b.com", &[], &mut c), [v(&["--dpi-desync=fake"]), None]);
        assert_eq!(borrow("--filter-tcp=443 --hostlist-domains=a.com", "a.com", &[], &mut c), [None, None]);
    }

    #[test]
    fn hosts_and_nets() {
        let mut h = Hosts::default();
        for l in ["# c", "youtube.com", "^dns.google", "Discord.GG."] {
            h.add(l);
        }
        assert!(h.matches("www.youtube.com") && h.matches("youtube.com") && h.matches("discord.gg"));
        assert!(h.matches("dns.google") && !h.matches("x.dns.google") && !h.matches("google"));
        let n = Net::parse("10.0.0.0/8").unwrap();
        assert!(n.contains(&"10.1.2.3".parse().unwrap()) && !n.contains(&"11.0.0.1".parse().unwrap()));
        assert!(Net::parse("2606:4700::/32").unwrap().contains(&"2606:4700:1::1".parse().unwrap()));
        assert!(Net::parse("1.2.3.4").unwrap().contains(&"1.2.3.4".parse().unwrap()));
        assert!(ports_include("80,443,2053-2096", 443) && ports_include("1024-65535", 2000) && !ports_include("80,8443", 443));
    }

    #[test]
    fn first_matching_profile_decides() {
        let dir = std::env::temp_dir().join(format!("hz-cov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let list = dir.join("list.txt");
        let ipset = dir.join("ipset.txt");
        let excl = dir.join("excl.txt");
        std::fs::write(&list, "youtube.com\r\n").unwrap();
        std::fs::write(&ipset, "104.16.0.0/12\r\n").unwrap();
        std::fs::write(&excl, "rutracker.org\r\n").unwrap();
        let l = list.display().to_string();
        let s = ipset.display().to_string();
        let e = excl.display().to_string();
        let general = format!(
            "--wf-tcp=80,443 --filter-udp=443 --hostlist=\"{l}\" --dpi-desync=fake --new --filter-tcp=80,443 --hostlist=\"{l}\" --dpi-desync=fake,multisplit --new --filter-tcp=80,443 --ipset=\"{s}\" --hostlist-exclude=\"{e}\" --dpi-desync=fake"
        );
        let mut c = Cache::default();
        let ip = |s: &str| vec![s.parse::<IpAddr>().unwrap()];
        assert!(touches(&general, "www.youtube.com", &ip("8.8.8.8"), &mut c));
        assert!(!touches(&general, "genderize.io", &ip("8.8.8.8"), &mut c));
        assert!(touches(&general, "site.com", &ip("104.17.1.1"), &mut c));
        assert!(!touches(&general, "rutracker.org", &ip("104.17.1.1"), &mut c));
        let ruled = format!("--wf-tcp=80,443 --filter-tcp=80,443 --hostlist-domains=youtube.com --new {general}");
        assert!(!touches(&ruled, "youtube.com", &ip("8.8.8.8"), &mut c));
        assert!(touches("--wf-tcp=443 --filter-tcp=443 --dpi-desync=fake", "any.org", &ip("1.1.1.1"), &mut c));
        assert!(!touches("--filter-tcp=443 --filter-l7=stun --dpi-desync=fake", "any.org", &ip("1.1.1.1"), &mut c));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
