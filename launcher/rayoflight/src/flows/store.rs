use super::{Memory, site_base};
use std::collections::{HashMap, HashSet};

pub const LIMIT: usize = 16384;

#[derive(Default)]
pub struct Store {
    map: HashMap<String, (Memory, u64)>,
    bases: HashMap<String, HashSet<String>>,
    tick: u64,
    limit: usize,
}

impl Store {
    pub fn new() -> Store {
        Store::with_limit(LIMIT)
    }

    pub fn with_limit(limit: usize) -> Store {
        Store { limit: limit.max(8), ..Default::default() }
    }

    pub fn get(&self, host: &str) -> Option<&Memory> {
        self.map.get(host).map(|e| &e.0)
    }

    pub fn get_mut(&mut self, host: &str) -> Option<&mut Memory> {
        self.map.get_mut(host).map(|e| &mut e.0)
    }

    pub fn by_age(&self) -> Vec<(&String, &Memory)> {
        let mut rows: Vec<(&String, &(Memory, u64))> = self.map.iter().collect();
        rows.sort_by(|a, b| a.1.1.cmp(&b.1.1).then_with(|| a.0.cmp(b.0)));
        rows.into_iter().map(|(h, e)| (h, &e.0)).collect()
    }

    pub fn siblings<'a>(&'a self, base: &str) -> impl Iterator<Item = (&'a String, &'a Memory)> + 'a {
        self.bases.get(base).into_iter().flatten().filter_map(move |h| self.map.get(h).map(|e| (h, &e.0)))
    }

    pub fn insert(&mut self, host: &str, m: Memory) {
        *self.entry_or(host, || m) = m;
    }

    pub fn entry_or(&mut self, host: &str, init: impl FnOnce() -> Memory) -> &mut Memory {
        self.tick += 1;
        let tick = self.tick;
        if !self.map.contains_key(host) {
            if self.map.len() >= self.limit {
                self.evict();
            }
            if let Some(b) = site_base(host) {
                self.bases.entry(b.to_string()).or_default().insert(host.to_string());
            }
            self.map.insert(host.to_string(), (init(), tick));
        }
        let e = self.map.get_mut(host).expect("just inserted");
        e.1 = tick;
        &mut e.0
    }

    fn evict(&mut self) {
        let mut ages: Vec<(u64, String)> = self.map.iter().map(|(h, e)| (e.1, h.clone())).collect();
        ages.sort_unstable();
        for (_, h) in ages.into_iter().take((self.limit / 8).max(1)) {
            self.map.remove(&h);
            if let Some(b) = site_base(&h) {
                if let Some(set) = self.bases.get_mut(b) {
                    set.remove(&h);
                    if set.is_empty() {
                        self.bases.remove(b);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Store {
        fn contains_key(&self, host: &str) -> bool {
            self.map.contains_key(host)
        }

        fn len(&self) -> usize {
            self.map.len()
        }
    }

    fn m(idx: usize) -> Memory {
        Memory { idx, ..Default::default() }
    }

    #[test]
    fn full_store_drops_the_least_recently_used_and_keeps_learning() {
        let mut s = Store::with_limit(16);
        for i in 0..16 {
            s.insert(&format!("s{i}.example.com"), m(1));
        }
        s.entry_or("s0.example.com", || m(9)).ok += 1;
        s.insert("new.example.org", m(3));
        assert_eq!(s.len(), 15);
        assert!(s.contains_key("s0.example.com"));
        assert!(!s.contains_key("s1.example.com") && !s.contains_key("s2.example.com"));
        assert_eq!(s.get("new.example.org").map(|m| m.idx), Some(3));
        assert_eq!(s.get("s0.example.com").map(|m| (m.idx, m.ok)), Some((1, 1)));
        let ages: Vec<&str> = s.by_age().into_iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(ages.last(), Some(&"new.example.org"));
        let sib: Vec<&str> = s.siblings("example.com").map(|(h, _)| h.as_str()).collect();
        assert_eq!(sib.len(), 14);
        assert!(!sib.contains(&"s1.example.com"));
        assert_eq!(s.siblings("example.org").count(), 1);
    }
}
