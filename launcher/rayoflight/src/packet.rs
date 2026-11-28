use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamps {
    pub val: u32,
    pub ecr: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tcp<'a> {
    pub raw: &'a [u8],
    pub v6: bool,
    pub ip_len: usize,
    pub tcp_len: usize,
    pub src: IpAddr,
    pub dst: IpAddr,
    pub sport: u16,
    pub dport: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub x2: u8,
    pub win: u16,
    pub ts: Option<Timestamps>,
    pub ttl: u8,
    pub ip_id: u16,
    pub df: bool,
    pub tos: u8,
    pub flow: u32,
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl<'a> Tcp<'a> {
    pub fn payload(&self) -> &'a [u8] {
        &self.raw[self.ip_len + self.tcp_len..]
    }

    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

fn find_ts(opts: &[u8]) -> Option<Timestamps> {
    let mut i = 0;
    while i < opts.len() {
        match opts[i] {
            0 => return None,
            1 => i += 1,
            kind => {
                let len = *opts.get(i + 1)? as usize;
                if len < 2 || i + len > opts.len() {
                    return None;
                }
                if kind == 8 && len == 10 {
                    return Some(Timestamps { val: be32(opts, i + 2), ecr: be32(opts, i + 6) });
                }
                i += len;
            }
        }
    }
    None
}

pub fn parse(raw: &[u8]) -> Option<Tcp<'_>> {
    let first = *raw.first()?;
    let (v6, ip_len, total, src, dst, ttl, ip_id, df, tos, flow) = match first >> 4 {
        4 => {
            if raw.len() < 20 {
                return None;
            }
            let ihl = (first & 0x0F) as usize * 4;
            let total = be16(raw, 2) as usize;
            if ihl < 20 || total < ihl || total > raw.len() || raw[9] != 6 {
                return None;
            }
            let frag = be16(raw, 6);
            if frag & 0x2000 != 0 || frag & 0x1FFF != 0 {
                return None;
            }
            let src = IpAddr::V4(Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15]));
            let dst = IpAddr::V4(Ipv4Addr::new(raw[16], raw[17], raw[18], raw[19]));
            (false, ihl, total, src, dst, raw[8], be16(raw, 4), frag & 0x4000 != 0, raw[1], 0)
        }
        6 => {
            if raw.len() < 40 || raw[6] != 6 {
                return None;
            }
            let total = 40 + be16(raw, 4) as usize;
            if total > raw.len() {
                return None;
            }
            let mut s = [0u8; 16];
            let mut d = [0u8; 16];
            s.copy_from_slice(&raw[8..24]);
            d.copy_from_slice(&raw[24..40]);
            (true, 40, total, IpAddr::V6(Ipv6Addr::from(s)), IpAddr::V6(Ipv6Addr::from(d)), raw[7], 0, false, 0, be32(raw, 0))
        }
        _ => return None,
    };
    let raw = &raw[..total];
    if total < ip_len + 20 {
        return None;
    }
    let t = &raw[ip_len..];
    let tcp_len = (t[12] >> 4) as usize * 4;
    if tcp_len < 20 || ip_len + tcp_len > total {
        return None;
    }
    Some(Tcp {
        raw,
        v6,
        ip_len,
        tcp_len,
        src,
        dst,
        sport: be16(t, 0),
        dport: be16(t, 2),
        seq: be32(t, 4),
        ack: be32(t, 8),
        flags: t[13],
        x2: t[12] & 0x0F,
        win: be16(t, 14),
        ts: find_ts(&t[20..tcp_len]),
        ttl,
        ip_id,
        df,
        tos,
        flow,
    })
}

fn sum16(data: &[u8], mut acc: u32) -> u32 {
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        acc += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        acc += (*last as u32) << 8;
    }
    acc
}

fn fold(mut acc: u32) -> u16 {
    while acc >> 16 != 0 {
        acc = (acc & 0xFFFF) + (acc >> 16);
    }
    !(acc as u16)
}

