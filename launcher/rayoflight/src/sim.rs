use crate::desync::{Config, FAKE_DEFAULT};
use crate::flows::{Action, BG_APP, BG_EVERY, BG_IDLE, Engine, Lists, Mode, Outcome, REASM_WAIT, SILENT};
use crate::lists::{Hosts, Nets};
use crate::packet::{self, Tcp};
use crate::tls;
use std::time::{Duration, Instant};

fn be16(b: &[u8], at: usize) -> usize {
    u16::from_be_bytes([b[at], b[at + 1]]) as usize
}

fn hello(sni: &str, pad: usize) -> Vec<u8> {
    let mut h = tls::with_sni(FAKE_DEFAULT, sni).unwrap();
    if pad == 0 {
        return h;
    }
    let mut p = 43;
    p += 1 + h[p] as usize;
    p += 2 + be16(&h, p);
    p += 1 + h[p] as usize;
    let total_at = p;
    let mut ext = vec![0x00, 0x15];
    ext.extend_from_slice(&(pad as u16).to_be_bytes());
    ext.extend(std::iter::repeat_n(0u8, pad));
    let grow = ext.len();
    h.splice(total_at + 2..total_at + 2, ext);
    let add = |h: &mut Vec<u8>, at: usize| {
        let v = (be16(h, at) + grow) as u16;
        h[at..at + 2].copy_from_slice(&v.to_be_bytes());
    };
    add(&mut h, 3);
    add(&mut h, total_at);
    let hs = (((h[6] as usize) << 16) | ((h[7] as usize) << 8) | h[8] as usize) + grow;
    h[6..9].copy_from_slice(&(hs as u32).to_be_bytes()[1..]);
    h
}

fn template(ts: bool) -> Vec<u8> {
    packet::tests::sample_v4(b"x", ts)
}

fn seg(tpl: &[u8], seq: u32, ip_id: u16, data: &[u8]) -> Vec<u8> {
    let t = packet::parse(tpl).unwrap();
    packet::segment(&t, seq, t.ts, ip_id, data)
}

fn engine(hosts: &str, excl_ip: &str) -> Engine<u8> {
    let mut h = Hosts::default();
    h.add_text(hosts);
    let pattern = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/tls_clienthello_www_google_com.bin")).unwrap();
    let lists = Lists { hosts: h, ip_excludes: Nets::parse_text(excl_ip), ..Default::default() };
    let mut n = 0u8;
    Engine::new(
        Config::general(pattern).unwrap(),
        lists,
        Box::new(move |b: &mut [u8]| {
            for x in b.iter_mut() {
                n = n.wrapping_add(37);
                *x = n;
            }
        }),
    )
}

const SIM_HOPS: u8 = 12;

fn server(pkts: &[Vec<u8>], start: u32, ts_recent: Option<u32>) -> Vec<u8> {
    let mut recent = ts_recent;
    let mut buf: Vec<Option<u8>> = Vec::new();
    for p in pkts {
        let t = packet::parse(p).unwrap();
        if t.ttl < SIM_HOPS || !packet::checksums_ok(p) {
            continue;
        }
        if let (Some(r), Some(ts)) = (recent, t.ts) {
            if (ts.val.wrapping_sub(r) as i32) < 0 {
                continue;
            }
        }
        if let Some(ts) = t.ts {
            recent = Some(recent.map_or(ts.val, |r| if (ts.val.wrapping_sub(r) as i32) > 0 { ts.val } else { r }));
        }
        let off = t.seq.wrapping_sub(start) as i32 as i64;
        let mut data = t.payload();
        let mut at = off;
        if at < 0 {
            let cut = (-at) as usize;
            if cut >= data.len() {
                continue;
            }
            data = &data[cut..];
            at = 0;
        }
        let at = at as usize;
        if buf.len() < at + data.len() {
            buf.resize(at + data.len(), None);
        }
        for (i, b) in data.iter().enumerate() {
            buf[at + i].get_or_insert(*b);
        }
    }
    buf.into_iter().map_while(|b| b).collect()
}

fn dpi_first(pkts: &[Vec<u8>]) -> Option<String> {
    pkts.iter().map(|p| packet::parse(p).unwrap()).find(|t| tls::is_client_hello(t.payload())).and_then(|t| tls::sni(t.payload()))
}

fn dpi_stream(pkts: &[Vec<u8>]) -> Option<String> {
    let parsed: Vec<Tcp> = pkts.iter().map(|p| packet::parse(p).unwrap()).filter(|t| !t.payload().is_empty()).collect();
    let base = parsed.first()?.seq;
    let mut buf: Vec<Option<u8>> = Vec::new();
    for t in &parsed {
        let off = t.seq.wrapping_sub(base) as i32;
        if off < 0 {
            continue;
        }
        let at = off as usize;
        if buf.len() < at + t.payload().len() {
            buf.resize(at + t.payload().len(), None);
        }
        for (i, b) in t.payload().iter().enumerate() {
            buf[at + i].get_or_insert(*b);
        }
    }
    let stream: Vec<u8> = buf.into_iter().map_while(|b| b).collect();
    tls::sni(&stream)
}

fn sent(a: Action<u8>) -> Vec<Vec<u8>> {
    match a {
        Action::Send(v) => v.into_iter().map(|(p, _)| p).collect(),
        other => panic!("expected Send, got {other:?}"),
    }
}

#[test]
fn single_packet_hello_fools_dpi_but_reaches_server() {
    let h = hello("www.youtube.com", 0);
    let tpl = template(true);
    let mut e = engine("youtube.com", "");
    let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, Instant::now()));
    assert_eq!(out.len(), 10);
    assert_eq!(server(&out, 1000, Some(0x1000)), h);
    assert_eq!(dpi_first(&out).as_deref(), Some("www.google.com"));
    assert_eq!(dpi_stream(&out).as_deref(), Some("www.google.com"));
    assert_eq!(dpi_first(&[seg(&tpl, 1000, 1, &h)]).as_deref(), Some("www.youtube.com"));
    assert_eq!(e.stats.handled, 1);
}

#[test]
fn two_segment_hello_with_sni_in_second_part() {
    let h = hello("rr1.googlevideo.com", 1300);
    assert_eq!(tls::record_len(&h), Some(h.len()));
    assert_eq!(tls::sni(&h).as_deref(), Some("rr1.googlevideo.com"));
    let tpl = template(true);
    let (a, b) = h.split_at(1200);
    assert_eq!(tls::sni(a), None);
    let mut e = engine("googlevideo.com", "");
    let now = Instant::now();
    assert_eq!(e.handle(&seg(&tpl, 1000, 0x1234, a), 1, now), Action::Hold);
    assert_eq!(e.handle(&seg(&tpl, 1000, 0x1234, a), 1, now), Action::Drop);
    let out = sent(e.handle(&seg(&tpl, 2200, 0x1235, b), 2, now));
    assert_eq!(out.len(), 11);
    let last = packet::parse(&out[10]).unwrap();
    assert_eq!((last.seq, last.ip_id, last.payload()), (2200, 0x1237, b));
    assert!(packet::checksums_ok(&out[10]));
    assert_eq!(server(&out, 1000, Some(0x1000)), h);
    assert_eq!(dpi_stream(&out).as_deref(), Some("www.google.com"));
}

#[test]
fn without_timestamps_falls_back_to_badseq() {
    let h = hello("www.youtube.com", 0);
    let tpl = template(false);
    let mut e = engine("youtube.com", "");
    let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, Instant::now()));
    assert_eq!(out.len(), 10);
    assert_eq!(server(&out, 1000, None), h);
    assert_eq!(dpi_first(&out).as_deref(), Some("www.google.com"));
    assert_eq!((e.stats.no_ts, e.stats.fallback), (1, 1));
}

fn syn(tpl: &[u8]) -> Vec<u8> {
    let mut t = packet::parse(tpl).unwrap();
    t.flags = 0x02;
    packet::segment(&t, 999, t.ts, 1, b"")
}

fn syn_ack(tpl: &[u8], ttl: u8) -> Vec<u8> {
    let mut p = reply(tpl, 0x12, b"");
    packet::set_ttl(&mut p, ttl);
    p
}

