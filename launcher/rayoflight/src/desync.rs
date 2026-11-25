use crate::packet::{self, Opts, Tcp, Timestamps};
use crate::tls;

pub const FAKE_DEFAULT: &[u8] = include_bytes!("../assets/fake_tls_default.bin");
pub const SNI_POOL: [&str; 8] = ["www.google.com", "www.microsoft.com", "www.bing.com", "yandex.ru", "vk.com", "mail.ru", "www.apple.com", "ok.ru"];
pub const BADSEQ_INCREMENT: i32 = -10000;
pub const BADSEQ_ACK_INCREMENT: i32 = -66000;
const MAX_PART: usize = 1400;
const MIN_SEQOVL: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fooling {
    Ts,
    Badseq,
    Md5sig,
    Ttl,
    Badsum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    Pos(usize),
    SniMiddle,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Technique {
    pub name: &'static str,
    pub label: &'static str,
    pub fooling: Fooling,
    pub repeats: (usize, usize),
    pub random_sni: bool,
    pub split: Split,
    pub disorder: bool,
}

pub const GENERAL: Technique =
    Technique { name: "general", label: "как general", fooling: Fooling::Ts, repeats: (8, 8), random_sni: false, split: Split::Pos(1), disorder: false };
pub const SNI_SPLIT: Technique = Technique { name: "sni-split", label: "разрез по имени сайта", split: Split::SniMiddle, ..GENERAL };
pub const RANDOM: Technique = Technique { name: "random", label: "случайный рисунок", repeats: (6, 10), random_sni: true, split: Split::Random, ..GENERAL };
pub const BADSEQ: Technique = Technique { name: "badseq", label: "фейк с неверным seq", fooling: Fooling::Badseq, ..GENERAL };
pub const MD5SIG: Technique = Technique { name: "md5sig", label: "фейк с подписью md5", fooling: Fooling::Md5sig, ..GENERAL };
pub const TTL: Technique = Technique { name: "ttl", label: "фейк с коротким TTL", fooling: Fooling::Ttl, ..GENERAL };
pub const BADSUM: Technique = Technique { name: "badsum", label: "фейк с неверной суммой", fooling: Fooling::Badsum, ..GENERAL };
pub const DISORDER: Technique = Technique { name: "disorder", label: "части в обратном порядке", disorder: true, ..GENERAL };
pub const NONE: Technique = Technique { name: "none", label: "без обхода", ..GENERAL };
pub const TECHNIQUES: [Technique; 8] = [GENERAL, SNI_SPLIT, RANDOM, BADSEQ, MD5SIG, TTL, BADSUM, DISORDER];
pub const MIN_FAKE_TTL: u8 = 3;
pub const MAX_FAKE_TTL: u8 = 20;

pub fn fake_ttl(hops: Option<u8>) -> Option<u8> {
    hops.filter(|h| *h > MIN_FAKE_TTL).map(|h| (h - 1).min(MAX_FAKE_TTL))
}

pub fn technique(name: &str) -> Option<Technique> {
    TECHNIQUES.iter().copied().find(|t| t.name.eq_ignore_ascii_case(name))
}

#[derive(Clone, Debug)]
pub struct Config {
    pub fake: Vec<u8>,
    pub pool: Vec<Vec<u8>>,
    pub ts_increment: i32,
    pub seqovl_pattern: Vec<u8>,
    pub seqovl: usize,
    pub fake_http: Vec<u8>,
    pub fake_voice: Vec<u8>,
    pub fake_game_udp: Vec<u8>,
}

impl Config {
    pub fn general(seqovl_pattern: Vec<u8>) -> Option<Config> {
        if seqovl_pattern.is_empty() {
            return None;
        }
        Some(Config {
            fake: tls::with_sni(FAKE_DEFAULT, "www.google.com")?,
            pool: SNI_POOL.iter().filter_map(|s| tls::with_sni(FAKE_DEFAULT, s)).collect(),
            ts_increment: -600000,
            seqovl_pattern,
            seqovl: 681,
            fake_http: Vec::new(),
            fake_voice: Vec::new(),
            fake_game_udp: Vec::new(),
        })
    }
}

pub fn pick(n: usize, rnd: &mut dyn FnMut(&mut [u8])) -> usize {
    if n <= 1 {
        return 0;
    }
    let mut b = [0u8; 2];
    rnd(&mut b);
    u16::from_be_bytes(b) as usize % n
}

fn base_fake<'a>(cfg: &'a Config, tech: &Technique, rnd: &mut dyn FnMut(&mut [u8])) -> &'a [u8] {
    if tech.random_sni && !cfg.pool.is_empty() { &cfg.pool[pick(cfg.pool.len(), rnd)] } else { &cfg.fake }
}