pub fn fix_checksums(pkt: &mut [u8]) {
    let v6 = pkt[0] >> 4 == 6;
    let ip_len = if v6 { 40 } else { (pkt[0] & 0x0F) as usize * 4 };
    let proto = if v6 { pkt[6] } else { pkt[9] };
    let at = if proto == 17 { 6 } else { 16 };
    let l4_len = pkt.len() - ip_len;
    let mut acc = if v6 {
        sum16(&pkt[8..40], 0) + proto as u32 + l4_len as u32
    } else {
        pkt[10] = 0;
        pkt[11] = 0;
        let c = fold(sum16(&pkt[..ip_len], 0));
        pkt[10..12].copy_from_slice(&c.to_be_bytes());
        sum16(&pkt[12..20], 0) + proto as u32 + l4_len as u32
    };
    pkt[ip_len + at] = 0;
    pkt[ip_len + at + 1] = 0;
    acc = sum16(&pkt[ip_len..], acc);
    let c = match fold(acc) {
        0 if proto == 17 => 0xFFFF,
        c => c,
    };
    pkt[ip_len + at..ip_len + at + 2].copy_from_slice(&c.to_be_bytes());
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Udp<'a> {
    pub raw: &'a [u8],
    pub v6: bool,
    pub ip_len: usize,
    pub src: IpAddr,
    pub dst: IpAddr,
    pub sport: u16,
    pub dport: u16,
    pub ip_id: u16,
}

impl<'a> Udp<'a> {
    pub fn payload(&self) -> &'a [u8] {
        &self.raw[self.ip_len + 8..]
    }
}

pub fn is_udp(raw: &[u8]) -> bool {
    match raw.first().map(|b| b >> 4) {
        Some(4) => raw.len() >= 20 && raw[9] == 17,
        Some(6) => raw.len() >= 40 && raw[6] == 17,
        _ => false,
    }
}

pub fn parse_udp(raw: &[u8]) -> Option<Udp<'_>> {
    if !is_udp(raw) {
        return None;
    }
    let (v6, ip_len, total, src, dst, ip_id) = if raw[0] >> 4 == 4 {
        let ihl = (raw[0] & 0x0F) as usize * 4;
        let total = be16(raw, 2) as usize;
        let frag = be16(raw, 6);
        if ihl < 20 || total < ihl + 8 || total > raw.len() || frag & 0x2000 != 0 || frag & 0x1FFF != 0 {
            return None;
        }
        (false, ihl, total, IpAddr::V4(Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15])), IpAddr::V4(Ipv4Addr::new(raw[16], raw[17], raw[18], raw[19])), be16(raw, 4))
    } else {
        let total = 40 + be16(raw, 4) as usize;
        if total > raw.len() || total < 48 {
            return None;
        }
        let mut s = [0u8; 16];
        let mut d = [0u8; 16];
        s.copy_from_slice(&raw[8..24]);
        d.copy_from_slice(&raw[24..40]);
        (true, 40, total, IpAddr::V6(Ipv6Addr::from(s)), IpAddr::V6(Ipv6Addr::from(d)), 0)
    };
    let raw = &raw[..total];
    Some(Udp { raw, v6, ip_len, src, dst, sport: be16(raw, ip_len), dport: be16(raw, ip_len + 2), ip_id })
}

pub fn udp_like(orig: &Udp, payload: &[u8]) -> Vec<u8> {
    let mut p = orig.raw[..orig.ip_len + 8].to_vec();
    p.extend_from_slice(payload);
    let udp_len = (8 + payload.len()) as u16;
    if orig.v6 {
        p[4..6].copy_from_slice(&udp_len.to_be_bytes());
    } else {
        let total = p.len() as u16;
        p[2..4].copy_from_slice(&total.to_be_bytes());
    }
    let at = orig.ip_len;
    p[at + 4..at + 6].copy_from_slice(&udp_len.to_be_bytes());
    fix_checksums(&mut p);
    p
}

pub fn checksums_ok(pkt: &[u8]) -> bool {
    let mut copy = pkt.to_vec();
    fix_checksums(&mut copy);
    copy == pkt
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    pub md5: Option<[u8; 16]>,
    pub ts: Option<Timestamps>,
}

fn options(o: &Opts) -> Vec<u8> {
    let mut v = Vec::new();
    if let Some(m) = o.md5 {
        v.extend_from_slice(&[19, 18]);
        v.extend_from_slice(&m);
    }
    if let Some(ts) = o.ts {
        v.extend_from_slice(&[8, 10]);
        v.extend_from_slice(&ts.val.to_be_bytes());
        v.extend_from_slice(&ts.ecr.to_be_bytes());
    }
    while v.len() % 4 != 0 {
        v.push(1);
    }
    v
}

