use super::net::{self, RAW_BASE};
use super::paths::Paths;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

fn hosts_file() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    PathBuf::from(root).join(r"System32\drivers\etc\hosts")
}

fn parse_pair(line: &str) -> Option<(&str, &str)> {
    let mut it = line.split_whitespace();
    Some((it.next()?, it.next()?))
}

pub fn download_list(paths: &Paths) -> anyhow::Result<()> {
    let text = net::get_text(&format!("{RAW_BASE}lists/hosts-list.txt"))?;
    fs::create_dir_all(paths.lists())?;
    fs::write(paths.lists().join("hosts-list.txt"), text)?;
    Ok(())
}

pub fn apply(paths: &Paths) -> anyhow::Result<Vec<String>> {
    let list = fs::read_to_string(paths.lists().join("hosts-list.txt"))?;
    let hosts = hosts_file();
    let mut log = Vec::new();

    let old_text = fs::read_to_string(&hosts).unwrap_or_default();
    if hosts.exists() {
        fs::copy(&hosts, hosts.with_extension("bak"))?;
    }

    let mut order: Vec<String> = Vec::new();
    let mut mappings: HashMap<String, (String, String)> = HashMap::new();
    for line in list.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some((ip, domain)) = parse_pair(line) {
            let key = domain.to_lowercase();
            if !mappings.contains_key(&key) {
                order.push(key.clone());
            }
            mappings.insert(key, (ip.to_string(), domain.to_string()));
        }
    }

    let mut updated: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut old_domains: HashSet<String> = HashSet::new();
    for line in old_text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            updated.push(line.to_string());
            continue;
        }
        match parse_pair(line) {
            Some((_, domain)) => {
                let key = domain.to_lowercase();
                old_domains.insert(key.clone());
                if mappings.contains_key(&key) || !seen.insert(key) {
                    continue;
                }
                updated.push(line.to_string());
            }
            None => updated.push(line.to_string()),
        }
    }

    for key in order {
        let (ip, domain) = &mappings[&key];
        if old_domains.contains(&key) {
            log.push(format!("Обновлено: {domain} → {ip}"));
        } else {
            log.push(format!("Добавлено: {ip} {domain}"));
        }
        updated.push(format!("{ip} {domain}"));
    }

    let mut out = updated.join("\r\n");
    out.push_str("\r\n");
    fs::write(&hosts, out)?;
    Ok(log)
}