#[test]
fn hops_from_syn_ack_drive_the_ttl_fake() {
    for ts in [true, false] {
        let h = hello("www.youtube.com", 0);
        let tpl = template(ts);
        let mut e = engine("youtube.com", "").with_mode(Mode::parse("ttl").unwrap());
        let now = Instant::now();
        assert_eq!(e.handle(&syn(&tpl), 0, now), Action::Pass);
        e.observe(&syn_ack(&tpl, 64 - SIM_HOPS), now);
        let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, now));
        let f = packet::parse(&out[0]).unwrap();
        assert_eq!((f.ttl, f.seq), (SIM_HOPS - 1, 1000));
        assert_eq!(server(&out, 1000, ts.then_some(0x1000)), h);
        assert_eq!(dpi_first(&out).as_deref(), Some("www.google.com"));
        assert_eq!(e.stats.fallback, 0);
    }
    let h = hello("www.youtube.com", 0);
    let tpl = template(false);
    let mut e = engine("youtube.com", "");
    let now = Instant::now();
    let _ = e.handle(&syn(&tpl), 0, now);
    e.observe(&syn_ack(&tpl, 128 - SIM_HOPS), now);
    let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, now));
    assert_eq!(packet::parse(&out[0]).unwrap().ttl, SIM_HOPS - 1);
    assert_eq!(server(&out, 1000, None), h);
    assert_eq!(e.stats.fallback, 1);
}

#[test]
fn unlisted_or_excluded_sites_are_untouched() {
    let tpl = template(true);
    let mut e = engine("youtube.com", "142.250.0.0/16");
    let h = hello("www.youtube.com", 0);
    assert_eq!(e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now()), Action::Pass);
    let mut e = engine("youtube.com", "");
    let other = hello("example.org", 1300);
    let (a, b) = other.split_at(1200);
    let pa = seg(&tpl, 1000, 1, a);
    let pb = seg(&tpl, 2200, 2, b);
    assert_eq!(e.handle(&pa, 1, Instant::now()), Action::Hold);
    assert_eq!(sent(e.handle(&pb, 2, Instant::now())), vec![pa, pb]);
    assert_eq!(e.stats.skipped, 1);
}

#[test]
fn later_packets_and_non_tls_pass() {
    let tpl = template(true);
    let mut e = engine("youtube.com", "");
    let h = hello("www.youtube.com", 0);
    let _ = e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now());
    assert_eq!(e.handle(&seg(&tpl, 1000 + h.len() as u32, 2, b"\x17\x03\x03\x00\x10data"), 0, Instant::now()), Action::Pass);
    assert_eq!(e.handle(&seg(&tpl, 5000, 3, b"GET / HTTP/1.1\r\n"), 0, Instant::now()), Action::Pass);
}

#[test]
fn stalled_reassembly_is_flushed_unchanged() {
    let tpl = template(true);
    let mut e = engine("googlevideo.com", "");
    let h = hello("rr1.googlevideo.com", 1300);
    let pa = seg(&tpl, 1000, 1, &h[..1200]);
    let t0 = Instant::now();
    assert_eq!(e.handle(&pa, 9, t0), Action::Hold);
    assert!(e.expire(t0 + Duration::from_millis(10)).is_empty());
    assert_eq!(e.expire(t0 + REASM_WAIT), vec![(pa.clone(), 9)]);
    let gap = seg(&tpl, 1000, 1, &h[..1200]);
    let mut e = engine("googlevideo.com", "");
    assert_eq!(e.handle(&gap, 1, t0), Action::Hold);
    let far = seg(&tpl, 9000, 2, b"zz");
    assert_eq!(sent(e.handle(&far, 2, t0)), vec![gap, far]);
}

fn with_apps(hosts: &str, all: &str, skip: &str, app: &'static str) -> Engine<u8> {
    let mut e = engine(hosts, "");
    e.lists_mut().apps = crate::procs::Apps::parse(&[all.to_string()], &[skip.to_string()]);
    e.verbose = true;
    e.with_apps(Box::new(move |_, _| Some(crate::procs::Owner { pid: 77, name: Some(app.to_string()), source: crate::procs::Source::Event })))
}

#[test]
fn whole_app_is_handled_even_off_list() {
    let tpl = template(true);
    let h = hello("example.org", 0);
    let mut e = with_apps("youtube.com", "curl", "", "curl.exe");
    assert_eq!(sent(e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now())).len(), 10);
    assert_eq!((e.stats.handled, e.stats.app_all), (1, 1));
    assert_eq!(e.notes, vec!["обработан example.org (curl.exe, pid 77, по событию): вся программа, приём general".to_string()]);
    let mut e = with_apps("youtube.com", "curl", "", "chrome.exe");
    assert_eq!(e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now()), Action::Pass);
    assert_eq!(e.notes, vec!["пропущен example.org (chrome.exe, pid 77, по событию): сайта нет в списке".to_string()]);
}

#[test]
fn decisions_are_kept_for_the_live_panel() {
    let tpl = template(true);
    let mut e = with_apps("youtube.com", "", "steam", "chrome.exe").with_clock(|| "12:00:00".to_string());
    e.track = true;
    let _ = e.handle(&seg(&tpl, 1000, 1, &hello("www.youtube.com", 0)), 0, Instant::now());
    let mut other = seg(&tpl, 1000, 1, &hello("example.org", 0));
    other[20..22].copy_from_slice(&50001u16.to_be_bytes());
    packet::fix_checksums(&mut other);
    let _ = e.handle(&other, 0, Instant::now());
    let d: Vec<(bool, &str, Option<&str>)> = e.recent.iter().map(|d| (d.handled, d.why, d.host.as_deref())).collect();
    assert_eq!(d, vec![(true, "сайт в списке", Some("www.youtube.com")), (false, "сайта нет в списке", Some("example.org"))]);
    assert_eq!((e.recent[0].app.as_deref(), e.recent[0].pid, e.recent[0].time.as_str()), (Some("chrome.exe"), Some(77), "12:00:00"));
    let mut quiet = engine("youtube.com", "");
    let _ = quiet.handle(&seg(&tpl, 1000, 1, &hello("www.youtube.com", 0)), 0, Instant::now());
    assert!(quiet.recent.is_empty());
}

#[test]
fn skipped_app_is_never_touched() {
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let mut e = with_apps("youtube.com", "steam", "steam", "steam.exe");
    assert_eq!(e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now()), Action::Pass);
    assert_eq!((e.stats.handled, e.stats.app_skipped), (0, 1));
    assert_eq!(e.notes, vec!["пропущен www.youtube.com (steam.exe, pid 77, по событию): программа в исключениях".to_string()]);
    let mut e = with_apps("youtube.com", "", "steam", "chrome.exe");
    assert_eq!(sent(e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now())).len(), 10);
}

#[test]
fn site_excludes_beat_whole_app() {
    let tpl = template(true);
    let h = hello("bank.example", 0);
    let mut e = with_apps("youtube.com", "curl", "", "curl.exe");
    e.lists_mut().host_excludes.add("bank.example");
    assert_eq!(e.handle(&seg(&tpl, 1000, 1, &h), 0, Instant::now()), Action::Pass);
    assert_eq!(e.notes, vec!["пропущен bank.example (curl.exe, pid 77, по событию): сайт в исключениях".to_string()]);
}

fn reply(tpl: &[u8], flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut t = packet::parse(tpl).unwrap();
    std::mem::swap(&mut t.src, &mut t.dst);
    std::mem::swap(&mut t.sport, &mut t.dport);
    t.flags = flags;
    packet::segment(&t, 5000, None, 9, payload)
}

fn server_hello() -> Vec<u8> {
    let mut d = vec![0x16, 0x03, 0x03, 0x00, 0x30, 0x02];
    d.resize(0x35, 0);
    d
}

fn with_port(pkt: &[u8], port: u16) -> Vec<u8> {
    let mut p = pkt.to_vec();
    p[20..22].copy_from_slice(&port.to_be_bytes());
    packet::fix_checksums(&mut p);
    p
}

fn first_cut(out: &[Vec<u8>]) -> usize {
    let p = packet::parse(out.last().unwrap()).unwrap();
    (p.seq - 1000) as usize
}

fn with_ack(pkt: &[u8], ack: u32) -> Vec<u8> {
    let mut p = pkt.to_vec();
    let ihl = ((p[0] & 0x0f) as usize) * 4;
    p[ihl + 8..ihl + 12].copy_from_slice(&ack.to_be_bytes());
    packet::fix_checksums(&mut p);
    p
}

