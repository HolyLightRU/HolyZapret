use super::paths::Paths;
use super::picker::{self, CheckKind, Target};
use std::collections::HashSet;

pub const INTERNET: &str = "Google";
const WATCHED: [&str; 2] = ["YouTube", "Discord"];
const STRIKES: u32 = 2;

pub fn targets(paths: &Paths) -> Vec<Target> {
    picker::load_targets(paths)
        .into_iter()
        .filter(|t| t.kind == CheckKind::Tls12 && (t.category == INTERNET || WATCHED.contains(&t.category.as_str())))
        .collect()
}

pub fn probe(targets: &[Target]) -> Vec<(String, bool)> {
    let checks: Vec<bool> = std::thread::scope(|s| {
        let jobs: Vec<_> = targets.iter().map(|t| s.spawn(move || picker::run_check(t).ok)).collect();
        jobs.into_iter().map(|j| j.join().unwrap_or(false)).collect()
    });
    let mut out: Vec<(String, bool)> = Vec::new();
    for (t, ok) in targets.iter().zip(checks) {
        match out.iter_mut().find(|(c, _)| *c == t.category) {
            Some((_, any)) => *any |= ok,
            None => out.push((t.category.clone(), ok)),
        }
    }
    out
}

#[derive(Default)]
pub struct Watch {
    seen_ok: HashSet<String>,
    strikes: u32,
    alerted: Vec<String>,
}

impl Watch {
    pub fn current(&self) -> Vec<String> {
        self.alerted.clone()
    }

    pub fn step(&mut self, results: &[(String, bool)]) -> Option<Vec<String>> {
        let online = results.iter().any(|(c, ok)| c == INTERNET && *ok);
        if !online {
            self.strikes = 0;
            return None;
        }
        for (c, ok) in results {
            if *ok {
                self.seen_ok.insert(c.clone());
            }
        }
        let failing: Vec<String> = results
            .iter()
            .filter(|(c, ok)| c != INTERNET && !ok && self.seen_ok.contains(c))
            .map(|(c, _)| c.clone())
            .collect();
        if failing.is_empty() {
            self.strikes = 0;
            if self.alerted.is_empty() {
                return None;
            }
            self.alerted.clear();
            return Some(Vec::new());
        }
        self.strikes += 1;
        if self.strikes >= STRIKES && failing != self.alerted {
            self.alerted = failing.clone();
            return Some(failing);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(yt: bool, dc: bool, g: bool) -> Vec<(String, bool)> {
        vec![("YouTube".into(), yt), ("Discord".into(), dc), ("Google".into(), g)]
    }

    #[test]
    fn alerts_after_two_failures_of_working_service() {
        let mut w = Watch::default();
        assert_eq!(w.step(&r(true, false, true)), None);
        assert_eq!(w.step(&r(false, false, true)), None);
        assert_eq!(w.step(&r(false, false, true)), Some(vec!["YouTube".to_string()]));
        assert_eq!(w.step(&r(false, false, true)), None);
        assert_eq!(w.step(&r(false, false, false)), None);
        assert_eq!(w.step(&r(true, false, true)), Some(vec![]));
        assert_eq!(w.step(&r(true, false, true)), None);
    }

    #[test]
    fn single_blip_is_ignored() {
        let mut w = Watch::default();
        w.step(&r(true, true, true));
        assert_eq!(w.step(&r(false, true, true)), None);
        assert_eq!(w.step(&r(true, true, true)), None);
        assert_eq!(w.step(&r(false, true, true)), None);
    }
}
