fn be16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]) as usize)
}

fn be24(b: &[u8], at: usize) -> Option<usize> {
    Some(((*b.get(at)? as usize) << 16) | ((*b.get(at + 1)? as usize) << 8) | *b.get(at + 2)? as usize)
}

pub fn is_client_hello(d: &[u8]) -> bool {
    d.len() >= 6 && d[0] == 0x16 && d[1] == 0x03 && d[2] <= 0x03 && d[5] == 0x01
}

pub fn record_len(d: &[u8]) -> Option<usize> {
    Some(5 + be16(d, 3)?)
}

pub fn session_id(d: &[u8]) -> Option<&[u8]> {
    let len = *d.get(43)? as usize;
    d.get(44..44 + len)
}

struct Ext {
    total_at: usize,
    at: usize,
}

fn extensions_at(d: &[u8]) -> Option<usize> {
    if !is_client_hello(d) {
        return None;
    }
    let mut p = 43;
    p += 1 + *d.get(p)? as usize;
    p += 2 + be16(d, p)?;
    p += 1 + *d.get(p)? as usize;
    be16(d, p)?;
    Some(p)
}

fn find_ext(d: &[u8], kind: usize) -> Option<Ext> {
    let total_at = extensions_at(d)?;
    let end = (total_at + 2 + be16(d, total_at)?).min(d.len());
    let mut p = total_at + 2;
    while p + 4 <= end {
        let t = be16(d, p)?;
        let len = be16(d, p + 2)?;
        if t == kind {
            return Some(Ext { total_at, at: p + 4 });
        }
        p += 4 + len;
    }
    None
}

fn host_in_sni(d: &[u8], ext: &Ext) -> Option<(usize, usize)> {
    let list = be16(d, ext.at)?;
    if list < 3 || *d.get(ext.at + 2)? != 0 {
        return None;
    }
    let len = be16(d, ext.at + 3)?;
    let at = ext.at + 5;
    d.get(at..at + len)?;
    Some((at, len))
}

pub fn sni_range(d: &[u8]) -> Option<(usize, usize)> {
    let ext = find_ext(d, 0)?;
    host_in_sni(d, &ext)
}

pub fn sni(d: &[u8]) -> Option<String> {
    let ext = find_ext(d, 0)?;
    let (at, len) = host_in_sni(d, &ext)?;
    let name = std::str::from_utf8(&d[at..at + len]).ok()?;
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    (!name.is_empty()).then_some(name)
}

pub fn with_sni(fake: &[u8], host: &str) -> Option<Vec<u8>> {
    let ext = find_ext(fake, 0)?;
    let (at, len) = host_in_sni(fake, &ext)?;
    let delta = host.len() as isize - len as isize;
    let mut out = Vec::with_capacity(fake.len() + host.len());
    out.extend_from_slice(&fake[..at]);
    out.extend_from_slice(host.as_bytes());
    out.extend_from_slice(&fake[at + len..]);
    let add16 = |b: &mut [u8], at: usize| -> Option<()> {
        let v = be16(b, at)? as isize + delta;
        let v = u16::try_from(v).ok()?;
        b[at..at + 2].copy_from_slice(&v.to_be_bytes());
        Some(())
    };
    add16(&mut out, 3)?;
    let hs = u32::try_from(be24(&out, 6)? as isize + delta).ok()?;
    out[6..9].copy_from_slice(&hs.to_be_bytes()[1..]);
    add16(&mut out, ext.total_at)?;
    add16(&mut out, ext.at - 2)?;
    add16(&mut out, ext.at)?;
    add16(&mut out, at - 2)?;
    Some(out)
}

pub fn with_padding(hello: &[u8], total: usize) -> Option<Vec<u8>> {
    if hello.len() >= total {
        return Some(hello.to_vec());
    }
    let ext_total = extensions_at(hello)?;
    let mut out = hello.to_vec();
    let grow = match find_ext(hello, 0x15) {
        Some(e) => {
            let grow = total - hello.len();
            let len = be16(&out, e.at - 2)? + grow;
            out[e.at - 2..e.at].copy_from_slice(&u16::try_from(len).ok()?.to_be_bytes());
            out.splice(e.at..e.at, std::iter::repeat_n(0u8, grow));
            grow
        }
        None => {
            let pad = total.checked_sub(hello.len() + 4)?;
            out.extend_from_slice(&[0x00, 0x15]);
            out.extend_from_slice(&u16::try_from(pad).ok()?.to_be_bytes());
            out.resize(out.len() + pad, 0);
            pad + 4
        }
    };
    let bump16 = |b: &mut [u8], at: usize| -> Option<()> {
        let v = u16::try_from(be16(b, at)? + grow).ok()?;
        b[at..at + 2].copy_from_slice(&v.to_be_bytes());
        Some(())
    };
    bump16(&mut out, 3)?;
    bump16(&mut out, ext_total)?;
    let hs = u32::try_from(be24(&out, 6)? + grow).ok()?;
    out[6..9].copy_from_slice(&hs.to_be_bytes()[1..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const DEFAULT_FAKE: &[u8] = include_bytes!("../assets/fake_tls_default.bin");

    #[test]
    fn reads_builtin_fake() {
        assert!(is_client_hello(DEFAULT_FAKE));
        assert_eq!(record_len(DEFAULT_FAKE), Some(DEFAULT_FAKE.len()));
        assert_eq!(sni(DEFAULT_FAKE).as_deref(), Some("www.microsoft.com"));
        assert_eq!(session_id(DEFAULT_FAKE).map(<[u8]>::len), Some(32));
    }

    #[test]
    fn replaces_sni_like_winws() {
        let g = with_sni(DEFAULT_FAKE, "www.google.com").unwrap();
        assert_eq!(g.len(), 677);
        assert_eq!(sni(&g).as_deref(), Some("www.google.com"));
        assert_eq!(record_len(&g), Some(g.len()));
        assert_eq!(be24(&g, 6), Some(g.len() - 9));
        assert_eq!(session_id(&g), session_id(DEFAULT_FAKE));
        let back = with_sni(&g, "www.microsoft.com").unwrap();
        assert_eq!(back, DEFAULT_FAKE);
    }

    #[test]
    fn finds_sni_position() {
        let (at, len) = sni_range(DEFAULT_FAKE).unwrap();
        assert_eq!(&DEFAULT_FAKE[at..at + len], b"www.microsoft.com");
    }

    #[test]
    fn reads_seqovl_pattern() {
        let p = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/tls_clienthello_www_google_com.bin")).unwrap();
        assert_eq!(p.len(), 681);
        assert!(is_client_hello(&p));
        assert_eq!(sni(&p).as_deref(), Some("www.google.com"));
    }

    #[test]
    fn partial_and_broken_input() {
        assert!(!is_client_hello(b"\x16\x03\x01"));
        assert_eq!(sni(&DEFAULT_FAKE[..60]), None);
        let mut bad = DEFAULT_FAKE.to_vec();
        bad[5] = 2;
        assert_eq!(sni(&bad), None);
        assert_eq!(with_sni(b"GET / HTTP/1.1", "x"), None);
    }
}