#[test]
fn gateway_learns_answer_from_device_ack() {
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let mut e = engine("youtube.com", "");
    e.track = true;
    e.forward = true;
    let t0 = Instant::now();
    let out = sent(e.handle(&seg(&tpl, 1000, 1, &h), 0, t0));
    assert!(out.len() > 2);
    assert_eq!(e.stats.handled, 1);
    let first = packet::parse(&tpl).unwrap();
    assert_eq!(e.recent.back().unwrap().app.as_deref(), Some(format!("{}{}", crate::flows::DEVICE, first.src).as_str()));
    let again = seg(&tpl, 1000, 2, &h);
    let _ = e.handle(&again, 0, t0);
    assert_eq!((e.pending(), e.stats.answered), (1, 0));
    let next = with_ack(&seg(&tpl, 1000 + h.len() as u32, 3, b"\x14\x03\x03\x00\x01\x01"), first.ack.wrapping_add(4000));
    let _ = e.handle(&next, 0, t0);
    assert_eq!((e.pending(), e.stats.answered), (0, 1));
    assert_eq!(e.recent.back().unwrap().outcome, Some(Outcome::Answered));
    let mut silent = engine("youtube.com", "");
    silent.forward = true;
    let _ = silent.handle(&seg(&tpl, 1000, 1, &h), 0, t0);
    let _ = silent.expire(t0 + SILENT);
    assert_eq!((silent.pending(), silent.stats.silent), (0, 1));
}

#[test]
fn outcomes_are_counted_per_technique() {
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let mut e = engine("youtube.com", "");
    e.track = true;
    let t0 = Instant::now();
    let _ = e.handle(&seg(&tpl, 1000, 1, &h), 0, t0);
    e.observe(&reply(&tpl, 0x18, &server_hello()), t0);
    let _ = e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50001), 0, t0);
    let mut rst = reply(&tpl, 0x04, b"");
    rst[22..24].copy_from_slice(&50001u16.to_be_bytes());
    packet::fix_checksums(&mut rst);
    e.observe(&rst, t0);
    let _ = e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50002), 0, t0);
    assert_eq!(e.pending(), 1);
    let _ = e.expire(t0 + SILENT);
    assert_eq!((e.stats.answered, e.stats.reset, e.stats.silent), (1, 1, 1));
    let g = e.stats.by_tech["general"];
    assert_eq!((g.used, g.answered, g.reset, g.silent), (3, 1, 1, 1));
    let outs: Vec<Option<Outcome>> = e.recent.iter().map(|d| d.outcome).collect();
    assert_eq!(outs, vec![Some(Outcome::Answered), Some(Outcome::Reset), Some(Outcome::Silent)]);
    e.observe(&reply(&tpl, 0x18, b"\x17\x03\x03\x00\x10data"), t0);
    assert_eq!(e.stats.answered, 1);
}

#[test]
fn adaptive_moves_to_next_technique_after_failure() {
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let mut e = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap());
    let t0 = Instant::now();
    let first = sent(e.handle(&seg(&tpl, 1000, 1, &h), 0, t0));
    assert_eq!(first_cut(&first), 1);
    let mut rst = reply(&tpl, 0x04, b"");
    packet::fix_checksums(&mut rst);
    e.observe(&rst, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.fails)), Some(("general", 1)));
    let again = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50003), 0, t0));
    assert_eq!(first_cut(&again), 1);
    let mut rst2 = reply(&tpl, 0x04, b"");
    rst2[22..24].copy_from_slice(&50003u16.to_be_bytes());
    packet::fix_checksums(&mut rst2);
    e.observe(&rst2, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.fails)), Some(("sni-split", 2)));
    let second = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50001), 0, t0));
    let (at, len) = tls::sni_range(&h).unwrap();
    assert_eq!(first_cut(&second), at + len / 2);
    let mut ok = reply(&tpl, 0x18, &server_hello());
    ok[22..24].copy_from_slice(&50001u16.to_be_bytes());
    packet::fix_checksums(&mut ok);
    e.observe(&ok, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.ok, m.fails)), Some(("sni-split", 1, 2)));
    assert_eq!(e.take_redeemed(), vec![t0, t0]);
    assert!(e.take_redeemed().is_empty());
    let third = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50004), 0, t0));
    assert_eq!(first_cut(&third), at + len / 2);
    let mut rst3 = reply(&tpl, 0x04, b"");
    rst3[22..24].copy_from_slice(&50004u16.to_be_bytes());
    packet::fix_checksums(&mut rst3);
    e.observe(&rst3, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.streak)), Some(("sni-split", 1)));
    let saved = e.memory_text();
    assert_eq!(saved, "www.youtube.com\tsni-split\t1\t3\n");
    let fourth = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50005), 0, t0));
    assert_eq!(first_cut(&fourth), at + len / 2);
    let mut rst4 = reply(&tpl, 0x04, b"");
    rst4[22..24].copy_from_slice(&50005u16.to_be_bytes());
    packet::fix_checksums(&mut rst4);
    e.observe(&rst4, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, _)| t), Some("sni-max"));
    for port in [50010u16, 50011] {
        let out = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), port), 0, t0));
        assert_eq!(dpi_first(&out).as_deref(), Some("max.ru"));
        let mut rst = reply(&tpl, 0x04, b"");
        rst[22..24].copy_from_slice(&port.to_be_bytes());
        packet::fix_checksums(&mut rst);
        e.observe(&rst, t0);
    }
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, _)| t), Some("none"));
    let raw = with_port(&seg(&tpl, 1000, 1, &h), 50006);
    assert_eq!(e.handle(&raw, 0, t0), Action::Pass);
    let mut ok2 = reply(&tpl, 0x18, &server_hello());
    ok2[22..24].copy_from_slice(&50006u16.to_be_bytes());
    packet::fix_checksums(&mut ok2);
    e.observe(&ok2, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.ok, m.streak)), Some(("none", 2, 0)));
    assert_eq!(e.take_redeemed().len(), 4);
    assert_eq!(e.stats.by_tech["none"].answered, 1);
    let mut fresh = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap());
    assert_eq!(fresh.load_memory(&e.memory_text()), 1);
    assert_eq!(fresh.memory_of("www.youtube.com").map(|(t, _)| t), Some("none"));
    let mut fresh = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap());
    assert_eq!(fresh.load_memory(&saved), 1);
    assert_eq!(fresh.memory_of("www.youtube.com").map(|(t, _)| t), Some("sni-split"));
    assert_eq!(fresh.load_memory("bad line\nx.com\tunknown\t1\t1\n"), 0);
    let mut fixed = engine("youtube.com", "");
    assert_eq!(fixed.load_memory(&saved), 0);
    assert_eq!(fixed.memory_text(), "");
}

#[test]
fn every_technique_keeps_the_server_stream_intact() {
    for name in ["general", "sni-split", "random", "badseq", "md5sig", "ttl", "badsum", "disorder", "overlap", "fake-only", "sni-vk", "sni-fonts", "sni-max", "fakedsplit", "fakedsplit-ttl", "overlap-max", "overlap-2", "overlap-sld", "fake-ts"] {
        for pad in [0usize, 1300] {
            let tpl = template(true);
            let h = hello("rr1.googlevideo.com", pad);
            let mut e = engine("googlevideo.com", "").with_mode(Mode::parse(name).unwrap());
            let now = Instant::now();
            let out = if pad == 0 {
                sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, now))
            } else {
                let (a, b) = h.split_at(1200);
                assert_eq!(e.handle(&seg(&tpl, 1000, 0x1234, a), 1, now), Action::Hold);
                sent(e.handle(&seg(&tpl, 2200, 0x1235, b), 2, now))
            };
            let kept: Vec<Vec<u8>> = out
                .iter()
                .filter(|p| {
                    let t = packet::parse(p).unwrap();
                    let fake_badseq = t.ack == 7777u32.wrapping_sub(66000);
                    let fake_md5 = p.len() > 41 && p[40] == 19;
                    !fake_badseq && !fake_md5
                })
                .cloned()
                .collect();
            assert_eq!(server(&kept, 1000, Some(0x1000)), h, "{name} pad {pad}");
            assert!(dpi_first(&out).is_none_or(|s| s != "rr1.googlevideo.com"), "{name} pad {pad}");
            assert!(out.iter().all(|p| packet::parse(p).unwrap().payload().len() <= 1400 + 681), "{name} pad {pad}");
        }
    }
}

#[test]
fn server_without_paws_needs_a_technique_without_ts_fakes() {
    let tpl = template(true);
    let h = hello("soundcloud.com", 0);
    for (name, intact) in [("general", false), ("overlap", true), ("fake-only", true)] {
        let mut e = engine("soundcloud.com", "").with_mode(Mode::parse(name).unwrap());
        let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, Instant::now()));
        let kept: Vec<Vec<u8>> = out.iter().filter(|p| packet::parse(p).unwrap().ack != 7777u32.wrapping_sub(66000)).cloned().collect();
        assert_eq!(server(&kept, 1000, None) == h, intact, "{name}");
        assert!(dpi_first(&out).is_some_and(|s| s != "soundcloud.com"), "{name}");
    }
}

