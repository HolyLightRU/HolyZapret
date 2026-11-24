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

pub fn idle_secs() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut li) == 0 {
            return 0;
        }
        (GetTickCount().wrapping_sub(li.dwTime) / 1000) as u64
    }
}

pub fn auto_pick(
    paths: &super::paths::Paths,
    current: Option<&super::strategy::Strategy>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<super::picker::Choice> {
    use super::picker::{self, Event};
    let all = super::strategy::discover(paths);
    let mut idx = picker::optimal(paths, &all);
    if let Some(c) = current {
        if let Some(i) = all.iter().position(|s| s.path == c.path) {
            if !idx.contains(&i) {
                idx.push(i);
            }
        }
    }
    if idx.is_empty() {
        anyhow::bail!("нет стратегий для подбора");
    }
    let list: Vec<_> = idx.into_iter().map(|i| all[i].clone()).collect();
    let targets = picker::load_targets(paths);
    let gf = super::gamefilter::load(paths);
    let results = std::sync::Mutex::new(Vec::new());
    let end = std::sync::Mutex::new(None);
    picker::run(paths.clone(), list, targets.clone(), gf, Vec::new(), cancel, |e| match e {
        Event::Done(o) => results.lock().unwrap().push(o),
        Event::Finished { cancelled, restore } => *end.lock().unwrap() = Some((cancelled, restore)),
        Event::Failed(e) => *end.lock().unwrap() = Some((true, Some(Err(e)))),
        _ => {}
    });
    match end.into_inner().unwrap() {
        Some((false, Some(Err(e)))) => anyhow::bail!("подбор прошёл, но службу не удалось запустить: {e}"),
        Some((false, _)) => {}
        Some((true, Some(Err(e)))) => anyhow::bail!("{e}"),
        _ => anyhow::bail!("подбор отменён: компьютером снова пользуются"),
    }
    Ok(picker::choose(&targets, &results.into_inner().unwrap(), current.map(|c| c.path.as_path())))
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
