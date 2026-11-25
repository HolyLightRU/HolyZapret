use crate::desync::{Config, FAKE_DEFAULT};
use crate::flows::{Action, Engine, Lists, Mode, Outcome, REASM_WAIT, SILENT};
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
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, _)| t), Some("none"));
    let raw = with_port(&seg(&tpl, 1000, 1, &h), 50006);
    assert_eq!(e.handle(&raw, 0, t0), Action::Pass);
    let mut ok2 = reply(&tpl, 0x18, &server_hello());
    ok2[22..24].copy_from_slice(&50006u16.to_be_bytes());
    packet::fix_checksums(&mut ok2);
    e.observe(&ok2, t0);
    assert_eq!(e.memory_of("www.youtube.com").map(|(t, m)| (t, m.ok, m.streak)), Some(("none", 2, 0)));
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
    for name in ["general", "sni-split", "random", "badseq", "md5sig", "ttl", "badsum", "disorder"] {
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
                    let fake_badseq = t.seq == 1000u32.wrapping_sub(10000);
                    let fake_md5 = p.len() > 41 && p[40] == 19;
                    !fake_badseq && !fake_md5
                })
                .cloned()
                .collect();
            assert_eq!(server(&kept, 1000, Some(0x1000)), h, "{name} pad {pad}");
            assert!(dpi_first(&out).is_some_and(|s| s != "rr1.googlevideo.com"), "{name} pad {pad}");
        }
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
}