#[test]
fn unlisted_site_joins_autolist_after_two_silences() {
    let tpl = template(true);
    let h = hello("blocked.example", 0);
    let mut e = engine("youtube.com", "");
    e.autolist = true;
    let t0 = Instant::now();
    assert_eq!(e.handle(&seg(&tpl, 1000, 1, &h), 0, t0), Action::Pass);
    assert_eq!(e.pending(), 1);
    let _ = e.expire(t0 + SILENT);
    assert_eq!(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50001), 0, t0 + SILENT), Action::Pass);
    let _ = e.expire(t0 + SILENT + SILENT);
    assert_eq!(e.take_auto_new(), vec!["blocked.example".to_string()]);
    assert_eq!(e.stats.auto_added, 1);
    assert_eq!((e.stats.silent, e.stats.answered), (0, 0));
    let out = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50002), 0, t0 + SILENT + SILENT));
    assert_eq!(out.len(), 10);
    let fine = hello("fine.example", 0);
    let _ = e.handle(&with_port(&seg(&tpl, 1000, 1, &fine), 50003), 0, t0);
    let mut ok = reply(&tpl, 0x18, &server_hello());
    ok[22..24].copy_from_slice(&50003u16.to_be_bytes());
    packet::fix_checksums(&mut ok);
    e.observe(&ok, t0);
    let _ = e.handle(&with_port(&seg(&tpl, 1000, 1, &fine), 50004), 0, t0);
    let _ = e.expire(t0 + SILENT);
    assert!(e.take_auto_new().is_empty());
    let mut off = engine("youtube.com", "");
    let _ = off.handle(&seg(&tpl, 1000, 1, &h), 0, t0);
    assert_eq!(off.pending(), 0);
    let mut excl = engine("youtube.com", "");
    excl.autolist = true;
    excl.lists_mut().host_excludes.add("blocked.example");
    let _ = excl.handle(&seg(&tpl, 1000, 1, &h), 0, t0);
    assert_eq!(excl.pending(), 0);
}

#[test]
fn syn_marks_start_and_mid_stream_hello_is_ignored() {
    let tpl = template(true);
    let t = packet::parse(&tpl).unwrap();
    let mut syn = packet::segment(&t, 999, t.ts, 1, b"");
    let ip = 20;
    syn[ip + 13] = 0x02;
    packet::fix_checksums(&mut syn);
    let mut e = engine("youtube.com", "");
    assert_eq!(e.handle(&syn, 0, Instant::now()), Action::Pass);
    let h = hello("www.youtube.com", 0);
    assert_eq!(e.handle(&seg(&tpl, 5000, 1, &h), 0, Instant::now()), Action::Pass);
    assert_eq!(e.flows(), 1);
}

mod quic_sim {
    use super::*;
    use crate::desync::SNI_POOL;
    use crate::flows::{QuicCfg, QuicMode};
    use crate::quic;

    const DCID: [u8; 8] = [9, 8, 7, 6, 5, 4, 3, 2];

    fn yt_hello() -> Vec<u8> {
        let (_, h) = quic::hello_of(&quic::tests::google()).unwrap();
        quic::with_sni(&h, "www.youtube.com").unwrap()
    }

    fn dgram(hello: &[u8], pn: u64, from: usize, to: usize) -> Vec<u8> {
        let mut frames = Vec::new();
        quic::crypto_frame(&mut frames, from as u64, &hello[from..to]);
        let pkt = quic::seal_initial(&DCID, &[], &[], pn, &frames, quic::MIN_DATAGRAM);
        packet::tests::sample_udp(&pkt)
    }

    fn server_reply(d: &[u8], first: u8) -> Vec<u8> {
        let mut p = d[..28].to_vec();
        p.extend_from_slice(&[first, 0, 0, 0, 1, 0, 0]);
        let total = p.len() as u16;
        p[2..4].copy_from_slice(&total.to_be_bytes());
        let (a, b) = (p[12..16].to_vec(), p[16..20].to_vec());
        p[12..16].copy_from_slice(&b);
        p[16..20].copy_from_slice(&a);
        let (s, t) = (p[20..22].to_vec(), p[22..24].to_vec());
        p[20..22].copy_from_slice(&t);
        p[22..24].copy_from_slice(&s);
        p[24..26].copy_from_slice(&15u16.to_be_bytes());
        packet::fix_checksums(&mut p);
        p
    }

    fn eng(mode: &str) -> Engine<u8> {
        engine("youtube.com", "").with_quic(QuicCfg::new(QuicMode::parse(mode).unwrap(), quic::tests::google()).unwrap())
    }

    fn payload(p: &[u8]) -> Vec<u8> {
        packet::parse_udp(p).unwrap().payload().to_vec()
    }

    #[test]
    fn like_winws_sends_eleven_google_fakes_first() {
        let d = dgram(&yt_hello(), 0, 0, yt_hello().len());
        let mut e = eng("general");
        let t0 = Instant::now();
        let out = sent(e.handle(&d, 0, t0));
        assert_eq!(out.len(), 12);
        assert!(out[..11].iter().all(|p| payload(p) == quic::tests::google() && packet::checksums_ok(p)));
        assert_eq!(out[11], d);
        assert_eq!(e.pending(), 1);
        e.observe(&server_reply(&d, 0x40), t0);
        assert_eq!(e.pending(), 1);
        e.observe(&server_reply(&d, 0xC1), t0);
        assert_eq!((e.pending(), e.stats.answered), (0, 1));
        assert_eq!(e.stats.by_tech["quic-general"].answered, 1);
        assert_eq!(e.handle(&dgram(&yt_hello(), 1, 0, 10), 0, t0), Action::Pass);
    }

    #[test]
    fn gateway_sees_quic_answer_by_new_connection_id() {
        let h = yt_hello();
        let d = dgram(&h, 0, 0, h.len());
        let t0 = Instant::now();
        let mut e = eng("general");
        e.forward = true;
        let _ = sent(e.handle(&d, 0, t0));
        assert_eq!(e.pending(), 1);
        assert_eq!(e.handle(&dgram(&h, 1, 0, h.len()), 0, t0), Action::Pass);
        assert_eq!((e.pending(), e.stats.answered), (1, 0));
        let mut frames = Vec::new();
        quic::crypto_frame(&mut frames, 0, &[0x14]);
        let moved = packet::tests::sample_udp(&quic::seal_initial(&[1, 2, 3, 4, 5, 6, 7, 8], &[], &[], 2, &frames, quic::MIN_DATAGRAM));
        assert_eq!(e.handle(&moved, 0, t0), Action::Pass);
        assert_eq!((e.pending(), e.stats.answered), (0, 1));
    }

    #[test]
    fn random_fakes_carry_other_names() {
        let h = yt_hello();
        let d = dgram(&h, 0, 0, h.len());
        let mut e = eng("random");
        let out = sent(e.handle(&d, 0, Instant::now()));
        assert!((7..=13).contains(&out.len()));
        assert_eq!(out.last().unwrap(), &d);
        for f in &out[..out.len() - 1] {
            let p = payload(f);
            assert!(p.len() >= quic::MIN_DATAGRAM);
            let (_, fh) = quic::hello_of(&p).unwrap();
            assert!(SNI_POOL.contains(&quic::sni(&fh).unwrap().as_str()));
        }
    }

