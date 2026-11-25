use rayoflight::lists::{Hosts, Net};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

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
                h.add_text(&std::fs::read_to_string(p).unwrap_or_default());
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Verdict {
    Touched,
    ByName,
    ByContent,
    NotListed,
    Excluded,
    Passed,
    NoProfile,
    Unseen,
}

pub fn verdict(args: &str, udp: bool, ip: IpAddr, port: u16, cache: &mut Cache) -> Verdict {
    let key = if udp { "--wf-udp=" } else { "--wf-tcp=" };
    if !tokens(args).iter().any(|t| t.strip_prefix(key).is_some_and(|v| ports_include(v, port))) {
        return Verdict::Unseen;
    }
    let mut seen = Vec::new();
    for p in profiles(args) {
        let port_ok = match (udp, &p.tcp, &p.udp) {
            (false, Some(s), _) | (true, _, Some(s)) => ports_include(s, port),
            (_, None, None) => true,
            _ => false,
        };
        if !port_ok {
            continue;
        }
        if let Some(l) = &p.l7 {
            let web = if udp { port == 443 && l.split(',').any(|x| x == "quic") } else { matches!(port, 80 | 443) && l.split(',').any(|x| x == "tls" || x == "http") };
            if !web {
                seen.push(Verdict::ByContent);
                continue;
            }
        }
        if p.ip_excludes.iter().any(|f| cache.nets(f).iter().any(|n| n.contains(&ip))) {
            seen.push(Verdict::Excluded);
            continue;
        }
        if (!p.ipsets.is_empty() || !p.ips.is_empty()) && !(p.ips.iter().any(|n| n.contains(&ip)) || p.ipsets.iter().any(|f| cache.nets(f).iter().any(|n| n.contains(&ip)))) {
            seen.push(Verdict::NotListed);
            continue;
        }
        if !p.hostlists.is_empty() || !p.domains.is_empty() {
            seen.push(Verdict::ByName);
            continue;
        }
        return if p.desync { Verdict::Touched } else { Verdict::Passed };
    }
    seen.into_iter().min_by_key(|v| match v {
        Verdict::ByName | Verdict::ByContent => 0,
        Verdict::Excluded => 1,
        _ => 2,
    })
    .unwrap_or(Verdict::NoProfile)
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
    fn ports() {
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

    #[test]
    fn verdict_for_app_traffic() {
        let dir = std::env::temp_dir().join(format!("hz-ver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("mine.txt");
        let excl = dir.join("excl.txt");
        let list = dir.join("list.txt");
        std::fs::write(&mine, "5.6.7.0/24\r\n").unwrap();
        std::fs::write(&excl, "9.9.9.9\r\n").unwrap();
        std::fs::write(&list, "youtube.com\r\n").unwrap();
        let (m, e, l) = (mine.display(), excl.display(), list.display());
        let args = |game: &str| {
            format!(
                "--wf-tcp=80,443,{game} --wf-udp=443,50000-50100,{game} --filter-udp=443 --hostlist=\"{l}\" --ipset-exclude=\"{e}\" --dpi-desync=fake --new --filter-udp=50000-50100 --filter-l7=discord,stun --dpi-desync=fake --new --filter-tcp=80,443 --hostlist=\"{l}\" --ipset-exclude=\"{e}\" --dpi-desync=fake --new --filter-tcp={game} --ipset=\"{m}\" --ipset-exclude=\"{e}\" --dpi-desync=fake --new --filter-udp={game} --ipset=\"{m}\" --ipset-exclude=\"{e}\" --dpi-desync=fake"
            )
        };
        let (on, off) = (args("1024-65535"), args("12"));
        let mut c = Cache::default();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(verdict(&on, true, ip("5.6.7.8"), 27015, &mut c), Verdict::Touched);
        assert_eq!(verdict(&off, true, ip("5.6.7.8"), 27015, &mut c), Verdict::Unseen);
        assert_eq!(verdict(&on, true, ip("1.2.3.4"), 27015, &mut c), Verdict::NotListed);
        assert_eq!(verdict(&on, true, ip("9.9.9.9"), 27015, &mut c), Verdict::Excluded);
        assert_eq!(verdict(&on, false, ip("1.2.3.4"), 443, &mut c), Verdict::ByName);
        assert_eq!(verdict(&off, true, ip("1.2.3.4"), 50010, &mut c), Verdict::ByContent);
        assert_eq!(verdict("--wf-udp=443,3000 --filter-udp=443 --dpi-desync=fake", true, ip("1.2.3.4"), 3000, &mut c), Verdict::NoProfile);
        assert_eq!(verdict(&off, false, ip("1.2.3.4"), 22, &mut c), Verdict::Unseen);
        assert_eq!(verdict("--wf-tcp=443 --filter-tcp=443 --hostlist-domains=a.com --new --filter-tcp=443 --dpi-desync=fake", false, ip("1.1.1.1"), 443, &mut c), Verdict::Touched);
        assert_eq!(verdict("--wf-tcp=443 --filter-tcp=443", false, ip("1.1.1.1"), 443, &mut c), Verdict::Passed);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
