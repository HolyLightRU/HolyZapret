use super::obfs::Proto;

pub struct Splitter {
    proto: Proto,
    cipher: Vec<u8>,
    plain: Vec<u8>,
    disabled: bool,
}

impl Splitter {
    pub fn new(proto: Proto) -> Splitter {
        Splitter { proto, cipher: Vec::new(), plain: Vec::new(), disabled: false }
    }

    pub fn split(&mut self, cipher: &[u8], plain: &[u8]) -> Vec<Vec<u8>> {
        if cipher.is_empty() {
            return Vec::new();
        }
        if self.disabled {
            return vec![cipher.to_vec()];
        }
        self.cipher.extend_from_slice(cipher);
        self.plain.extend_from_slice(plain);
        let mut parts = Vec::new();
        let mut off = 0;
        let total = self.cipher.len();
        while off < total {
            match self.next_len(off, total - off) {
                None => break,
                Some(0) => {
                    parts.push(self.cipher[off..].to_vec());
                    off = total;
                    self.disabled = true;
                }
                Some(n) => {
                    parts.push(self.cipher[off..off + n].to_vec());
                    off += n;
                }
            }
        }
        self.cipher.drain(..off);
        self.plain.drain(..off);
        parts
    }

    pub fn flush(&mut self) -> Option<Vec<u8>> {
        if self.cipher.is_empty() {
            return None;
        }
        self.plain.clear();
        Some(std::mem::take(&mut self.cipher))
    }

    fn next_len(&self, off: usize, avail: usize) -> Option<usize> {
        let p = &self.plain[off..];
        let (header, payload) = match self.proto {
            Proto::Abridged => {
                let first = p[0];
                if first == 0x7f || first == 0xff {
                    if avail < 4 {
                        return None;
                    }
                    (4, u32::from_le_bytes([p[1], p[2], p[3], 0]) as usize * 4)
                } else {
                    (1, (first & 0x7f) as usize * 4)
                }
            }
            Proto::Intermediate | Proto::Secure => {
                if avail < 4 {
                    return None;
                }
                (4, (u32::from_le_bytes([p[0], p[1], p[2], p[3]]) & 0x7fff_ffff) as usize)
            }
        };
        if payload == 0 {
            return Some(0);
        }
        if avail < header + payload { None } else { Some(header + payload) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(s: &mut Splitter, data: &[u8]) -> Vec<Vec<u8>> {
        let cipher: Vec<u8> = data.iter().map(|b| b ^ 0x5a).collect();
        s.split(&cipher, data).into_iter().map(|p| p.iter().map(|b| b ^ 0x5a).collect()).collect()
    }

    #[test]
    fn intermediate_packets_are_cut_on_boundaries() {
        let mut s = Splitter::new(Proto::Intermediate);
        let mut stream = Vec::new();
        for n in [8u32, 12, 4] {
            stream.extend_from_slice(&n.to_le_bytes());
            stream.extend(std::iter::repeat_n(n as u8, n as usize));
        }
        let a = feed(&mut s, &stream[..10]);
        assert!(a.is_empty());
        let b = feed(&mut s, &stream[10..]);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].len(), 12);
        assert_eq!(b[1].len(), 16);
        assert_eq!(b[2].len(), 8);
        assert!(s.flush().is_none());
    }

    #[test]
    fn abridged_short_and_long_headers() {
        let mut s = Splitter::new(Proto::Abridged);
        let mut stream = vec![2u8];
        stream.extend([1u8; 8]);
        stream.extend([0x7f, 0x80, 0, 0]);
        stream.extend(vec![2u8; 0x80 * 4]);
        let parts = feed(&mut s, &stream);
        assert_eq!(parts.iter().map(Vec::len).collect::<Vec<_>>(), vec![9, 4 + 512]);
    }

    #[test]
    fn zero_length_disables_splitting() {
        let mut s = Splitter::new(Proto::Secure);
        let parts = feed(&mut s, &[0, 0, 0, 0, 9, 9]);
        assert_eq!(parts, vec![vec![0, 0, 0, 0, 9, 9]]);
        assert_eq!(feed(&mut s, &[1, 2, 3]), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn flush_returns_partial_tail() {
        let mut s = Splitter::new(Proto::Intermediate);
        assert!(feed(&mut s, &[16, 0, 0, 0, 1, 2]).is_empty());
        assert_eq!(s.flush().map(|v| v.len()), Some(6));
    }
}
