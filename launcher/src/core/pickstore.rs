use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Score {
    pub ok: usize,
    pub total: usize,
    pub frozen: usize,
    pub ms: u32,
    pub failed: bool,
}

impl Score {
    pub fn key(&self) -> (bool, std::cmp::Reverse<usize>, usize, u32) {
        (self.failed, std::cmp::Reverse(self.ok), self.frozen, self.ms)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastPick {
    pub when: String,
    pub scores: HashMap<PathBuf, Score>,
}

fn file() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("HolyZapret").join("last-pick.tsv"))
}

pub fn to_text(p: &LastPick) -> String {
    let mut lines = vec![format!("# {}", p.when)];
    let mut rows: Vec<_> = p.scores.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    for (path, s) in rows {
        lines.push(format!("{}\t{}\t{}\t{}\t{}\t{}", path.display(), s.ok, s.total, s.frozen, s.ms, s.failed as u8));
    }
    lines.join("\r\n") + "\r\n"
}

pub fn parse(text: &str) -> LastPick {
    let mut p = LastPick::default();
    for line in text.lines() {
        if let Some(w) = line.strip_prefix("# ") {
            p.when = w.trim().to_string();
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 6 {
            continue;
        }
        let n = |i: usize| f[i].trim().parse::<usize>().ok();
        if let (Some(ok), Some(total), Some(frozen), Some(ms)) = (n(1), n(2), n(3), n(4)) {
            p.scores.insert(PathBuf::from(f[0]), Score { ok, total, frozen, ms: ms as u32, failed: f[5].trim() == "1" });
        }
    }
    p
}

pub fn load() -> Option<LastPick> {
    let text = std::fs::read_to_string(file()?).ok()?;
    let p = parse(&text);
    (!p.scores.is_empty()).then_some(p)
}

pub fn save(p: &LastPick) -> anyhow::Result<()> {
    let f = file().ok_or_else(|| anyhow::anyhow!("нет LOCALAPPDATA"))?;
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(f, to_text(p))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_order() {
        let mut p = LastPick { when: "21.11 22:40".into(), ..Default::default() };
        p.scores.insert(PathBuf::from(r"C:\hz\general.bat"), Score { ok: 33, total: 33, frozen: 0, ms: 500, failed: false });
        p.scores.insert(PathBuf::from(r"C:\hz\b.bat"), Score { ok: 30, total: 33, frozen: 2, ms: 300, failed: false });
        p.scores.insert(PathBuf::from(r"C:\hz\c.bat"), Score { ok: 0, total: 33, frozen: 0, ms: 0, failed: true });
        let back = parse(&to_text(&p));
        assert_eq!(back, p);
        let mut v: Vec<_> = back.scores.values().collect();
        v.sort_by_key(|s| s.key());
        assert_eq!((v[0].ok, v[2].failed), (33, true));
    }
}
