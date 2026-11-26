use super::gamefilter::GameFilter;
use super::paths::Paths;
use std::fs;
use std::path::PathBuf;

const ENGINES: [&str; 2] = ["holyzapret.exe", "winws.exe"];

#[derive(Clone, Debug)]
pub struct Strategy {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub engine: PathBuf,
    pub bin: PathBuf,
    pub lists: PathBuf,
    pub ray: Option<(String, PathBuf)>,
}

impl Strategy {
    pub fn source(&self) -> &std::path::Path {
        self.ray.as_ref().map(|r| r.1.as_path()).unwrap_or(&self.path)
    }
}

impl Strategy {
    pub fn matches(&self, stored: &str) -> bool {
        let hit = |s: &str| self.id.eq_ignore_ascii_case(s) || self.name.eq_ignore_ascii_case(s);
        hit(stored) || super::names::repair(stored).is_some_and(|f| hit(&f)) || (self.id == super::ray::DEFAULT_ID && !stored.to_lowercase().starts_with("rayoflight:"))
    }
}

pub const SCRIPTS: &str = "scripts";

pub fn scripts_dir(paths: &Paths) -> PathBuf {
    paths.root.join(SCRIPTS)
}

pub fn discover(paths: &Paths) -> Vec<Strategy> {
    super::ray::variants(paths)
}

fn engine_pos(lower: &str) -> Option<(usize, usize)> {
    ENGINES.iter().filter_map(|e| lower.find(e).map(|i| (i, e.len()))).min_by_key(|(i, _)| *i)
}

pub fn extract_args(strategy: &Strategy, _paths: &Paths, gf: &GameFilter) -> anyhow::Result<String> {
    let text = fs::read_to_string(strategy.source())?;
    let bin = format!("{}\\", strategy.bin.display());
    let lists = format!("{}\\", strategy.lists.display());

    let mut args = String::new();
    let mut capture = false;
    for raw in text.lines() {
        let mut line = raw.trim().to_string();
        if line.is_empty() || line.to_lowercase().starts_with("rem ") || line.starts_with("::") {
            continue;
        }
        if !capture {
            let Some((idx, len)) = engine_pos(&line.to_lowercase()) else { continue };
            line = line[idx + len..].trim_start_matches('"').to_string();
            capture = true;
        }
        let continued = line.ends_with('^');
        let line = line.trim_end_matches('^').trim();
        args.push(' ');
        args.push_str(line);
        if !continued {
            break;
        }
    }
    if args.trim().is_empty() {
        anyhow::bail!("в {} не найден запуск движка", strategy.path.display());
    }
    let args = unescape_cmd(&args);
    let args = replace_ci(&args, "%BIN%", &bin);
    let args = replace_ci(&args, "%LISTS%", &lists);
    let args = apply_game_filter(&args, gf);
    Ok(super::lists::inject_user_lists(args.trim(), &lists))
}

pub fn apply_game_filter(args: &str, gf: &GameFilter) -> String {
    args.split(' ')
        .map(|tok| {
            let l = tok.to_lowercase();
            if !l.contains("%gamefilter") {
                return tok.to_string();
            }
            let udp = l.starts_with("--wf-udp") || l.starts_with("--filter-udp");
            let t = replace_ci(tok, "%GameFilterTCP%", gf.tcp_value());
            let t = replace_ci(&t, "%GameFilterUDP%", gf.udp_value());
            replace_ci(&t, "%GameFilter%", if udp { gf.udp_value() } else { gf.tcp_value() })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn unescape_cmd(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut quoted = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                out.push(c);
            }
            '^' if !quoted => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

pub fn lists_dir(strategy: &Strategy, _paths: &Paths) -> PathBuf {
    strategy.lists.clone()
}

fn replace_ci(s: &str, pat: &str, with: &str) -> String {
    let (ls, lp) = (s.to_lowercase(), pat.to_lowercase());
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = ls[i..].find(&lp) {
        out.push_str(&s[i..i + p]);
        out.push_str(with);
        i += p + pat.len();
    }
    out.push_str(&s[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_filter_follows_protocol() {
        use crate::core::gamefilter::Mode;
        let gf = GameFilter { mode: Mode::All, tcp: "1000-2000".into(), udp: "3000-4000".into() };
        let a = "--wf-tcp=80,443,%GameFilter% --wf-udp=443,%GameFilter% --filter-tcp=%GameFilter% --new --filter-udp=%GameFilter% --x=%GameFilterUDP%";
        assert_eq!(
            apply_game_filter(a, &gf),
            "--wf-tcp=80,443,1000-2000 --wf-udp=443,3000-4000 --filter-tcp=1000-2000 --new --filter-udp=3000-4000 --x=3000-4000"
        );
        let tcp_only = GameFilter { mode: Mode::Tcp, ..gf };
        assert_eq!(apply_game_filter("--wf-udp=443,%GameFilterUDP% --wf-tcp=%GameFilterTCP%", &tcp_only), "--wf-udp=443,12 --wf-tcp=1000-2000");
    }

    #[test]
    fn cmd_carets_are_unescaped_outside_quotes() {
        assert_eq!(unescape_cmd("--dpi-desync-fake-tls=^! --x=1"), "--dpi-desync-fake-tls=! --x=1");
        assert_eq!(unescape_cmd(r#"--a="C:/p^q" --b=^^"#), r#"--a="C:/p^q" --b=^"#);
    }

}