    #[test]
    fn shuffle_keeps_size_and_hello_but_hides_the_name() {
        let h = yt_hello();
        let d = dgram(&h, 0, 0, h.len());
        let mut e = eng("shuffle");
        let out = sent(e.handle(&d, 0, Instant::now()));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), d.len());
        assert!(packet::checksums_ok(&out[0]));
        let p = payload(&out[0]);
        let (i, back) = quic::hello_of(&p).unwrap();
        assert_eq!((back, i.pn), (h, 0));
        let chunks = quic::crypto_chunks(&i.frames).unwrap();
        assert!(chunks.len() > 4);
        assert_ne!(chunks[0].0, 0);
        assert!(!chunks.iter().any(|(_, b)| b.windows(15).any(|w| w == b"www.youtube.com")));
    }

    fn from_port(p: &[u8], port: u16) -> Vec<u8> {
        let mut q = p.to_vec();
        q[20..22].copy_from_slice(&port.to_be_bytes());
        packet::fix_checksums(&mut q);
        q
    }

    #[test]
    fn quic_adaptive_walks_to_block_and_stays() {
        let h = yt_hello();
        let base = dgram(&h, 0, 0, h.len());
        let mut e = eng("adaptive");
        let mut t = Instant::now();
        let mut port = 51000u16;
        let mut next = |e: &mut Engine<u8>, t: Instant| {
            port += 1;
            let d = from_port(&base, port);
            (e.handle(&d, 0, t), d)
        };
        for _ in 0..2 {
            let (a, _) = next(&mut e, t);
            assert_eq!(sent(a).len(), 12);
            t += SILENT;
            let _ = e.expire(t);
        }
        assert_eq!(e.quic_memory_of("www.youtube.com").map(|(m, _)| m), Some("quic-random"));
        let (a, d) = next(&mut e, t);
        let out = sent(a);
        assert!(out.len() > 6 && out.last() == Some(&d));
        e.observe(&server_reply(&d, 0xC1), t);
        assert_eq!(e.quic_memory_of("www.youtube.com").map(|(m, x)| (m, x.ok)), Some(("quic-random", 1)));
        for _ in 0..2 {
            let _ = next(&mut e, t);
            t += SILENT;
            let _ = e.expire(t);
        }
        let (a, d) = next(&mut e, t);
        assert_eq!(sent(a), vec![d]);
        t += SILENT;
        let _ = e.expire(t);
        let _ = next(&mut e, t);
        t += SILENT;
        let _ = e.expire(t);
        assert_eq!(e.quic_memory_of("www.youtube.com").map(|(m, _)| m), Some("quic-block"));
        for _ in 0..3 {
            let (a, _) = next(&mut e, t);
            assert_eq!(a, Action::Drop);
        }
        let saved = e.memory_text();
        assert!(saved.contains("www.youtube.com\tquic-block\t1\t"));
        let mut fresh = eng("adaptive");
        assert_eq!(fresh.load_memory(&saved), 1);
        assert_eq!(fresh.quic_memory_of("www.youtube.com").map(|(m, _)| m), Some("quic-block"));
        let mut fixed = eng("general");
        assert_eq!(fixed.load_memory(&saved), 0);
    }

    #[test]
    fn block_drops_the_whole_flow() {
        let h = yt_hello();
        let mut e = eng("block");
        let now = Instant::now();
        assert_eq!(e.handle(&dgram(&h, 0, 0, h.len()), 0, now), Action::Drop);
        assert_eq!(e.handle(&dgram(&h, 1, 0, h.len()), 0, now), Action::Drop);
        assert_eq!((e.stats.handled, e.pending()), (1, 0));
    }

    #[test]
    fn hello_in_two_datagrams_and_unlisted_sites() {
        let h = yt_hello();
        let mut e = eng("general");
        let now = Instant::now();
        let a = dgram(&h, 1, 150, h.len());
        let b = dgram(&h, 0, 0, 150);
        assert_eq!(e.handle(&a, 1, now), Action::Hold);
        let out = sent(e.handle(&b, 2, now));
        assert_eq!(out.len(), 13);
        assert_eq!((&out[11], &out[12]), (&a, &b));
        let (_, g) = quic::hello_of(&quic::tests::google()).unwrap();
        let mut other = engine("youtube.com", "").with_quic(QuicCfg::new(QuicMode::Fake, quic::tests::google()).unwrap());
        assert_eq!(other.handle(&dgram(&g, 0, 0, g.len()), 0, now), Action::Pass);
        assert_eq!(other.stats.skipped, 1);
        let mut off = engine("youtube.com", "");
        assert_eq!(off.handle(&dgram(&h, 0, 0, h.len()), 0, now), Action::Pass);
    }

    #[test]
    fn stalled_quic_hello_is_released() {
        let h = yt_hello();
        let mut e = eng("general");
        let t0 = Instant::now();
        let part = dgram(&h, 0, 0, 200);
        assert_eq!(e.handle(&part, 7, t0), Action::Hold);
        assert!(e.expire(t0 + Duration::from_millis(100)).is_empty());
        let out = e.expire(t0 + REASM_WAIT);
        assert_eq!(out, vec![(part.clone(), 7)]);
        assert_eq!(e.handle(&dgram(&h, 1, 200, h.len()), 0, t0), Action::Pass);
    }
}

mod general_profiles {
    use super::*;
    use crate::flows::{IDLE, QuicCfg, QuicMode};
    use crate::profiles::{Own, Ports};
    use crate::quic;

    const HTTP_FAKE: &[u8] = b"\x16\x03\x01\x00\x10sferum-fake-bytes";
    const DST: &str = "142.250.74.110/32";

    fn eng(own: &str, hosts: &str, google: &str, ipset: &str) -> Engine<u8> {
        let mut h = Hosts::default();
        h.add_text(hosts);
        let mut g = Hosts::default();
        g.add_text(google);
        let lists = Lists {
            hosts: h,
            google: g,
            ipset: Nets::parse_text(ipset),
            own: Own::parse(own).unwrap(),
            game_tcp: Ports::parse("1024-65535").unwrap(),
            game_udp: Ports::parse("1024-65535").unwrap(),
            ..Default::default()
        };
        let pattern = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/tls_clienthello_www_google_com.bin")).unwrap();
        let mut cfg = Config::general(pattern).unwrap();
        cfg.fake_http = HTTP_FAKE.to_vec();
        cfg.fake_voice = vec![0xAA; 64];
        cfg.fake_game_udp = vec![0xBB; 32];
        let mut n = 0u8;
        Engine::new(
            cfg,
            lists,
            Box::new(move |b: &mut [u8]| {
                for x in b.iter_mut() {
                    n = n.wrapping_add(37);
                    *x = n;
                }
            }),
        )
    }

    fn to_port(p: &[u8], port: u16) -> Vec<u8> {
        let mut q = p.to_vec();
        let at = if q[0] >> 4 == 6 { 40 } else { (q[0] & 0x0f) as usize * 4 };
        q[at + 2..at + 4].copy_from_slice(&port.to_be_bytes());
        packet::fix_checksums(&mut q);
        q
    }

    fn payloads(out: &[Vec<u8>]) -> Vec<Vec<u8>> {
        out.iter().map(|p| packet::parse(p).map(|t| t.payload().to_vec()).unwrap_or_else(|| packet::parse_udp(p).unwrap().payload().to_vec())).collect()
    }

