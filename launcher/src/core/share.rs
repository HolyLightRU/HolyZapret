use super::gamefilter::{self, GameFilter};
use super::lists::{self, Kind, USER_FILES};
use super::paths::Paths;
use super::rules::{self, Rule};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use serde_json::{Value, json};
use std::io::{Read, Write};

pub const PREFIX: &str = "HZ1:";
const KEYS: [&str; 4] = ["d", "x", "i", "e"];
const MAX_ITEMS: usize = 5000;
const MAX_JSON: u64 = 2 << 20;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Share {
    pub strategy: Option<String>,
    pub game: Option<GameFilter>,
    pub lists: [Vec<String>; 4],
    pub rules: Vec<Rule>,
}

impl Share {
    pub fn collect(paths: &Paths, strategy: Option<String>, game: &GameFilter) -> Share {
        let dirs = super::reset::list_dirs(paths);
        let mut out: [Vec<String>; 4] = Default::default();
        for (i, (file, ..)) in USER_FILES.iter().enumerate() {
            for d in &dirs {
                let doc = lists::Doc::load(&d.join(file));
                for n in 0..doc.len() {
                    let v = doc.entry(n);
                    if !out[i].iter().any(|x| x.eq_ignore_ascii_case(v)) {
                        out[i].push(v.to_string());
                    }
                }
            }
        }
        Share { strategy, game: game.enabled().then(|| game.clone()), lists: out, rules: rules::load(paths) }
    }

    pub fn is_empty(&self) -> bool {
        self.strategy.is_none() && self.game.is_none() && self.rules.is_empty() && self.lists.iter().all(Vec::is_empty)
    }

    pub fn list_count(&self) -> usize {
        self.lists.iter().map(Vec::len).sum()
    }

    pub fn encode(&self) -> String {
        let mut j = json!({ "v": 1 });
        if let Some(s) = &self.strategy {
            j["s"] = json!(s);
        }
        if let Some(g) = &self.game {
            j["g"] = json!(g.to_text());
        }
        for (k, l) in KEYS.iter().zip(&self.lists) {
            if !l.is_empty() {
                j[*k] = json!(l);
            }
        }
        if !self.rules.is_empty() {
            let r: Vec<String> = self.rules.iter().map(Rule::line).collect();
            j["r"] = json!(r);
        }
        let mut z = DeflateEncoder::new(Vec::new(), Compression::best());
        let _ = z.write_all(j.to_string().as_bytes());
        let bytes = z.finish().unwrap_or_default();
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
    }
}

pub fn decode(text: &str) -> Result<Share, String> {
    let start = text.find(PREFIX).ok_or("Это не код HolyZapret: такой код начинается с HZ1:")?;
    let mut body = String::new();
    for c in text[start + PREFIX.len()..].chars() {
        if c.is_whitespace() {
            continue;
        }
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            break;
        }
        body.push(c);
    }
    let broken = "Код повреждён: скопируйте его целиком, от HZ1: до конца";
    let raw = URL_SAFE_NO_PAD.decode(body.as_bytes()).map_err(|_| broken)?;
    let mut data = Vec::new();
    DeflateDecoder::new(raw.as_slice()).take(MAX_JSON).read_to_end(&mut data).map_err(|_| broken)?;
    let v: Value = serde_json::from_slice(&data).map_err(|_| broken)?;
    match v["v"].as_u64() {
        Some(1) => {}
        Some(_) => return Err("Код сделан в более новой версии HolyZapret: обновите лаунчер".into()),
        None => return Err(broken.into()),
    }
    let strs = |k: &str| -> Vec<String> {
        v[k].as_array().map(|a| a.iter().filter_map(Value::as_str).take(MAX_ITEMS).map(String::from).collect()).unwrap_or_default()
    };
    let mut out: [Vec<String>; 4] = Default::default();
    for (i, k) in KEYS.iter().enumerate() {
        for raw in strs(k) {
            let ok = match USER_FILES[i].2 {
                Kind::Domains => lists::normalize_domain(&raw),
                Kind::Ips => lists::normalize_ip(&raw),
            };
            if let Ok(x) = ok {
                if !out[i].iter().any(|y| y.eq_ignore_ascii_case(&x)) {
                    out[i].push(x);
                }
            }
        }
    }
    let strategy = v["s"].as_str().map(str::trim).filter(|s| !s.is_empty() && s.len() <= 200 && !s.contains("..")).map(String::from);
    let game = v["g"].as_str().map(gamefilter::parse).filter(GameFilter::enabled);
    let rules = rules::parse(&strs("r").join("\n"));
    let share = Share { strategy, game, lists: out, rules };
    if share.is_empty() {
        return Err("В коде нет ни варианта RayOfLight, ни списков, ни правил".into());
    }
    Ok(share)
}