pub fn segment(orig: &Tcp, seq: u32, ts: Option<Timestamps>, ip_id: u16, payload: &[u8]) -> Vec<u8> {
    segment_ex(orig, seq, orig.ack, &Opts { md5: None, ts }, ip_id, payload)
}

pub fn segment_ex(orig: &Tcp, seq: u32, ack: u32, opts: &Opts, ip_id: u16, payload: &[u8]) -> Vec<u8> {
    let opt = options(opts);
    let tcp_len = 20 + opt.len();
    let ip_len = if orig.v6 { 40 } else { 20 };
    let mut p = vec![0u8; ip_len + tcp_len + payload.len()];
    match (orig.src, orig.dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            p[0] = 0x45;
            p[1] = orig.tos;
            let total = p.len() as u16;
            p[2..4].copy_from_slice(&total.to_be_bytes());
            p[4..6].copy_from_slice(&ip_id.to_be_bytes());
            if orig.df {
                p[6] = 0x40;
            }
            p[8] = orig.ttl;
            p[9] = 6;
            p[12..16].copy_from_slice(&s.octets());
            p[16..20].copy_from_slice(&d.octets());
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            p[0..4].copy_from_slice(&orig.flow.to_be_bytes());
            p[4..6].copy_from_slice(&((tcp_len + payload.len()) as u16).to_be_bytes());
            p[6] = 6;
            p[7] = orig.ttl;
            p[8..24].copy_from_slice(&s.octets());
            p[24..40].copy_from_slice(&d.octets());
        }
        _ => unreachable!(),
    }
    let t = &mut p[ip_len..];
    t[0..2].copy_from_slice(&orig.sport.to_be_bytes());
    t[2..4].copy_from_slice(&orig.dport.to_be_bytes());
    t[4..8].copy_from_slice(&seq.to_be_bytes());
    t[8..12].copy_from_slice(&ack.to_be_bytes());
    t[12] = ((tcp_len / 4) as u8) << 4 | orig.x2;
    t[13] = orig.flags;
    t[14..16].copy_from_slice(&orig.win.to_be_bytes());
    t[20..tcp_len].copy_from_slice(&opt);
    t[tcp_len..].copy_from_slice(payload);
    fix_checksums(&mut p);
    p
}

pub fn fixed(raw: &[u8]) -> Vec<u8> {
    let mut p = raw.to_vec();
    fix_checksums(&mut p);
    p
}

pub fn set_ttl(p: &mut [u8], ttl: u8) {
    match p.first().map(|b| b >> 4) {
        Some(4) if p.len() >= 20 => {
            p[8] = ttl;
            fix_checksums(p);
        }
        Some(6) if p.len() >= 40 => p[7] = ttl,
        _ => {}
    }
}

pub fn wscale_in(opts: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i < opts.len() {
        match opts[i] {
            0 => return None,
            1 => i += 1,
            kind => {
                let len = *opts.get(i + 1)? as usize;
                if len < 2 || i + len > opts.len() {
                    return None;
                }
                if kind == 3 && len == 3 {
                    return Some(opts[i + 2].min(14));
                }
                i += len;
            }
        }
    }
    None
}

pub fn wscale(t: &Tcp) -> Option<u8> {
    wscale_in(t.raw.get(t.ip_len + 20..t.ip_len + t.tcp_len)?)
}

pub fn with_window(raw: &[u8], win: u16) -> Vec<u8> {
    let mut p = raw.to_vec();
    let off = match p.first().map(|b| b >> 4) {
        Some(4) => (p[0] & 0x0f) as usize * 4,
        Some(6) => 40,
        _ => return p,
    };
    if p.len() >= off + 20 {
        p[off + 14..off + 16].copy_from_slice(&win.to_be_bytes());
        fix_checksums(&mut p);
    }
    p
}

pub fn set_flags(p: &mut [u8], flags: u8) {
    let off = match p.first().map(|b| b >> 4) {
        Some(4) => (p[0] & 0x0f) as usize * 4,
        Some(6) => 40,
        _ => return,
    };
    if p.len() >= off + 20 {
        p[off + 13] = flags;
        fix_checksums(p);
    }
}