pub fn fake_from(base: &[u8], hello: &[u8], rnd: &mut dyn FnMut(&mut [u8])) -> Vec<u8> {
    let mut f = base.to_vec();
    rnd(&mut f[11..43]);
    let sid = f[43] as usize;
    rnd(&mut f[44..44 + sid]);
    if let Some(real) = tls::session_id(hello).filter(|s| s.len() == sid) {
        f[44..44 + sid].copy_from_slice(real);
    }
    f
}

pub fn fake_for(cfg: &Config, hello: &[u8], rnd: &mut dyn FnMut(&mut [u8])) -> Vec<u8> {
    fake_from(&cfg.fake, hello, rnd)
}

fn pattern(p: &[u8], n: usize) -> Vec<u8> {
    p.iter().copied().cycle().take(n).collect()
}

pub fn first_ip_id(orig: &Tcp, rnd: &mut dyn FnMut(&mut [u8])) -> u16 {
    if orig.v6 || orig.ip_id != 0 {
        return orig.ip_id;
    }
    let mut b = [0u8; 2];
    rnd(&mut b);
    u16::from_be_bytes(b).max(1)
}

pub fn next_ip_id(id: u16) -> u16 {
    match id.wrapping_add(1) {
        0 => 1,
        v => v,
    }
}

pub fn split_pos(tech: &Technique, hello: &[u8], data_len: usize, rnd: &mut dyn FnMut(&mut [u8])) -> usize {
    let limit = data_len.saturating_sub(1);
    if limit == 0 {
        return 0;
    }
    let pos = match tech.split {
        Split::Pos(p) => p,
        Split::SniMiddle => tls::sni_range(hello).map(|(at, len)| at + len.max(2) / 2).unwrap_or(1),
        Split::Random => {
            let top = tls::sni_range(hello).map(|(at, len)| at + len).unwrap_or(limit).min(limit).max(1);
            1 + pick(top, rnd)
        }
    };
    if pos == 0 || pos > limit { 1 } else { pos }
}

pub struct Out {
    pub packets: Vec<Vec<u8>>,
    pub next_ip_id: u16,
    pub split_pos: usize,
    pub fallback: Option<&'static str>,
}

fn fooling_for(tech: &Technique, has_ts: bool, ttl: Option<u8>) -> (Fooling, Option<&'static str>) {
    match tech.fooling {
        Fooling::Ts if !has_ts && ttl.is_some() => (Fooling::Ttl, Some("ttl")),
        Fooling::Ts if !has_ts => (Fooling::Badseq, Some("badseq")),
        Fooling::Ttl if ttl.is_none() && has_ts => (Fooling::Ts, Some("ts")),
        Fooling::Ttl if ttl.is_none() => (Fooling::Badseq, Some("badseq")),
        f => (f, None),
    }
}

pub fn first_segment(orig: &Tcp, hello: &[u8], cfg: &Config, tech: &Technique, hops: Option<u8>, rnd: &mut dyn FnMut(&mut [u8])) -> Option<Out> {
    first_segment_with(orig, hello, cfg, tech, hops, None, rnd)
}

