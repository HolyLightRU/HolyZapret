use super::flowseal;
use super::gamefilter::GameFilter;
use super::paths::Paths;
use std::fs;
use std::path::{Path, PathBuf};

const ENGINES: [&str; 2] = ["holyzapret.exe", "winws.exe"];

#[derive(Clone, Debug)]
pub struct Strategy {
    pub id: String,
    pub category: String,
    pub name: String,
    pub path: PathBuf,
    pub engine: PathBuf,
    pub bin: PathBuf,
    pub lists: PathBuf,
    pub lists_label: String,
    pub blocks: usize,
    pub game_filter: bool,
}

impl Strategy {
    pub fn matches(&self, stored: &str) -> bool {
        self.id.eq_ignore_ascii_case(stored) || self.name.eq_ignore_ascii_case(stored)
    }
}

fn is_service_file(name: &str) -> bool {
    let l = name.to_lowercase();
    l.starts_with("service") || l == "holy_host_system.bat" || l == "manager.bat"
}

pub fn discover(paths: &Paths) -> Vec<Strategy> {
    let mut out = Vec::new();
    collect(paths, &paths.root, "Основные", &mut out);
    if let Ok(rd) = fs::read_dir(&paths.root) {
        let mut dirs: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for d in dirs {
            let name = d.file_name().to_string_lossy().into_owned();
            if matches!(name.as_str(), "bin" | "lists" | "exp-list" | "utils" | "launcher" | "target" | "flowseal.tmp")
                || name.starts_with('.')
            {
                continue;
            }
            let category = match flowseal::version_in(&d.path()) {
                Some(v) if name == flowseal::DIR => format!("Flowseal {v}"),
                _ => name,
            };
            collect(paths, &d.path(), &category, &mut out);
        }
    }
    out
}

fn engine_pos(lower: &str) -> Option<(usize, usize)> {
    ENGINES.iter().filter_map(|e| lower.find(e).map(|i| (i, e.len()))).min_by_key(|(i, _)| *i)
}

fn collect(paths: &Paths, dir: &Path, category: &str, out: &mut Vec<Strategy>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut files: Vec<_> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bat")))
        .collect();
    files.sort_by_key(|e| e.file_name());
    let own_engine = dir != paths.root.as_path() && dir.join("bin").join("winws.exe").is_file();
    for f in files {
        let file = f.file_name().to_string_lossy().into_owned();
        if is_service_file(&file) {
            continue;
        }
        let Ok(text) = fs::read_to_string(f.path()) else { continue };
        let lower = text.to_lowercase();
        if engine_pos(&lower).is_none() {
            continue;
        }
        let name = file.trim_end_matches(".bat").trim_end_matches(".BAT").to_string();
        let (engine, bin, lists) = if own_engine {
            (dir.join("bin").join("winws.exe"), dir.join("bin"), dir.join("lists"))
        } else if text.contains("exp-list") {
            (paths.engine(), paths.bin(), paths.exp_lists())
        } else {
            (paths.engine(), paths.bin(), paths.lists())
        };
        let rel = f.path().strip_prefix(&paths.root).map(|p| p.to_path_buf()).unwrap_or_else(|_| f.path());
        let id = rel.with_extension("").to_string_lossy().replace('/', "\\");
        let lists_label = lists
            .strip_prefix(&paths.root)
            .map(|p| p.to_string_lossy().replace('/', "\\"))
            .unwrap_or_else(|_| lists.display().to_string());
        out.push(Strategy {
            id,
            category: category.to_string(),
            name,
            path: f.path(),
            engine,
            bin,
            lists,
            lists_label,
            blocks: text.matches("--new").count() + 1,
            game_filter: lower.contains("%gamefilter"),
        });
    }
}

pub fn extract_args(strategy: &Strategy, _paths: &Paths, gf: &GameFilter) -> anyhow::Result<String> {
    let text = fs::read_to_string(&strategy.path)?;
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

    #[test]
    fn real_strategies_have_no_carets_left() {
        let root = std::path::PathBuf::from("..").canonicalize().unwrap();
        if !root.join("bin").join("HolyZapret.exe").exists() {
            return;
        }
        let p = Paths { root };
        let all = discover(&p);
        assert!(!all.is_empty());
        for s in &all {
            let args = extract_args(s, &p, &GameFilter::all()).unwrap();
            assert!(!args.contains('^'), "{}: {args}", s.name);
            assert!(!args.contains('%'), "{}: {args}", s.name);
        }
    }
}
