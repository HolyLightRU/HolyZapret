use aes::Aes128;
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use sha2::{Digest, Sha256};

pub const V1: u32 = 1;
pub const MIN_DATAGRAM: usize = 1200;
const SALT_V1: [u8; 20] = [0x38, 0x76, 0x2c, 0xf7, 0xf5, 0x59, 0x34, 0xb3, 0x4d, 0x17, 0x9a, 0xe6, 0xa4, 0xc8, 0x0c, 0xad, 0xcc, 0xbb, 0x7f, 0x0a];
const TAG: usize = 16;

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    for p in parts {
        inner.update(p);
    }
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn expand(prk: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut prev: Vec<u8> = Vec::new();
    let mut i = 1u8;
    while out.len() < len {
        let t = hmac(prk, &[&prev, info, &[i]]);
        out.extend_from_slice(&t);
        prev = t.to_vec();
        i += 1;
    }
    out.truncate(len);
    out
}

fn expand_label(secret: &[u8], label: &str, len: usize) -> Vec<u8> {
    let full = format!("tls13 {label}");
    let mut info = Vec::with_capacity(4 + full.len());
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push(full.len() as u8);
    info.extend_from_slice(full.as_bytes());
    info.push(0);
    expand(secret, &info, len)
}

#[derive(Clone)]
pub struct Keys {
    aead: Aes128,
    iv: [u8; 12],
    hp: Aes128,
    h: u128,
}

impl Keys {
    pub fn client(dcid: &[u8]) -> Keys {
        Keys::derive(dcid, "client in")
    }

    pub fn server(dcid: &[u8]) -> Keys {
        Keys::derive(dcid, "server in")
    }

    fn derive(dcid: &[u8], side: &str) -> Keys {
        let initial = hmac(&SALT_V1, &[dcid]);
        let secret = expand_label(&initial, side, 32);
        let key = expand_label(&secret, "quic key", 16);
        let iv = expand_label(&secret, "quic iv", 12);
        let hp = expand_label(&secret, "quic hp", 16);
        let aead = Aes128::new(GenericArray::from_slice(&key));
        let h = u128::from_be_bytes(block(&aead, [0u8; 16]));
        let mut ivb = [0u8; 12];
        ivb.copy_from_slice(&iv);
        Keys { aead, iv: ivb, hp: Aes128::new(GenericArray::from_slice(&hp)), h }
    }

    fn nonce(&self, pn: u64) -> [u8; 12] {
        let mut n = self.iv;
        for (i, b) in pn.to_be_bytes().iter().enumerate() {
            n[4 + i] ^= b;
        }
        n
    }

    fn mask(&self, sample: &[u8]) -> [u8; 16] {
        let mut s = [0u8; 16];
        s.copy_from_slice(&sample[..16]);
        block(&self.hp, s)
    }
}

fn block(c: &Aes128, b: [u8; 16]) -> [u8; 16] {
    let mut g = GenericArray::from(b);
    c.encrypt_block(&mut g);
    g.into()
}

fn gmul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if (x >> (127 - i)) & 1 == 1 {
            z ^= v;
        }
        v = if v & 1 == 1 { (v >> 1) ^ (0xE1u128 << 120) } else { v >> 1 };
    }
    z
}

fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> u128 {
    let mut y = 0u128;
    for data in [aad, ct] {
        for chunk in data.chunks(16) {
            let mut b = [0u8; 16];
            b[..chunk.len()].copy_from_slice(chunk);
            y = gmul(y ^ u128::from_be_bytes(b), h);
        }
    }
    let lens = ((aad.len() as u128 * 8) << 64) | (ct.len() as u128 * 8);
    gmul(y ^ lens, h)
}

fn ctr(k: &Keys, nonce: &[u8; 12], data: &mut [u8]) {
    let mut counter = [0u8; 16];
    counter[..12].copy_from_slice(nonce);
    for (i, chunk) in data.chunks_mut(16).enumerate() {
        counter[12..].copy_from_slice(&(i as u32 + 2).to_be_bytes());
        let ks = block(&k.aead, counter);
        for (b, s) in chunk.iter_mut().zip(ks) {
            *b ^= s;
        }
    }
}