pub fn break_tcp_sum(p: &mut [u8]) {
    let off = match p.first().map(|b| b >> 4) {
        Some(4) => (p[0] & 0x0f) as usize * 4,
        Some(6) => 40,
        _ => return,
    };
    if p.len() >= off + 18 {
        p[off + 16] ^= 0x5a;
    }
}

pub fn hops(ttl: u8) -> u8 {
    let initial: u8 = if ttl <= 64 {
        64
    } else if ttl <= 128 {
        128
    } else {
        255
    };
    initial - ttl
}

pub fn with_ip_id(raw: &[u8], ip_id: u16) -> Vec<u8> {
    let mut p = raw.to_vec();
    if p.first().is_some_and(|b| b >> 4 == 4) && p.len() >= 20 {
        p[4..6].copy_from_slice(&ip_id.to_be_bytes());
        fix_checksums(&mut p);
    }
    p
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn window_is_rewritten_and_scale_is_read_from_options() {
        assert_eq!(wscale_in(&[2, 4, 5, 180, 1, 3, 3, 8, 1, 1, 4, 2]), Some(8));
        assert_eq!(wscale_in(&[1, 1, 8, 10, 0, 0, 0, 1, 0, 0, 0, 2]), None);
        assert_eq!(wscale_in(&[3, 9, 1]), None);
        assert_eq!(wscale_in(&[]), None);
        let raw = sample_v4(b"hello", true);
        assert_eq!((parse(&raw).unwrap().win, wscale(&parse(&raw).unwrap())), (512, None));
        let small = with_window(&raw, 1);
        let t = parse(&small).unwrap();
        assert_eq!((t.win, t.payload(), t.seq), (1, &b"hello"[..], 1000));
        assert!(checksums_ok(&small));
    }

    pub fn sample_udp(payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 28 + payload.len()];
        p[0] = 0x45;
        let total = p.len() as u16;
        p[2..4].copy_from_slice(&total.to_be_bytes());
        p[4..6].copy_from_slice(&0x2222u16.to_be_bytes());
        p[8] = 128;
        p[9] = 17;
        p[12..16].copy_from_slice(&[192, 168, 1, 10]);
        p[16..20].copy_from_slice(&[142, 250, 1, 1]);
        p[20..22].copy_from_slice(&50000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        p[28..].copy_from_slice(payload);
        fix_checksums(&mut p);
        p
    }

    #[test]
    fn udp_parse_and_rebuild() {
        let p = sample_udp(b"hello quic");
        assert!(is_udp(&p) && parse(&p).is_none());
        let u = parse_udp(&p).unwrap();
        assert_eq!((u.sport, u.dport, u.payload(), u.ip_id), (50000, 443, &b"hello quic"[..], 0x2222));
        assert!(checksums_ok(&p));
        let q = udp_like(&u, b"longer payload here");
        let u2 = parse_udp(&q).unwrap();
        assert_eq!((u2.payload(), u2.src, u2.dst), (&b"longer payload here"[..], u.src, u.dst));
        assert!(checksums_ok(&q));
        assert_ne!(&q[26..28], &[0, 0]);
        assert!(parse_udp(&sample_v4(b"x", true)).is_none());
    }

    #[test]
    fn ttl_and_checksum_helpers() {
        assert_eq!((hops(52), hops(116), hops(250), hops(64)), (12, 12, 5, 0));
        let mut p = sample_v4(b"hello", true);
        set_ttl(&mut p, 9);
        assert_eq!(parse(&p).unwrap().ttl, 9);
        assert!(checksums_ok(&p));
        break_tcp_sum(&mut p);
        assert!(!checksums_ok(&p));
        assert!(checksums_ok(&fixed(&p)));
    }

    pub fn sample_v4(payload: &[u8], ts: bool) -> Vec<u8> {
        let opts: &[u8] = if ts { &[1, 1, 8, 10, 0, 0, 0x10, 0, 0, 0, 0x20, 0] } else { &[] };
        let tcp_len = 20 + opts.len();
        let mut p = vec![0u8; 20 + tcp_len + payload.len()];
        p[0] = 0x45;
        let total = p.len() as u16;
        p[2..4].copy_from_slice(&total.to_be_bytes());
        p[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
        p[6] = 0x40;
        p[8] = 128;
        p[9] = 6;
        p[12..16].copy_from_slice(&[192, 168, 1, 10]);
        p[16..20].copy_from_slice(&[142, 250, 74, 110]);
        let t = &mut p[20..];
        t[0..2].copy_from_slice(&50000u16.to_be_bytes());
        t[2..4].copy_from_slice(&443u16.to_be_bytes());
        t[4..8].copy_from_slice(&1000u32.to_be_bytes());
        t[8..12].copy_from_slice(&7777u32.to_be_bytes());
        t[12] = ((tcp_len / 4) as u8) << 4;
        t[13] = 0x18;
        t[14..16].copy_from_slice(&512u16.to_be_bytes());
        t[20..tcp_len].copy_from_slice(opts);
        t[tcp_len..].copy_from_slice(payload);
        fix_checksums(&mut p);
        p
    }

    #[test]
    fn parses_ipv4_tcp_with_timestamps() {
        let raw = sample_v4(b"hello", true);
        let t = parse(&raw).unwrap();
        assert_eq!((t.sport, t.dport, t.seq, t.ack, t.win, t.flags), (50000, 443, 1000, 7777, 512, 0x18));
        assert_eq!(t.ts, Some(Timestamps { val: 0x1000, ecr: 0x2000 }));
        assert_eq!((t.ttl, t.ip_id, t.df), (128, 0x1234, true));
        assert_eq!(t.payload(), b"hello");
        assert!(checksums_ok(&raw));
        assert_eq!(parse(&sample_v4(b"x", false)).unwrap().ts, None);
    }

    #[test]
    fn rejects_fragments_and_garbage() {
        let mut raw = sample_v4(b"x", false);
        raw[6] = 0x20;
        assert!(parse(&raw).is_none());
        assert!(parse(&[0x45, 0, 0]).is_none());
        let mut udp = sample_v4(b"x", false);
        udp[9] = 17;
        assert!(parse(&udp).is_none());
    }

    #[test]
    fn builds_segment_like_winws() {
        let raw = sample_v4(b"0123456789", true);
        let o = parse(&raw).unwrap();
        let ts = o.ts.map(|t| Timestamps { val: t.val.wrapping_add(-600000i32 as u32), ecr: t.ecr });
        let p = segment(&o, 999, ts, 0x1235, b"abc");
        let s = parse(&p).unwrap();
        assert_eq!((s.seq, s.ack, s.win, s.flags, s.ip_id, s.ttl, s.df), (999, 7777, 512, 0x18, 0x1235, 128, true));
        assert_eq!(s.ts.unwrap().val, 0x1000u32.wrapping_sub(600000));
        assert_eq!(&p[40..52], &[8, 10, 0xFF, 0xF6, 0xE8, 0x40, 0, 0, 0x20, 0, 1, 1]);
        assert_eq!(s.payload(), b"abc");
        assert!(checksums_ok(&p));
        assert_eq!(parse(&with_ip_id(&raw, 7)).unwrap().ip_id, 7);
        assert!(checksums_ok(&with_ip_id(&raw, 7)));
    }

    #[test]
    fn md5_and_timestamps_like_winws() {
        let raw = sample_v4(b"0123456789", true);
        let o = parse(&raw).unwrap();
        let p = segment_ex(&o, 5, 9, &Opts { md5: Some([7; 16]), ts: o.ts }, 1, b"x");
        let s = parse(&p).unwrap();
        assert_eq!((s.seq, s.ack, s.tcp_len, s.ts), (5, 9, 48, o.ts));
        assert_eq!(&p[40..42], &[19, 18]);
        assert_eq!(&p[58..60], &[8, 10]);
        assert!(checksums_ok(&p));
        let only = segment_ex(&o, 5, 9, &Opts { md5: Some([7; 16]), ts: None }, 1, b"x");
        assert_eq!(parse(&only).unwrap().tcp_len, 40);
        assert_eq!(&only[58..60], &[1, 1]);
    }

    #[test]
    fn known_checksum() {
        let mut ip = [0x45u8, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0, 0, 0xc0, 0xa8, 0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7];
        let c = fold(sum16(&ip, 0));
        ip[10..12].copy_from_slice(&c.to_be_bytes());
        assert_eq!(c, 0xb861);
    }
}