    #[test]
    fn http_on_port_80_uses_the_http_fake() {
        let tpl = to_port(&template(true), 80);
        let req = b"GET / HTTP/1.1\r\nHost: rutracker.org\r\nAccept: */*\r\n\r\n".to_vec();
        let now = Instant::now();
        let mut e = eng("http", "rutracker.org", "", "");
        assert_eq!(e.handle(&syn(&tpl), 0, now), Action::Pass);
        let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &req), 0, now));
        assert_eq!(out.len(), 10);
        assert!(payloads(&out[..8]).iter().all(|p| p == HTTP_FAKE));
        assert_eq!(server(&out, 1000, Some(0x1000)), req);
        assert_eq!(e.stats.handled, 1);
        let mut off = eng("", "rutracker.org", "", "");
        let _ = off.handle(&syn(&tpl), 0, now);
        assert_eq!(off.handle(&seg(&tpl, 1000, 0x1234, &req), 0, now), Action::Pass);
        let mut other = eng("http", "example.org", "", "");
        let _ = other.handle(&syn(&tpl), 0, now);
        assert_eq!(other.handle(&seg(&tpl, 1000, 0x1234, &req), 0, now), Action::Pass);
        assert_eq!((other.stats.skipped, other.recent.len()), (0, 0));
    }

    #[test]
    fn google_list_zeroes_ip_id() {
        let tpl = template(true);
        let h = hello("www.youtube.com", 0);
        let now = Instant::now();
        let mut e = eng("google", "", "youtube.com", "");
        let out = sent(e.handle(&seg(&tpl, 1000, 0x1234, &h), 0, now));
        assert_eq!(out.len(), 10);
        assert!(out.iter().all(|p| packet::parse(p).unwrap().ip_id == 0 && packet::checksums_ok(p)));
        assert_eq!(server(&out, 1000, Some(0x1000)), h);
        let mut plain = eng("", "", "youtube.com", "");
        assert_eq!(plain.handle(&seg(&tpl, 1000, 0x1234, &h), 0, now), Action::Pass);
    }

    #[test]
    fn discord_media_on_cloudflare_ports() {
        let now = Instant::now();
        let mut e = eng("cf", "", "", "");
        let out = sent(e.handle(&seg(&to_port(&template(true), 2053), 1000, 1, &hello("c-fra08.discord.media", 0)), 0, now));
        assert_eq!(out.len(), 10);
        let other = to_port(&template(true), 2083);
        assert_eq!(e.handle(&seg(&other, 1000, 1, &hello("example.org", 0)), 0, now), Action::Pass);
    }

    #[test]
    fn my_ips_cover_tcp_and_quic() {
        let now = Instant::now();
        let mut e = eng("myip-tcp", "", "", DST);
        assert_eq!(sent(e.handle(&seg(&template(true), 1000, 1, &hello("example.org", 0)), 0, now)).len(), 10);
        let alt = to_port(&template(true), 8443);
        assert_eq!(sent(e.handle(&seg(&alt, 1000, 1, &hello("example.org", 0)), 0, now)).len(), 10);
        let mut none = eng("myip-tcp", "", "", "10.0.0.0/8");
        assert_eq!(none.handle(&seg(&template(true), 1000, 1, &hello("example.org", 0)), 0, now), Action::Pass);
        let (_, g) = quic::hello_of(&quic::tests::google()).unwrap();
        let named = quic::with_sni(&g, "example.org").unwrap();
        let mut frames = Vec::new();
        quic::crypto_frame(&mut frames, 0, &named);
        let d = packet::tests::sample_udp(&quic::seal_initial(&[1, 2, 3, 4, 5, 6, 7, 8], &[], &[], 0, &frames, quic::MIN_DATAGRAM));
        let mut q = eng("myip-quic", "", "", "142.250.1.1/32").with_quic(QuicCfg::new(QuicMode::Fake, quic::tests::google()).unwrap());
        assert_eq!(sent(q.handle(&d, 0, now)).len(), 12);
        let mut q2 = eng("", "", "", "142.250.1.1/32").with_quic(QuicCfg::new(QuicMode::Fake, quic::tests::google()).unwrap());
        assert_eq!(q2.handle(&d, 0, now), Action::Pass);
    }

    #[test]
    fn game_tcp_only_first_packet_to_my_ips() {
        let tpl = to_port(&template(true), 27015);
        let data = vec![0x55u8; 40];
        let now = Instant::now();
        let mut e = eng("game-tcp", "", "", DST);
        let _ = e.handle(&syn(&tpl), 0, now);
        let out = sent(e.handle(&seg(&tpl, 1000, 1, &data), 0, now));
        assert!(payloads(&out[..8]).iter().all(|p| p == HTTP_FAKE));
        assert_eq!(server(&out, 1000, Some(0x1000)), data);
        assert_eq!(e.handle(&seg(&tpl, 1040, 2, &data), 0, now), Action::Pass);
        let mut far = eng("game-tcp", "", "", "10.0.0.0/8");
        let _ = far.handle(&syn(&tpl), 0, now);
        assert_eq!(far.handle(&seg(&tpl, 1000, 1, &data), 0, now), Action::Pass);
        let h = hello("oc.e4mc.link", 0);
        let mut quiet = eng("game-tcp", "e4mc.link", "", "10.0.0.0/8");
        quiet.track = true;
        let _ = quiet.handle(&syn(&tpl), 0, now);
        assert_eq!(quiet.handle(&seg(&tpl, 1000, 1, &h), 0, now), Action::Pass);
        assert!(quiet.recent.is_empty() && quiet.stats.skipped == 0);
        let mut mine = eng("game-tcp", "", "", DST);
        let _ = mine.handle(&syn(&tpl), 0, now);
        let out = sent(mine.handle(&seg(&tpl, 1000, 1, &h), 0, now));
        assert!(payloads(&out[..8]).iter().all(|p| p == HTTP_FAKE));
    }

    #[test]
    fn discord_voice_and_stun_get_fakes() {
        let mut disc = vec![0u8; 74];
        disc[1] = 1;
        disc[3] = 70;
        let d = to_port(&packet::tests::sample_udp(&disc), 19300);
        let now = Instant::now();
        let mut e = eng("voice", "", "", "");
        let out = sent(e.handle(&d, 0, now));
        assert_eq!(out.len(), 7);
        assert!(payloads(&out[..6]).iter().all(|p| p == &vec![0xAA; 64]));
        assert!(out.iter().all(|p| packet::checksums_ok(p)));
        assert_eq!(out[6], d);
        assert_eq!(e.stats.voice, 1);
        let rtp = to_port(&packet::tests::sample_udp(&[0x80u8; 100]), 19300);
        assert_eq!(e.handle(&rtp, 0, now), Action::Pass);
        let mut off = eng("", "", "", "");
        assert_eq!(off.handle(&d, 0, now), Action::Pass);
    }

    #[test]
    fn game_udp_first_two_datagrams() {
        let d = to_port(&packet::tests::sample_udp(&[7u8; 50]), 27015);
        let now = Instant::now();
        let mut e = eng("game-udp", "", "", "142.250.1.1/32");
        for _ in 0..2 {
            let out = sent(e.handle(&d, 0, now));
            assert_eq!(out.len(), 11);
            assert!(payloads(&out[..10]).iter().all(|p| p == &vec![0xBB; 32]));
        }
        assert_eq!(e.handle(&d, 0, now), Action::Pass);
        assert_eq!(e.stats.game_udp, 2);
        let _ = e.expire(now + IDLE);
        assert_eq!(sent(e.handle(&d, 0, now + IDLE)).len(), 11);
        let mut far = eng("game-udp", "", "", "10.0.0.0/8");
        assert_eq!(far.handle(&d, 0, now), Action::Pass);
    }

    fn from_port(port: u16) -> Vec<u8> {
        let mut d = to_port(&packet::tests::sample_udp(&[7u8; 50]), 27015);
        d[20..22].copy_from_slice(&port.to_be_bytes());
        packet::fix_checksums(&mut d);
        d
    }

    fn answer(out: &[u8]) -> Vec<u8> {
        let mut r = out.to_vec();
        let (src, dst) = (r[12..16].to_vec(), r[16..20].to_vec());
        r[12..16].copy_from_slice(&dst);
        r[16..20].copy_from_slice(&src);
        let (sp, dp) = (r[20..22].to_vec(), r[22..24].to_vec());
        r[20..22].copy_from_slice(&dp);
        r[22..24].copy_from_slice(&sp);
        packet::fix_checksums(&mut r);
        r
    }

    #[test]
    fn game_udp_adapts_per_program_without_my_ip() {
        use crate::flows::{Mode, SILENT, udp_key};
        use crate::procs::{Owner, Source};
        let now = Instant::now();
        let mut e = eng("game-udp", "", "", "10.0.0.0/8")
            .with_mode(Mode::parse("adaptive").unwrap())
            .with_udp_apps(Box::new(|_, _| Some(Owner { pid: 7, name: Some("game.exe".into()), source: Source::Table })));
        e.track = true;
        let key = udp_key(Some("game.exe"));
        for i in 0..3u32 {
            let at = now + SILENT * (2 * i);
            assert_eq!(e.handle(&from_port(40001 + i as u16), 0, at), Action::Pass);
            let _ = e.expire(at + SILENT);
        }
        assert_eq!(e.stats.silent, 3);
        assert_eq!(e.udp_memory_of(&key).map(|(t, m)| (t, m.fails)), Some(("udp-fake", 3)));
        let at = now + SILENT * 10;
        let d = from_port(40010);
        let out = sent(e.handle(&d, 0, at));
        assert_eq!(out.len(), 11);
        assert!(payloads(&out[..10]).iter().all(|p| p == &vec![0xBB; 32]));
        e.observe(&answer(&d), at);
        assert_eq!(e.udp_memory_of(&key).map(|(t, m)| (t, m.ok)), Some(("udp-fake", 1)));
        assert_eq!(e.take_redeemed().len(), 3);
        assert_eq!(sent(e.handle(&d, 0, at)).len(), 11);
        assert_eq!(e.handle(&d, 0, at), Action::Pass);
        assert_eq!(e.recent.back().map(|r| (r.host.clone(), r.technique)), Some((Some(key.clone()), Some("udp-fake"))));
        assert!(e.memory_text().contains("udp:game.exe\tudp-fake\t1\t3\n"));
        let mut fresh = eng("game-udp", "", "", "10.0.0.0/8").with_mode(Mode::parse("adaptive").unwrap());
        assert_eq!(fresh.load_memory(&e.memory_text()), 1);
        assert_eq!(fresh.udp_memory_of(&key).map(|(t, _)| t), Some("udp-fake"));
        let mut fixed = eng("game-udp", "", "", "10.0.0.0/8");
        assert_eq!(fixed.handle(&from_port(40020), 0, at), Action::Pass);
        assert!(fixed.udp_memory_of(&key).is_none());
    }

    #[test]
    fn silent_relays_do_not_switch_while_others_answer() {
        use crate::flows::{Mode, SILENT, UDP_WINDOW, udp_key};
        use crate::procs::{Owner, Source};
        let now = Instant::now();
        let mut e = eng("game-udp", "", "", "10.0.0.0/8")
            .with_mode(Mode::parse("adaptive").unwrap())
            .with_udp_apps(Box::new(|_, _| Some(Owner { pid: 7, name: Some("dota2.exe".into()), source: Source::Table })));
        let key = udp_key(Some("dota2.exe"));
        let d = from_port(40100);
        assert_eq!(e.handle(&d, 0, now), Action::Pass);
        e.observe(&answer(&d), now);
        for i in 0..4u32 {
            let at = now + SILENT * (2 * i + 1);
            assert_eq!(e.handle(&from_port(40101 + i as u16), 0, at), Action::Pass);
            let _ = e.expire(at + SILENT);
        }
        assert_eq!(e.udp_memory_of(&key).map(|(t, m)| (t, m.fails)), Some(("udp-none", 4)));
        let later = now + UDP_WINDOW + SILENT * 10;
        assert_eq!(e.handle(&from_port(40200), 0, later), Action::Pass);
        let _ = e.expire(later + SILENT);
        assert_eq!(e.udp_memory_of(&key).map(|(t, _)| t), Some("udp-fake"));
        assert_eq!(sent(e.handle(&from_port(40201), 0, later + SILENT * 2)).len(), 11);
    }

    #[test]
    fn noisy_program_is_left_alone() {
        use crate::flows::{Mode, SILENT};
        let now = Instant::now();
        let mut e = eng("game-udp", "", "", "10.0.0.0/8").with_mode(Mode::parse("adaptive").unwrap());
        e.track = true;
        for i in 0..40u16 {
            assert_eq!(e.handle(&from_port(41000 + i), 0, now), Action::Pass);
        }
        let _ = e.expire(now + SILENT);
        assert_eq!((e.stats.silent, e.recent.len()), (2, 2));
        let later = now + SILENT * 2;
        for i in 0..200u16 {
            assert_eq!(e.handle(&from_port(42000 + i), 0, later), Action::Pass);
        }
        let _ = e.expire(later + SILENT);
        assert_eq!((e.stats.silent, e.recent.len()), (4, 4));
        let _ = e.expire(later + SILENT * 2);
        assert_eq!(e.handle(&from_port(45000), 0, later + SILENT * 2), Action::Pass);
        let _ = e.expire(later + SILENT * 4);
        assert_eq!(e.recent.len(), 4);
    }
}