fn tag(k: &Keys, nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;
    (u128::from_be_bytes(block(&k.aead, j0)) ^ ghash(k.h, aad, ct)).to_be_bytes()
}

pub fn seal(k: &Keys, pn: u64, aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let nonce = k.nonce(pn);
    let mut out = plain.to_vec();
    ctr(k, &nonce, &mut out);
    let t = tag(k, &nonce, aad, &out);
    out.extend_from_slice(&t);
    out
}

pub fn open(k: &Keys, pn: u64, aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < TAG {
        return None;
    }
    let (ct, t) = sealed.split_at(sealed.len() - TAG);
    let nonce = k.nonce(pn);
    if tag(k, &nonce, aad, ct) != t {
        return None;
    }
    let mut out = ct.to_vec();
    ctr(k, &nonce, &mut out);
    Some(out)
}

pub fn read_varint(d: &[u8], at: &mut usize) -> Option<u64> {
    let first = *d.get(*at)?;
    let len = 1usize << (first >> 6);
    let bytes = d.get(*at..*at + len)?;
    let mut v = (first & 0x3f) as u64;
    for b in &bytes[1..] {
        v = (v << 8) | *b as u64;
    }
    *at += len;
    Some(v)
}

pub fn write_varint(out: &mut Vec<u8>, v: u64) {
    match v {
        0..=63 => out.push(v as u8),
        64..=16383 => out.extend_from_slice(&(0x4000u16 | v as u16).to_be_bytes()),
        16384..=1_073_741_823 => out.extend_from_slice(&(0x8000_0000u32 | v as u32).to_be_bytes()),
        _ => out.extend_from_slice(&(0xC000_0000_0000_0000u64 | v).to_be_bytes()),
    }
}

pub fn is_initial(d: &[u8]) -> bool {
    d.len() >= 7 && d[0] & 0xF0 == 0xC0 && u32::from_be_bytes([d[1], d[2], d[3], d[4]]) == V1
}

