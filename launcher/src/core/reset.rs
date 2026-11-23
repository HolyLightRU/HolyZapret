use super::lists::{self, USER_FILES};
use super::paths::Paths;
use std::fs;
use std::path::PathBuf;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};

const KEY: &str = r"Software\HolyZapret";
const KEEP: [&str; 1] = ["Window"];

pub fn list_dirs(paths: &Paths) -> Vec<PathBuf> {
    let mut dirs = vec![paths.lists(), paths.exp_lists()];
    for s in super::strategy::discover(paths) {
        if !dirs.contains(&s.lists) {
            dirs.push(s.lists);
        }
    }
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

fn clear_registry() {
    let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_ALL_ACCESS) else { return };
    let names: Vec<String> = k.enum_values().flatten().map(|(n, _)| n).filter(|n| !KEEP.contains(&n.as_str())).collect();
    for n in names {
        let _ = k.delete_value(n);
    }
}

pub fn clear_lists(dirs: &[PathBuf]) -> anyhow::Result<usize> {
    let mut cleared = 0;
    for dir in dirs {
        for (file, _, kind) in USER_FILES {
            let p = dir.join(file);
            cleared += lists::Doc::load(&p).len();
            lists::Doc::default().save(&p, kind)?;
        }
    }
    Ok(cleared)
}

pub fn run(paths: &Paths) -> anyhow::Result<usize> {
    super::tgproxy::control::stop();
    super::lanproxy::control::shutdown();
    let _ = super::lanproxy::control::enable(false);
    let cleared = clear_lists(&list_dirs(paths))?;
    let my_sites = super::checks::my_sites_path(paths);
    if my_sites.exists() {
        fs::write(&my_sites, "")?;
    }
    let rules = super::rules::path(paths);
    if rules.exists() {
        fs::remove_file(rules)?;
    }
    super::gamefilter::save(paths, &super::gamefilter::GameFilter::default())?;
    super::pickstore::clear();
    clear_registry();
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_lists_return_to_placeholders() {
        let dir = std::env::temp_dir().join(format!("hz-reset-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("list-general-user.txt"), "# мои\r\nsteampowered.com\r\nx.com\r\n").unwrap();
        fs::write(dir.join("ipset-all-user.txt"), "1.2.3.4/32\r\n").unwrap();
        fs::write(dir.join("list-general.txt"), "youtube.com\r\n").unwrap();
        assert_eq!(clear_lists(&[dir.clone()]).unwrap(), 3);
        assert_eq!(fs::read_to_string(dir.join("list-general-user.txt")).unwrap(), "domain.example.abc\r\n");
        assert_eq!(fs::read_to_string(dir.join("ipset-all-user.txt")).unwrap(), "203.0.113.113/32\r\n");
        assert_eq!(fs::read_to_string(dir.join("list-exclude-user.txt")).unwrap(), "domain.example.abc\r\n");
        assert_eq!(fs::read_to_string(dir.join("list-general.txt")).unwrap(), "youtube.com\r\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