pub fn apply_lists(paths: &Paths, share: &Share) -> anyhow::Result<usize> {
    let mut added = 0;
    for d in super::reset::list_dirs(paths) {
        let mut here = 0;
        for (i, (file, _, kind)) in USER_FILES.iter().enumerate() {
            if share.lists[i].is_empty() {
                continue;
            }
            let p = d.join(file);
            let mut doc = lists::Doc::load(&p);
            let n = doc.add(&share.lists[i]);
            if n > 0 {
                doc.save(&p, *kind)?;
            }
            here += n;
        }
        added = added.max(here);
    }
    Ok(added)
}

pub fn apply_rules(paths: &Paths, share: &Share) -> anyhow::Result<usize> {
    let mut list = rules::load(paths);
    let before = list.clone();
    for r in &share.rules {
        rules::upsert(&mut list, r.clone());
    }
    if list != before {
        rules::save(paths, &list)?;
    }
    Ok(share.rules.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Share {
        Share {
            strategy: Some("experimental\\R06 general, наложение у SNI".into()),
            game: Some(GameFilter::all()),
            lists: [vec!["rutracker.org".into(), "kinozal.tv".into()], vec![], vec!["104.16.0.0/13".into()], vec![]],
            rules: vec![
                Rule { technique: "general".into(), domains: vec!["example.com".into(), "cdn.example.com".into()], origin: "general".into(), ..Default::default() },
                Rule { technique: "sni-split".into(), domains: vec!["soundcloud.com".into()], origin: "general".into(), ..Default::default() },
            ],
        }
    }

    #[test]
    fn round_trip_and_noise() {
        let s = sample();
        let code = s.encode();
        assert!(code.starts_with(PREFIX));
        assert_eq!(decode(&code).unwrap(), s);
        let (a, b) = code.split_at(30);
        let message = format!("Привет, вот мой код:\n{a}\n{b}\nвставь в лаунчер");
        assert_eq!(decode(&message).unwrap(), s);
    }

    #[test]
    fn bad_codes_rejected() {
        assert!(decode("привет").is_err());
        assert!(decode("HZ1:@@@").is_err());
        let code = sample().encode();
        assert!(decode(&code[..code.len() / 2]).is_err());
        let mut z = DeflateEncoder::new(Vec::new(), Compression::best());
        z.write_all(br#"{"v":1,"d":["not a domain!","../x","ok.example.org"],"i":["999.1.1.1"],"s":"..\\..\\evil","r":["unknown|a.com|"]}"#).unwrap();
        let s = decode(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(z.finish().unwrap()))).unwrap();
        assert_eq!(s.lists[0], vec!["ok.example.org".to_string()]);
        assert!(s.lists[2].is_empty() && s.strategy.is_none() && s.rules.is_empty());
        let mut z = DeflateEncoder::new(Vec::new(), Compression::best());
        z.write_all(br#"{"v":2}"#).unwrap();
        assert!(decode(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(z.finish().unwrap()))).unwrap_err().contains("новой версии"));
    }
}