#[test]
fn background_check_tries_next_technique_without_touching_stats() {
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let mut e = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap());
    e.track = true;
    let t0 = Instant::now();
    for port in [50021u16, 50022] {
        sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), port), 0, t0));
        let mut rst = reply(&tpl, 0x04, b"");
        rst[22..24].copy_from_slice(&port.to_be_bytes());
        packet::fix_checksums(&mut rst);
        e.observe(&rst, t0);
    }
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, _)| t), Some("sni-split"));
    assert!(e.bg_targets(t0 + Duration::from_secs(5)).is_empty());
    let later = t0 + BG_IDLE;
    let targets = e.bg_targets(later);
    assert_eq!(targets.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), vec!["www.youtube.com"]);
    assert!(e.bg_targets(later + Duration::from_secs(1)).is_empty());
    let (handled, reset) = (e.stats.handled, e.stats.reset);
    e.add_bg_port(50030);
    let probe = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50030), 0, later));
    assert!(probe.len() > 1);
    let mut ok = reply(&tpl, 0x18, &server_hello());
    ok[22..24].copy_from_slice(&50030u16.to_be_bytes());
    packet::fix_checksums(&mut ok);
    e.observe(&ok, later);
    assert_eq!((e.stats.handled, e.stats.reset, e.stats.answered), (handled, reset, 0));
    assert_eq!(e.take_redeemed().len(), 2);
    assert_eq!(e.recent.back().and_then(|d| d.app.clone()).as_deref(), Some(BG_APP));
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.ok)), Some(("sni-split", 1)));
    assert!(e.bg_targets(later + BG_EVERY * 2).is_empty());
}

#[test]
fn background_check_takes_turns_between_sites() {
    let tpl = template(true);
    let mut e = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap());
    e.track = true;
    let t0 = Instant::now();
    for (i, name) in ["a.youtube.com", "b.youtube.com", "c.youtube.com"].iter().enumerate() {
        let port = 50050 + i as u16;
        sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &hello(name, 0)), port), 0, t0));
        let mut rst = reply(&tpl, 0x04, b"");
        rst[22..24].copy_from_slice(&port.to_be_bytes());
        packet::fix_checksums(&mut rst);
        e.observe(&rst, t0);
    }
    let first: Vec<String> = e.bg_targets(t0 + BG_IDLE).into_iter().map(|t| t.0).collect();
    assert_eq!(first.len(), 2);
    let second: Vec<String> = e.bg_targets(t0 + BG_IDLE + BG_EVERY).into_iter().map(|t| t.0).collect();
    let missed = ["a.youtube.com", "b.youtube.com", "c.youtube.com"].into_iter().find(|n| !first.iter().any(|f| f == n)).unwrap();
    assert_eq!(second.first().map(String::as_str), Some(missed));
}

fn answered_flow(e: &mut Engine<u8>, tpl: &[u8], port: u16, site: &str, t0: Instant) -> u32 {
    let h = hello(site, 0);
    e.handle(&with_port(&syn(tpl), port), 0, t0);
    e.observe(&with_port_in(&syn_ack(tpl, 52), port), t0);
    sent(e.handle(&with_port(&seg(tpl, 1000, 1, &h), port), 0, t0));
    let mut ok = reply(tpl, 0x18, &server_hello());
    ok[22..24].copy_from_slice(&port.to_be_bytes());
    packet::fix_checksums(&mut ok);
    e.observe(&ok, t0);
    1000 + h.len() as u32
}

fn with_port_in(pkt: &[u8], port: u16) -> Vec<u8> {
    let mut p = pkt.to_vec();
    p[22..24].copy_from_slice(&port.to_be_bytes());
    packet::fix_checksums(&mut p);
    p
}

fn client_data(tpl: &[u8], port: u16, seq: u32, received: u32, data: &[u8]) -> Vec<u8> {
    with_ack(&with_port(&seg(tpl, seq, 7, data), port), 5001u32.wrapping_add(received))
}

#[test]
fn frozen_download_is_noticed_and_sends_the_site_to_warp() {
    use crate::warp::Shared;
    let tpl = template(true);
    let mut e = engine("rbxcdn.com", "").with_mode(Mode::parse("adaptive").unwrap()).with_warp(Shared::new(40000, vec![]));
    e.track = true;
    e.dns_alt = true;
    let t0 = Instant::now();
    let next = answered_flow(&mut e, &tpl, 50060, "tr.rbxcdn.com", t0);
    assert_eq!((e.stats.answered, e.stats.frozen), (1, 0));
    let req = b"GET /image";
    e.handle(&client_data(&tpl, 50060, next, 3000, req), 0, t0);
    let stuck = 17 * 1024;
    let more = next + req.len() as u32;
    e.handle(&client_data(&tpl, 50060, more, stuck, req), 0, t0 + Duration::from_millis(500));
    for ms in [800u64, 1600, 2900] {
        e.handle(&client_data(&tpl, 50060, more, stuck, req), 0, t0 + Duration::from_millis(500 + ms));
        assert_eq!(e.stats.frozen, 0, "рано: {ms} мс");
    }
    e.handle(&client_data(&tpl, 50060, more, stuck, req), 0, t0 + Duration::from_millis(4000));
    assert_eq!((e.stats.answered, e.stats.frozen), (0, 1));
    assert_eq!(e.stats.by_tech["general"].frozen, 1);
    assert_eq!(e.recent.back().and_then(|d| d.outcome), Some(Outcome::Frozen));
    let dst = packet::parse(&tpl).unwrap().dst;
    assert_eq!(e.avoid("tr.rbxcdn.com", t0), vec![dst]);
    assert!(e.avoid("other.rbxcdn.com", t0).is_empty());
    assert!(e.take_flush());
    assert!(!e.take_flush());
    assert!(e.notes.iter().any(|n| n.contains("заморозка: tr.rbxcdn.com") && n.contains("17 КБ") && n.contains("другой адрес")));
    assert_eq!(e.memory_of("tr.rbxcdn.com").map(|(t, _)| t), Some("general"));
    e.handle(&client_data(&tpl, 50060, more, stuck, req), 0, t0 + Duration::from_secs(6));
    assert_eq!(e.stats.frozen, 1);

    let t1 = t0 + Duration::from_secs(10);
    let next = answered_flow(&mut e, &tpl, 50061, "tr.rbxcdn.com", t1);
    e.handle(&client_data(&tpl, 50061, next, 20 * 1024, b"x"), 0, t1);
    let mut fin = client_data(&tpl, 50061, next + 1, 20 * 1024, b"");
    let ihl = ((fin[0] & 0x0f) as usize) * 4;
    fin[ihl + 13] = 0x11;
    packet::fix_checksums(&mut fin);
    e.handle(&fin, 0, t1 + Duration::from_secs(1));
    e.handle(&fin, 0, t1 + Duration::from_secs(2));
    assert_eq!(e.stats.frozen, 1);
    e.handle(&fin, 0, t1 + Duration::from_secs(5));
    assert_eq!(e.stats.frozen, 2);
    assert_eq!(e.memory_of("tr.rbxcdn.com").map(|(t, _)| t), Some("warp"));
    assert!(e.notes.iter().any(|n| n.contains("через WARP")));
}