pub fn first_segment_with(
    orig: &Tcp,
    hello: &[u8],
    cfg: &Config,
    tech: &Technique,
    hops: Option<u8>,
    fake_blob: Option<&[u8]>,
    rnd: &mut dyn FnMut(&mut [u8]),
) -> Option<Out> {
    let data = orig.payload();
    if data.len() <= 1 {
        return None;
    }
    let pos = split_pos(tech, hello, data.len(), rnd);
    let mut ip_id = first_ip_id(orig, rnd);
    let (lo, hi) = tech.repeats;
    let repeats = lo + pick(hi.saturating_sub(lo) + 1, rnd);
    let mut packets = Vec::with_capacity(repeats + 2);
    let ttl = fake_ttl(hops);
    let (fooling, fallback) = fooling_for(tech, orig.ts.is_some(), ttl);
    let fooled_ts = orig.ts.map(|ts| Timestamps { val: ts.val.wrapping_add(cfg.ts_increment as u32), ecr: ts.ecr });
    let plain = Opts { md5: None, ts: orig.ts };
    let (seq, ack, opts) = match fooling {
        Fooling::Ts => (orig.seq, orig.ack, Opts { md5: None, ts: fooled_ts }),
        Fooling::Ttl => (orig.seq, orig.ack, Opts { md5: None, ts: fooled_ts }),
        Fooling::Badseq => (orig.seq.wrapping_add(BADSEQ_INCREMENT as u32), orig.ack.wrapping_add(BADSEQ_ACK_INCREMENT as u32), plain),
        Fooling::Md5sig => {
            let mut m = [0u8; 16];
            rnd(&mut m);
            (orig.seq, orig.ack, Opts { md5: Some(m), ts: orig.ts })
        }
        Fooling::Badsum => (orig.seq, orig.ack, plain),
    };
    let fake = match fake_blob {
        Some(f) => f.to_vec(),
        None => {
            let base = base_fake(cfg, tech, rnd).to_vec();
            fake_from(&base, hello, rnd)
        }
    };
    let mut pkt = packet::segment_ex(orig, seq, ack, &opts, ip_id, &fake);
    if fooling == Fooling::Ttl {
        if let Some(t) = ttl {
            packet::set_ttl(&mut pkt, t);
        }
    }
    if fooling == Fooling::Badsum {
        packet::break_tcp_sum(&mut pkt);
    }
    for _ in 0..repeats {
        packets.push(pkt.clone());
    }
    if !orig.v6 {
        ip_id = next_ip_id(ip_id);
    }
    let room = MAX_PART.saturating_sub(pos);
    let seqovl = if room >= MIN_SEQOVL { cfg.seqovl.min(room) } else { 0 };
    let mut part1 = pattern(&cfg.seqovl_pattern, seqovl);
    part1.extend_from_slice(&data[..pos]);
    let first = (orig.seq.wrapping_sub(seqovl as u32), part1);
    let second = (orig.seq.wrapping_add(pos as u32), data[pos..].to_vec());
    let parts = if tech.disorder { [second, first] } else { [first, second] };
    for (s, d) in parts {
        packets.push(packet::segment(orig, s, orig.ts, ip_id, &d));
        if !orig.v6 {
            ip_id = next_ip_id(ip_id);
        }
    }
    Some(Out { packets, next_ip_id: ip_id, split_pos: pos, fallback })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::tests::sample_v4;

    fn google() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/tls_clienthello_www_google_com.bin")).unwrap()
    }

    fn zeros(b: &mut [u8]) {
        b.fill(0xAB);
    }

    #[test]
    fn general_config_matches_winws() {
        let c = Config::general(google()).unwrap();
        assert_eq!((c.fake.len(), GENERAL.repeats, c.ts_increment, c.seqovl, GENERAL.split), (677, (8, 8), -600000, 681, Split::Pos(1)));
        assert_eq!(tls::sni(&c.fake).as_deref(), Some("www.google.com"));
        assert_eq!(pattern(&c.seqovl_pattern, 681), google());
        assert_eq!(pattern(b"ab", 5), b"ababa");
        assert_eq!(c.pool.len(), SNI_POOL.len());
        assert!(c.pool.iter().zip(SNI_POOL).all(|(f, s)| tls::sni(f).as_deref() == Some(s)));
        assert_eq!(technique("SNI-split"), Some(SNI_SPLIT));
        assert_eq!(technique("nope"), None);
    }

    #[test]
    fn fake_gets_random_and_real_session_id() {
        let c = Config::general(google()).unwrap();
        let hello = tls::with_sni(FAKE_DEFAULT, "www.youtube.com").unwrap();
        let mut rnd = zeros;
        let f = fake_for(&c, &hello, &mut rnd);
        assert_eq!(&f[11..43], &[0xAB; 32]);
        assert_eq!(&f[44..76], tls::session_id(&hello).unwrap());
        assert_eq!(tls::sni(&f).as_deref(), Some("www.google.com"));
        let mut short = hello.clone();
        short[43] = 16;
        let f = fake_for(&c, &short, &mut rnd);
        assert_eq!(&f[44..76], &[0xAB; 32]);
    }

    #[test]
    fn general_order_and_fields() {
        let c = Config::general(google()).unwrap();
        let hello = tls::with_sni(FAKE_DEFAULT, "www.youtube.com").unwrap();
        let raw = sample_v4(&hello, true);
        let o = packet::parse(&raw).unwrap();
        let mut rnd = zeros;
        let out = first_segment(&o, &hello, &c, &GENERAL, None, &mut rnd).unwrap();
        assert_eq!(out.fallback, None);
        assert_eq!(out.packets.len(), 10);
        let ps: Vec<Tcp> = out.packets.iter().map(|p| packet::parse(p).unwrap()).collect();
        for f in &ps[..8] {
            assert_eq!((f.seq, f.ip_id, f.payload().len()), (1000, 0x1234, 677));
            assert_eq!(f.ts.unwrap().val, 0x1000u32.wrapping_sub(600000));
        }
        assert_eq!(out.packets[0], out.packets[7]);
        assert_eq!((ps[8].seq, ps[8].ip_id, ps[8].payload().len()), (1000u32.wrapping_sub(681), 0x1235, 682));
        assert_eq!(&ps[8].payload()[..681], &google()[..]);
        assert_eq!(ps[8].payload()[681], 0x16);
        assert_eq!((ps[9].seq, ps[9].ip_id), (1001, 0x1236));
        assert_eq!(ps[9].payload(), &hello[1..]);
        assert_eq!(ps[8].ts, o.ts);
        assert_eq!(out.next_ip_id, 0x1237);
        assert!(out.packets.iter().all(|p| packet::checksums_ok(p)));
    }

    #[test]
    fn no_timestamps_falls_back_and_other_foolings_work() {
        let c = Config::general(google()).unwrap();
        let hello = tls::with_sni(FAKE_DEFAULT, "www.youtube.com").unwrap();
        let raw = sample_v4(&hello, false);
        let o = packet::parse(&raw).unwrap();
        let mut rnd = zeros;
        let out = first_segment(&o, &hello, &c, &GENERAL, None, &mut rnd).unwrap();
        assert_eq!((out.fallback, out.packets.len()), (Some("badseq"), 10));
        let f = packet::parse(&out.packets[0]).unwrap();
        assert_eq!((f.seq, f.ts), (1000u32.wrapping_sub(10000), None));
        let near = first_segment(&o, &hello, &c, &GENERAL, Some(12), &mut rnd).unwrap();
        assert_eq!(near.fallback, Some("ttl"));
        let f = packet::parse(&near.packets[0]).unwrap();
        assert_eq!((f.seq, f.ttl, f.ts), (1000, 11, None));
        assert!(packet::checksums_ok(&near.packets[0]));
        let bad = first_segment(&o, &hello, &c, &BADSEQ, None, &mut rnd).unwrap();
        assert_eq!(bad.fallback, None);
        let f = packet::parse(&bad.packets[0]).unwrap();
        assert_eq!((f.seq, f.ack, f.ts), (1000u32.wrapping_sub(10000), 7777u32.wrapping_sub(66000), None));
        let md5 = first_segment(&o, &hello, &c, &MD5SIG, None, &mut rnd).unwrap();
        let f = packet::parse(&md5.packets[0]).unwrap();
        assert_eq!((f.seq, f.tcp_len), (1000, 40));
        assert_eq!(&md5.packets[0][40..42], &[19, 18]);
        let tiny = sample_v4(b"\x16", true);
        assert!(first_segment(&packet::parse(&tiny).unwrap(), &hello, &c, &GENERAL, None, &mut rnd).is_none());
    }

    #[test]
    fn ttl_badsum_and_disorder() {
        let c = Config::general(google()).unwrap();
        let hello = tls::with_sni(FAKE_DEFAULT, "www.youtube.com").unwrap();
        let raw = sample_v4(&hello, true);
        let o = packet::parse(&raw).unwrap();
        let mut rnd = zeros;
        assert_eq!((fake_ttl(Some(12)), fake_ttl(Some(40)), fake_ttl(Some(3)), fake_ttl(None)), (Some(11), Some(20), None, None));
        let t = first_segment(&o, &hello, &c, &TTL, Some(12), &mut rnd).unwrap();
        assert_eq!(t.fallback, None);
        let f = packet::parse(&t.packets[0]).unwrap();
        assert_eq!((f.seq, f.ttl, f.ts.unwrap().val), (1000, 11, 0x1000u32.wrapping_sub(600000)));
        let parts: Vec<Tcp> = t.packets[8..].iter().map(|p| packet::parse(p).unwrap()).collect();
        assert!(parts.iter().all(|p| p.ttl == 128));
        let far = first_segment(&o, &hello, &c, &TTL, None, &mut rnd).unwrap();
        assert_eq!(far.fallback, Some("ts"));
        assert_eq!(packet::parse(&far.packets[0]).unwrap().ttl, 128);
        let s = first_segment(&o, &hello, &c, &BADSUM, None, &mut rnd).unwrap();
        assert!(!packet::checksums_ok(&s.packets[0]));
        assert!(s.packets[8..].iter().all(|p| packet::checksums_ok(p)));
        let f = packet::parse(&s.packets[0]).unwrap();
        assert_eq!((f.seq, f.ts), (1000, o.ts));
        let d = first_segment(&o, &hello, &c, &DISORDER, None, &mut rnd).unwrap();
        let ps: Vec<Tcp> = d.packets.iter().map(|p| packet::parse(p).unwrap()).collect();
        assert_eq!((ps[8].seq, ps[8].ip_id), (1001, 0x1235));
        assert_eq!((ps[9].seq, ps[9].ip_id, ps[9].payload().len()), (1000u32.wrapping_sub(681), 0x1236, 682));
        assert_eq!(d.next_ip_id, 0x1237);
    }

    #[test]
    fn sni_split_cuts_inside_the_host_name() {
        let c = Config::general(google()).unwrap();
        let hello = tls::with_sni(FAKE_DEFAULT, "www.youtube.com").unwrap();
        let (at, len) = tls::sni_range(&hello).unwrap();
        let raw = sample_v4(&hello, true);
        let o = packet::parse(&raw).unwrap();
        let mut rnd = zeros;
        let out = first_segment(&o, &hello, &c, &SNI_SPLIT, None, &mut rnd).unwrap();
        assert_eq!(out.split_pos, at + len / 2);
        let ps: Vec<Tcp> = out.packets.iter().map(|p| packet::parse(p).unwrap()).collect();
        assert_eq!(ps[8].payload().len(), 681 + out.split_pos);
        assert_eq!(ps[9].seq, 1000 + out.split_pos as u32);
        assert!(!ps[8].payload().windows(15).any(|w| w == b"www.youtube.com"));
        assert!(!ps[9].payload().windows(15).any(|w| w == b"www.youtube.com"));
    }

    #[test]
    fn random_varies_but_stays_valid() {
        let c = Config::general(google()).unwrap();
        let hello = tls::with_sni(FAKE_DEFAULT, "www.youtube.com").unwrap();
        let raw = sample_v4(&hello, true);
        let o = packet::parse(&raw).unwrap();
        let (at, len) = tls::sni_range(&hello).unwrap();
        let mut seed = 1u32;
        let mut rnd = move |b: &mut [u8]| {
            for x in b.iter_mut() {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                *x = (seed >> 16) as u8;
            }
        };
        let mut names = std::collections::HashSet::new();
        let mut repeats = std::collections::HashSet::new();
        let mut cuts = std::collections::HashSet::new();
        for _ in 0..60 {
            let out = first_segment(&o, &hello, &c, &RANDOM, None, &mut rnd).unwrap();
            let n = out.packets.len() - 2;
            assert!((6..=10).contains(&n));
            repeats.insert(n);
            assert!(out.split_pos >= 1 && out.split_pos <= at + len);
            cuts.insert(out.split_pos);
            names.insert(tls::sni(packet::parse(&out.packets[0]).unwrap().payload()).unwrap());
        }
        assert!(names.len() >= 4 && repeats.len() >= 3 && cuts.len() >= 10, "{names:?} {repeats:?} {}", cuts.len());
    }

    #[test]
    fn big_cut_shrinks_overlap_to_fit_mtu() {
        let c = Config::general(google()).unwrap();
        let mut rnd = zeros;
        let data = vec![0x16u8; 1460];
        let raw = sample_v4(&data, true);
        let o = packet::parse(&raw).unwrap();
        let far = Technique { split: Split::Pos(1200), ..GENERAL };
        let out = first_segment(&o, &data, &c, &far, None, &mut rnd).unwrap();
        let p1 = packet::parse(&out.packets[8]).unwrap();
        assert_eq!(p1.payload().len(), 1400);
        let very = Technique { split: Split::Pos(1380), ..GENERAL };
        let out = first_segment(&o, &data, &c, &very, None, &mut rnd).unwrap();
        assert_eq!(packet::parse(&out.packets[8]).unwrap().payload().len(), 1380);
    }
}