pub fn is_long_header(d: &[u8]) -> bool {
    d.first().is_some_and(|b| b & 0x80 != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Initial {
    pub dcid: Vec<u8>,
    pub scid: Vec<u8>,
    pub token: Vec<u8>,
    pub pn: u64,
    pub frames: Vec<u8>,
    pub size: usize,
}

pub fn long_dcid(d: &[u8]) -> Option<&[u8]> {
    if !is_long_header(d) {
        return None;
    }
    let dl = *d.get(5)? as usize;
    if dl > 20 {
        return None;
    }
    d.get(6..6 + dl)
}

pub fn open_initial(d: &[u8]) -> Option<Initial> {
    if !is_initial(d) {
        return None;
    }
    let mut p = 5;
    let dl = *d.get(p)? as usize;
    if dl > 20 {
        return None;
    }
    let dcid = d.get(p + 1..p + 1 + dl)?.to_vec();
    p += 1 + dl;
    let sl = *d.get(p)? as usize;
    if sl > 20 {
        return None;
    }
    let scid = d.get(p + 1..p + 1 + sl)?.to_vec();
    p += 1 + sl;
    let tl = read_varint(d, &mut p)? as usize;
    let token = d.get(p..p + tl)?.to_vec();
    p += tl;
    let len = read_varint(d, &mut p)? as usize;
    let pn_at = p;
    let end = pn_at.checked_add(len)?;
    if end > d.len() || len < 20 {
        return None;
    }
    let k = Keys::client(&dcid);
    let mask = k.mask(d.get(pn_at + 4..pn_at + 20)?);
    let mut head = d[..pn_at + 4].to_vec();
    head[0] ^= mask[0] & 0x0f;
    let pn_len = (head[0] & 3) as usize + 1;
    head.truncate(pn_at + pn_len);
    let mut pn = 0u64;
    for i in 0..pn_len {
        head[pn_at + i] ^= mask[1 + i];
        pn = (pn << 8) | head[pn_at + i] as u64;
    }
    let frames = open(&k, pn, &head, &d[pn_at + pn_len..end])?;
    Some(Initial { dcid, scid, token, pn, frames, size: end })
}

pub fn header_len(dcid: &[u8], scid: &[u8], token: &[u8]) -> usize {
    let mut v = Vec::new();
    write_varint(&mut v, token.len() as u64);
    7 + dcid.len() + scid.len() + v.len() + token.len()
}

pub fn seal_initial(dcid: &[u8], scid: &[u8], token: &[u8], pn: u64, frames: &[u8], min: usize) -> Vec<u8> {
    let pn_len = 4usize;
    let mut head = vec![0xC0 | (pn_len as u8 - 1)];
    head.extend_from_slice(&V1.to_be_bytes());
    head.push(dcid.len() as u8);
    head.extend_from_slice(dcid);
    head.push(scid.len() as u8);
    head.extend_from_slice(scid);
    write_varint(&mut head, token.len() as u64);
    head.extend_from_slice(token);
    let fixed = header_len(dcid, scid, token) + 2 + pn_len + TAG;
    let mut plain = frames.to_vec();
    let want = min.saturating_sub(fixed).max(20);
    if plain.len() < want {
        plain.resize(want, 0);
    }
    let len = pn_len + plain.len() + TAG;
    head.extend_from_slice(&(0x4000u16 | len as u16).to_be_bytes());
    let pn_at = head.len();
    head.extend_from_slice(&(pn as u32).to_be_bytes());
    let k = Keys::client(dcid);
    let sealed = seal(&k, pn, &head, &plain);
    let mut out = head;
    out.extend_from_slice(&sealed);
    let mask = k.mask(&out[pn_at + 4..pn_at + 20]);
    out[0] ^= mask[0] & 0x0f;
    for i in 0..pn_len {
        out[pn_at + i] ^= mask[1 + i];
    }
    out
}

pub fn crypto_chunks(frames: &[u8]) -> Option<Vec<(u64, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < frames.len() {
        let kind = read_varint(frames, &mut p)?;
        match kind {
            0x00 | 0x01 => {}
            0x02 | 0x03 => {
                read_varint(frames, &mut p)?;
                read_varint(frames, &mut p)?;
                let ranges = read_varint(frames, &mut p)?;
                read_varint(frames, &mut p)?;
                for _ in 0..ranges {
                    read_varint(frames, &mut p)?;
                    read_varint(frames, &mut p)?;
                }
                if kind == 0x03 {
                    for _ in 0..3 {
                        read_varint(frames, &mut p)?;
                    }
                }
            }
            0x06 => {
                let off = read_varint(frames, &mut p)?;
                let len = read_varint(frames, &mut p)? as usize;
                out.push((off, frames.get(p..p + len)?.to_vec()));
                p += len;
            }
            _ => return None,
        }
    }
    Some(out)
}

pub fn assemble(chunks: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut buf: Vec<Option<u8>> = Vec::new();
    for (off, data) in chunks {
        let at = *off as usize;
        if at > 65536 {
            continue;
        }
        if buf.len() < at + data.len() {
            buf.resize(at + data.len(), None);
        }
        for (i, b) in data.iter().enumerate() {
            buf[at + i].get_or_insert(*b);
        }
    }
    buf.into_iter().map_while(|b| b).collect()
}

pub fn hello_len(stream: &[u8]) -> Option<usize> {
    (stream.len() >= 4 && stream[0] == 1).then(|| 4 + ((stream[1] as usize) << 16 | (stream[2] as usize) << 8 | stream[3] as usize))
}

pub fn as_record(hello: &[u8]) -> Vec<u8> {
    let mut r = vec![0x16, 0x03, 0x01];
    r.extend_from_slice(&(hello.len() as u16).to_be_bytes());
    r.extend_from_slice(hello);
    r
}

pub fn sni(hello: &[u8]) -> Option<String> {
    crate::tls::sni(&as_record(hello))
}

pub fn with_sni(hello: &[u8], host: &str) -> Option<Vec<u8>> {
    crate::tls::with_sni(&as_record(hello), host).map(|r| r[5..].to_vec())
}

pub fn crypto_frame(out: &mut Vec<u8>, off: u64, data: &[u8]) {
    out.push(0x06);
    write_varint(out, off);
    write_varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

pub fn hello_of(d: &[u8]) -> Option<(Initial, Vec<u8>)> {
    let i = open_initial(d)?;
    let stream = assemble(&crypto_chunks(&i.frames)?);
    let need = hello_len(&stream)?;
    let hello = stream.get(..need)?.to_vec();
    Some((i, hello))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn google() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bin/quic_initial_www_google_com.bin")).unwrap()
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn hkdf_matches_rfc5869_case1() {
        let ikm = [0x0bu8; 22];
        let salt = hex("000102030405060708090a0b0c");
        let info = hex("f0f1f2f3f4f5f6f7f8f9");
        let prk = hmac(&salt, &[&ikm]);
        assert_eq!(prk.to_vec(), hex("077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"));
        let okm = expand(&prk, &info, 42);
        assert_eq!(okm, hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"));
    }

    #[test]
    fn gcm_matches_nist_case3() {
        let key = hex("feffe9928665731c6d6a8f9467308308");
        let iv = hex("cafebabefacedbaddecaf888");
        let pt = hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255");
        let aead = Aes128::new(GenericArray::from_slice(&key));
        let h = u128::from_be_bytes(block(&aead, [0u8; 16]));
        let mut ivb = [0u8; 12];
        ivb.copy_from_slice(&iv);
        let k = Keys { aead: aead.clone(), iv: ivb, hp: aead, h };
        let out = seal(&k, 0, &[], &pt);
        assert_eq!(out[..pt.len()].to_vec(), hex("42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091473f5985"));
        assert_eq!(out[pt.len()..].to_vec(), hex("4d5c2af327cd64a62cf35abd2ba6fab4"));
        assert_eq!(open(&k, 0, &[], &out), Some(pt));
    }

    #[test]
    fn opens_chrome_initial_for_google() {
        let g = google();
        let (i, hello) = hello_of(&g).unwrap();
        assert_eq!(i.dcid.len(), 8);
        assert_eq!(i.size, g.len());
        assert_eq!(sni(&hello).as_deref(), Some("www.google.com"));
        let mut broken = g.clone();
        broken[100] ^= 1;
        assert!(open_initial(&broken).is_none());
    }

    #[test]
    fn seal_then_open_round_trip_with_new_name() {
        let (i, hello) = hello_of(&google()).unwrap();
        let renamed = with_sni(&hello, "www.youtube.com").unwrap();
        assert_eq!(sni(&renamed).as_deref(), Some("www.youtube.com"));
        let mut frames = Vec::new();
        crypto_frame(&mut frames, 0, &renamed);
        let dcid = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let pkt = seal_initial(&dcid, &i.scid, &[], 3, &frames, MIN_DATAGRAM);
        assert_eq!(pkt.len(), MIN_DATAGRAM);
        let (back, h2) = hello_of(&pkt).unwrap();
        assert_eq!((back.pn, back.dcid.as_slice(), h2), (3, &dcid[..], renamed));
    }

    #[test]
    fn varints_and_chunks() {
        for v in [0u64, 63, 64, 16383, 16384, 1_073_741_823, 1_073_741_824] {
            let mut b = Vec::new();
            write_varint(&mut b, v);
            let mut p = 0;
            assert_eq!(read_varint(&b, &mut p), Some(v));
            assert_eq!(p, b.len());
        }
        let mut f = vec![0x01, 0x00];
        crypto_frame(&mut f, 5, b"world");
        f.extend_from_slice(&[0x02, 0x01, 0x00, 0x00, 0x00]);
        crypto_frame(&mut f, 0, b"hello");
        let chunks = crypto_chunks(&f).unwrap();
        assert_eq!(assemble(&chunks), b"helloworld");
        assert!(crypto_chunks(&[0x1f]).is_none());
    }
}