#[test]
fn normal_download_and_lost_packet_are_not_a_freeze() {
    let tpl = template(true);
    let mut e = engine("rbxcdn.com", "").with_mode(Mode::parse("adaptive").unwrap());
    e.track = true;
    let t0 = Instant::now();
    let next = answered_flow(&mut e, &tpl, 50070, "tr.rbxcdn.com", t0);
    let req = b"GET /a";
    e.handle(&client_data(&tpl, 50070, next, 3000, req), 0, t0);
    e.handle(&client_data(&tpl, 50070, next, 15 * 1024, req), 0, t0 + Duration::from_millis(300));
    e.handle(&client_data(&tpl, 50070, next + req.len() as u32, 26 * 1024, req), 0, t0 + Duration::from_secs(4));
    e.handle(&client_data(&tpl, 50070, next + 2 * req.len() as u32, 26 * 1024, req), 0, t0 + Duration::from_secs(9));
    assert_eq!(e.stats.frozen, 0);

    let next = answered_flow(&mut e, &tpl, 50071, "tr.rbxcdn.com", t0);
    e.handle(&client_data(&tpl, 50071, next, 4000, req), 0, t0);
    e.handle(&client_data(&tpl, 50071, next, 4000, req), 0, t0 + Duration::from_secs(5));
    assert_eq!(e.stats.frozen, 0, "до 12 КБ это не заморозка");

    let next = answered_flow(&mut e, &tpl, 50072, "tr.rbxcdn.com", t0);
    e.handle(&client_data(&tpl, 50072, next, 64 * 1024, req), 0, t0);
    e.handle(&client_data(&tpl, 50072, next + req.len() as u32, 17 * 1024, req), 0, t0 + Duration::from_secs(1));
    e.handle(&client_data(&tpl, 50072, next + req.len() as u32, 17 * 1024, req), 0, t0 + Duration::from_secs(5));
    assert_eq!(e.stats.frozen, 0, "после 32 КБ соединение больше не проверяется");
    assert_eq!(e.stats.answered, 3);
}

#[test]
fn warp_step_resets_once_then_settles_through_proxy() {
    use crate::warp::Shared;
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let shared = Shared::new(40000, vec!["youtube.com".into()]);
    let mut e = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap()).with_warp(shared.clone());
    e.track = true;
    let t0 = Instant::now();
    let off = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50040), 0, t0));
    assert!(off.len() > 1);
    assert!(shared.wanted(t0));
    shared.set_listen(40400);
    shared.set_ready(true);
    let raw = with_port(&seg(&tpl, 1000, 1, &h), 50041);
    let dst = packet::parse(&raw).unwrap().dst;
    match e.handle(&raw, 0, t0) {
        Action::Reject(rst, _) => {
            let r = packet::parse(&rst).unwrap();
            assert_eq!((r.sport, r.dport, r.flags), (443, 50041, 0x14));
        }
        other => panic!("expected Reject, got {other:?}"),
    }
    assert!(shared.has_ip(&dst));
    assert!(sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50042), 0, t0)).len() > 1);
    e.warp_settle(Some("www.youtube.com".into()), 50043, dst, 443, Outcome::Answered, t0);
    assert_eq!(e.recent.back().map(|d| (d.technique, d.outcome)), Some((Some("warp"), Some(Outcome::Answered))));
    assert_eq!(e.stats.by_tech["warp"].answered, 1);
    let mut plain = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap()).with_warp(Shared::new(40000, vec![]));
    assert!(plain.memory_text().is_empty());
    assert!(plain.load_memory("www.youtube.com\twarp\t3\t16\n") == 1);
    assert_eq!(plain.memory_of("www.youtube.com").map(|(t, _)| t), Some("warp"));
    let mut fixed = engine("youtube.com", "").with_warp(Shared::new(40000, vec![]));
    assert_eq!(fixed.load_memory("www.youtube.com\twarp\t3\t16\n"), 0);
}

#[test]
fn subdomain_inherits_seeded_parent_technique() {
    let tpl = template(true);
    let h = hello("www.youtube.com", 0);
    let mut e = engine("youtube.com", "").with_mode(Mode::parse("adaptive").unwrap());
    assert_eq!(e.load_memory("youtube.com\tsni-split\t0\t0\n"), 1);
    let t0 = Instant::now();
    let out = sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &h), 50060), 0, t0));
    let (at, len) = tls::sni_range(&h).unwrap();
    assert_eq!(first_cut(&out), at + len / 2);
    let mut ok = reply(&tpl, 0x18, &server_hello());
    ok[22..24].copy_from_slice(&50060u16.to_be_bytes());
    packet::fix_checksums(&mut ok);
    e.observe(&ok, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.ok)), Some(("sni-split", 1)));
}

#[test]
fn new_subdomain_starts_from_what_works_for_its_siblings() {
    assert_eq!(crate::flows::site_base("style.sndcdn.com"), Some("sndcdn.com"));
    assert_eq!(crate::flows::site_base("kws5.lovetrue.co.uk"), Some("lovetrue.co.uk"));
    assert_eq!(crate::flows::site_base("sndcdn.com"), Some("sndcdn.com"));
    assert_eq!(crate::flows::site_base("localhost"), None);
    let tpl = template(true);
    let mut e = engine("sndcdn.com\nsoundcloud.com\nco.uk", "").with_mode(Mode::parse("adaptive").unwrap());
    let memory = "al.sndcdn.com\tsni-max\t18\t32\na-v2.sndcdn.com\tsni-split\t2\t0\napi-v2.soundcloud.com\tnone\t125\t27\nkws5.lovetrue.co.uk\tsni-max\t3\t0\n";
    assert_eq!(e.load_memory(memory), 4);
    let t0 = Instant::now();
    let fresh = |e: &mut Engine<u8>, host: &str, port: u16| dpi_first(&sent(e.handle(&with_port(&seg(&tpl, 1000, 1, &hello(host, 0)), port), 0, t0)));
    assert_eq!(fresh(&mut e, "style.sndcdn.com", 50070).as_deref(), Some("max.ru"));
    assert_eq!(fresh(&mut e, "m.soundcloud.com", 50071).as_deref(), Some("www.google.com"));
    assert_eq!(fresh(&mut e, "kws5.sadnews.co.uk", 50072).as_deref(), Some("www.google.com"));
}

#[test]
fn adaptive_can_start_from_the_picked_technique() {
    let names = |m: Mode| match m {
        Mode::Adaptive(list) => list.iter().map(|t| t.name).collect::<Vec<_>>(),
        Mode::Fixed(_) => vec![],
    };
    let plain = names(Mode::parse("adaptive").unwrap());
    let max = names(Mode::parse("adaptive:sni-max").unwrap());
    assert_eq!((max[0], max.len(), max[1]), ("sni-max", plain.len(), "general"));
    let extra = names(Mode::parse("adaptive:fakedsplit").unwrap());
    assert_eq!((extra[0], extra.len()), ("fakedsplit", plain.len() + 1));
    assert_eq!(names(Mode::parse("adaptive:").unwrap()), plain);
    assert!(Mode::parse("adaptive:nope").is_none() && Mode::parse("general:ttl").is_none());
    assert!(Mode::is_adaptive("adaptive:ttl") && Mode::is_adaptive("Adaptive") && !Mode::is_adaptive("ttl"));
    let tpl = template(true);
    let mut e = engine("youtube.com", "").with_mode(Mode::parse("adaptive:sni-max").unwrap());
    let out = sent(e.handle(&seg(&tpl, 1000, 1, &hello("www.youtube.com", 0)), 0, Instant::now()));
    assert_eq!(dpi_first(&out).as_deref(), Some("max.ru"));
}
