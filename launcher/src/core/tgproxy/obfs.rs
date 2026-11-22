use aes::Aes256;
use ctr::Ctr128BE;
use ctr::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha256};

pub type Cipher = Ctr128BE<Aes256>;

pub const INIT_LEN: usize = 64;
pub const TAG_ABRIDGED: [u8; 4] = [0xef; 4];
pub const TAG_INTERMEDIATE: [u8; 4] = [0xee; 4];
pub const TAG_SECURE: [u8; 4] = [0xdd; 4];

const RESERVED_STARTS: [&[u8; 4]; 7] = [b"HEAD", b"POST", b"GET ", &[0xee; 4], &[0xdd; 4], &[0x16, 0x03, 0x01, 0x02], b"OPTI"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Abridged,
    Intermediate,
    Secure,
}

impl Proto {
    pub fn tag(self) -> [u8; 4] {
        match self {
            Proto::Abridged => TAG_ABRIDGED,
            Proto::Intermediate => TAG_INTERMEDIATE,
            Proto::Secure => TAG_SECURE,
        }
    }

    fn from_tag(t: &[u8]) -> Option<Proto> {
        match t {
            [0xef, 0xef, 0xef, 0xef] => Some(Proto::Abridged),
            [0xee, 0xee, 0xee, 0xee] => Some(Proto::Intermediate),
            [0xdd, 0xdd, 0xdd, 0xdd] => Some(Proto::Secure),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Hello {
    pub dc: u16,
    pub media: bool,
    pub test: bool,
    pub proto: Proto,
    prekey_iv: [u8; 48],
}

impl Hello {
    pub fn dc_idx(&self) -> i16 {
        let dc = self.dc as i16;
        if self.media { -dc } else { dc }
    }
}

pub struct Ciphers {
    pub clt_dec: Cipher,
    pub clt_enc: Cipher,
    pub tg_enc: Cipher,
    pub tg_dec: Cipher,
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("system rng");
    b
}

fn hashed(prekey: &[u8], secret: &[u8; 16]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prekey);
    h.update(secret);
    h.finalize().into()
}

fn cipher(key: &[u8], iv: &[u8]) -> Cipher {
    Cipher::new(key.into(), iv.into())
}

fn skip(c: &mut Cipher, n: usize) {
    let mut z = vec![0u8; n];
    c.apply_keystream(&mut z);
}

pub fn parse_hello(init: &[u8; INIT_LEN], secret: &[u8; 16]) -> Option<Hello> {
    let key = hashed(&init[8..40], secret);
    let mut dec = *init;
    cipher(&key, &init[40..56]).apply_keystream(&mut dec);
    let proto = Proto::from_tag(&dec[56..60])?;
    let idx = i16::from_le_bytes([dec[60], dec[61]]) as i32;
    let mut dc = idx.unsigned_abs();
    let test = dc >= 10000;
    if test {
        dc -= 10000;
    }
    if dc == 0 || dc > u16::MAX as u32 {
        return None;
    }
    let mut prekey_iv = [0u8; 48];
    prekey_iv.copy_from_slice(&init[8..56]);
    Some(Hello { dc: dc as u16, media: idx < 0, test, proto, prekey_iv })
}

fn acceptable(rnd: &[u8; INIT_LEN]) -> bool {
    rnd[0] != 0xef && !RESERVED_STARTS.iter().any(|s| &rnd[..4] == *s) && rnd[4..8] != [0, 0, 0, 0]
}

pub fn obfuscated_init(proto: Proto, dc_idx: i16, key_of: impl Fn(&[u8]) -> [u8; 32]) -> [u8; INIT_LEN] {
    let rnd = loop {
        let r = random::<INIT_LEN>();
        if acceptable(&r) {
            break r;
        }
    };
    let key = key_of(&rnd[8..40]);
    let mut stream = rnd;
    cipher(&key, &rnd[40..56]).apply_keystream(&mut stream);
    let tail_rnd = random::<2>();
    let dc = dc_idx.to_le_bytes();
    let tag = proto.tag();
    let plain = [tag[0], tag[1], tag[2], tag[3], dc[0], dc[1], tail_rnd[0], tail_rnd[1]];
    let mut out = rnd;
    for i in 0..8 {
        out[56 + i] = plain[i] ^ (stream[56 + i] ^ rnd[56 + i]);
    }
    out
}

pub fn relay_init(proto: Proto, dc_idx: i16) -> [u8; INIT_LEN] {
    obfuscated_init(proto, dc_idx, |k| {
        let mut a = [0u8; 32];
        a.copy_from_slice(k);
        a
    })
}

fn reversed(b: &[u8]) -> Vec<u8> {
    b.iter().rev().copied().collect()
}

pub fn ciphers(hello: &Hello, secret: &[u8; 16], relay: &[u8; INIT_LEN]) -> Ciphers {
    let mut clt_dec = cipher(&hashed(&hello.prekey_iv[..32], secret), &hello.prekey_iv[32..]);
    skip(&mut clt_dec, INIT_LEN);
    let rev = reversed(&hello.prekey_iv);
    let clt_enc = cipher(&hashed(&rev[..32], secret), &rev[32..]);
    let mut tg_enc = cipher(&relay[8..40], &relay[40..56]);
    skip(&mut tg_enc, INIT_LEN);
    let rrev = reversed(&relay[8..56]);
    let tg_dec = cipher(&rrev[..32], &rrev[32..]);
    Ciphers { clt_dec, clt_enc, tg_enc, tg_dec }
}

pub fn apply(c: &mut Cipher, data: &mut [u8]) {
    c.apply_keystream(data);
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub struct Peer {
        pub enc: Cipher,
        pub dec: Cipher,
    }

    pub fn client(secret: &[u8; 16], proto: Proto, dc_idx: i16) -> ([u8; INIT_LEN], Peer) {
        let init = obfuscated_init(proto, dc_idx, |k| hashed(k, secret));
        let mut enc = cipher(&hashed(&init[8..40], secret), &init[40..56]);
        skip(&mut enc, INIT_LEN);
        let rev = reversed(&init[8..56]);
        let dec = cipher(&hashed(&rev[..32], secret), &rev[32..]);
        (init, Peer { enc, dec })
    }

    pub fn telegram(relay: &[u8; INIT_LEN]) -> (Proto, i16, Peer) {
        let mut dec = cipher(&relay[8..40], &relay[40..56]);
        let mut plain = *relay;
        dec.apply_keystream(&mut plain);
        let proto = Proto::from_tag(&plain[56..60]).unwrap();
        let dc = i16::from_le_bytes([plain[60], plain[61]]);
        let rev = reversed(&relay[8..56]);
        let enc = cipher(&rev[..32], &rev[32..]);
        (proto, dc, Peer { enc, dec })
    }

    #[test]
    fn hello_and_relay_roundtrip() {
        let secret = random::<16>();
        for (proto, idx) in [(Proto::Secure, 2i16), (Proto::Abridged, -4), (Proto::Intermediate, 10002), (Proto::Secure, -10001)] {
            let (init, mut cl) = client(&secret, proto, idx);
            let hello = parse_hello(&init, &secret).unwrap();
            assert_eq!(hello.proto, proto);
            assert_eq!(hello.media, idx < 0);
            assert_eq!(hello.test, idx.unsigned_abs() >= 10000);
            assert_eq!(hello.dc as i32, (idx.unsigned_abs() as i32) % 10000);
            let relay = relay_init(hello.proto, hello.dc_idx());
            let mut cs = ciphers(&hello, &secret, &relay);
            let (tproto, tdc, mut tg) = telegram(&relay);
            assert_eq!((tproto, tdc), (proto, hello.dc_idx()));

            let msg = b"hello telegram, this is a longer payload to cross block boundaries".to_vec();
            let mut wire = msg.clone();
            cl.enc.apply_keystream(&mut wire);
            apply(&mut cs.clt_dec, &mut wire);
            apply(&mut cs.tg_enc, &mut wire);
            tg.dec.apply_keystream(&mut wire);
            assert_eq!(wire, msg);

            let reply = b"reply from the data center".to_vec();
            let mut wire = reply.clone();
            tg.enc.apply_keystream(&mut wire);
            apply(&mut cs.tg_dec, &mut wire);
            apply(&mut cs.clt_enc, &mut wire);
            cl.dec.apply_keystream(&mut wire);
            assert_eq!(wire, reply);
        }
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let (init, _) = client(&[1; 16], Proto::Secure, 2);
        let misses = (0..20).filter(|_| parse_hello(&init, &random::<16>()).is_none()).count();
        assert_eq!(misses, 20);
    }

    #[test]
    fn relay_init_avoids_reserved_prefixes() {
        for _ in 0..500 {
            let r = relay_init(Proto::Abridged, 2);
            assert!(acceptable(&r));
        }
    }
}
