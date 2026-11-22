use super::paths::Paths;
use std::fmt;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const API: &str = "https://api.github.com/repos/HolyLightRU/HolyZapret/releases/latest";
const TMP_DIR: &str = ".update.tmp";
const SKIP_DIRS: [&str; 5] = ["launcher", super::flowseal::DIR, ".git", ".idea", TMP_DIR];
const LOCKING: [&str; 2] = ["HolyZapret.exe", "winws.exe"];
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub zip_url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub version: String,
    pub updated: usize,
    pub added: usize,
    pub unchanged: usize,
    pub preserved: usize,
    pub self_replaced: bool,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Версия {}: обновлено файлов — {}, новых — {}, без изменений — {}, сохранено ваших — {}",
            self.version, self.updated, self.added, self.unchanged, self.preserved
        )?;
        if self.self_replaced {
            write!(f, ". Лаунчер обновлён, перезапустите его")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    Replace,
    Keep,
    Skip,
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(secs)))
        .user_agent("HolyZapret-Launcher")
        .build()
        .into()
}

fn strip_v(tag: &str) -> &str {
    let t = tag.trim();
    t.strip_prefix(['v', 'V']).unwrap_or(t)
}

pub fn parse_release(json: &str) -> anyhow::Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow::anyhow!("в ответе GitHub нет версии"))?;
    let version = strip_v(tag).to_string();
    if version.is_empty() {
        anyhow::bail!("в ответе GitHub пустая версия");
    }
    let assets: Vec<(String, String)> = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let url = a["browser_download_url"].as_str()?;
            let name = a["name"].as_str().unwrap_or(url.rsplit('/').next().unwrap_or(url));
            Some((name.to_lowercase(), url.to_string()))
        })
        .filter(|(name, _)| name.ends_with(".zip"))
        .collect();
    let zip_url = assets
        .iter()
        .find(|(name, _)| name.starts_with("holy-zapret"))
        .or_else(|| assets.first())
        .map(|(_, url)| url.clone())
        .ok_or_else(|| anyhow::anyhow!("в релизе {version} нет zip-архива"))?;
    Ok(Release { version, zip_url })
}

pub fn latest() -> anyhow::Result<Release> {
    let json = agent(15)
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_to_string()?;
    parse_release(&json)
}

fn numbers(v: &str) -> Option<Vec<u64>> {
    let v = strip_v(v);
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|p| p.parse().ok()).collect()
}

pub fn is_newer(local: &str, remote: &str) -> bool {
    if strip_v(remote).is_empty() {
        return false;
    }
    match (numbers(local), numbers(remote)) {
        (Some(mut l), Some(mut r)) => {
            let n = l.len().max(r.len());
            l.resize(n, 0);
            r.resize(n, 0);
            r > l
        }
        _ => strip_v(local) != strip_v(remote),
    }
}

fn parts(rel: &Path) -> Vec<String> {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn rule(rel: &Path) -> Rule {
    let p = parts(rel);
    let Some(name) = p.last() else { return Rule::Skip };
    if SKIP_DIRS.contains(&p[0].as_str()) {
        return Rule::Skip;
    }
    if name == ".gitignore" || name.ends_with(".old") || p == ["bin", "game_filter.enabled"] {
        return Rule::Skip;
    }
    if name.ends_with("-user.txt") || p == ["lists", "my-sites.txt"] {
        return Rule::Keep;
    }
    Rule::Replace
}

#[cfg(test)]
fn protected(rel: &Path) -> bool {
    rule(rel) != Rule::Replace
}

fn safe_rel(name: &str) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for seg in name.split(['/', '\\']) {
        match seg {
            "" | "." => continue,
            ".." => return None,
            s if s.contains(':') || s.chars().any(|c| c.is_control()) => return None,
            s => p.push(s),
        }
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

fn extract(bytes: &[u8], target: &Path) -> anyhow::Result<usize> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut written = 0;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.is_dir() || f.is_symlink() || f.enclosed_name().is_none() {
            continue;
        }
        let Some(rel) = std::str::from_utf8(f.name_raw()).ok().and_then(safe_rel) else { continue };
        let mut data = Vec::new();
        (&mut f).take(MAX_UNPACKED - total + 1).read_to_end(&mut data)?;
        total += data.len() as u64;
        if total > MAX_UNPACKED {
            anyhow::bail!("архив обновления распаковывается в слишком большой объём");
        }
        let out = target.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&out, data)?;
        written += 1;
    }
    Ok(written)
}

fn is_release_root(dir: &Path) -> bool {
    dir.join("bin").join("HolyZapret.exe").is_file()
}

fn release_root(tmp: &Path) -> anyhow::Result<PathBuf> {
    if is_release_root(tmp) {
        return Ok(tmp.to_path_buf());
    }
    let found: Vec<PathBuf> = fs::read_dir(tmp)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && is_release_root(p))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        _ => anyhow::bail!("в архиве нет bin\\HolyZapret.exe — это не похоже на сборку HolyZapret"),
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let t = e.file_type()?;
        if t.is_dir() {
            walk(&e.path(), out)?;
        } else if t.is_file() {
            out.push(e.path());
        }
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase(),
        _ => false,
    }
}

fn same_content(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => matches!((fs::read(a), fs::read(b)), (Ok(x), Ok(y)) if x == y),
        _ => false,
    }
}

fn old_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let first = dest.with_file_name(format!("{name}.old"));
    if !first.exists() || fs::remove_file(&first).is_ok() {
        return first;
    }
    (1..)
        .map(|i| dest.with_file_name(format!("{name}.{i}.old")))
        .find(|p| !p.exists() || fs::remove_file(p).is_ok())
        .unwrap_or(first)
}

fn replace_in_use(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let old = old_path(dest);
    fs::rename(dest, &old)
        .map_err(|e| anyhow::anyhow!("файл {} занят и не может быть заменён: {e}", dest.display()))?;
    if let Err(e) = fs::copy(src, dest) {
        let _ = fs::rename(&old, dest);
        anyhow::bail!("не удалось записать {}: {e}", dest.display());
    }
    Ok(())
}

fn put(src: &Path, dest: &Path, self_exe: Option<&Path>) -> anyhow::Result<bool> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() && self_exe.is_some_and(|exe| same_path(dest, exe)) {
        replace_in_use(src, dest)?;
        return Ok(true);
    }
    match fs::copy(src, dest) {
        Ok(_) => Ok(false),
        Err(_) if dest.is_file() => replace_in_use(src, dest).map(|_| false),
        Err(e) => anyhow::bail!("не удалось записать {}: {e}", dest.display()),
    }
}

fn copy_over(src_root: &Path, root: &Path, self_exe: Option<&Path>, sum: &mut Summary) -> anyhow::Result<()> {
    let mut files = Vec::new();
    walk(src_root, &mut files)?;
    files.sort();
    for src in files {
        let rel = src.strip_prefix(src_root)?.to_path_buf();
        let dest = root.join(&rel);
        let existed = dest.exists();
        if existed && !dest.is_file() {
            anyhow::bail!("вместо файла {} лежит папка — удалите её и повторите", rel.display());
        }
        match rule(&rel) {
            Rule::Skip => continue,
            Rule::Keep if existed => sum.preserved += 1,
            Rule::Replace if existed && same_content(&src, &dest) => sum.unchanged += 1,
            _ => {
                if put(&src, &dest, self_exe)? {
                    sum.self_replaced = true;
                }
                if existed {
                    sum.updated += 1;
                } else {
                    sum.added += 1;
                }
            }
        }
    }
    Ok(())
}

fn install_zip(paths: &Paths, bytes: &[u8], version: &str) -> anyhow::Result<Summary> {
    let tmp = paths.root.join(TMP_DIR);
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| -> anyhow::Result<Summary> {
        if extract(bytes, &tmp)? == 0 {
            anyhow::bail!("архив обновления пуст");
        }
        let src = release_root(&tmp)?;
        let self_exe = std::env::current_exe().ok();
        let mut sum = Summary { version: version.to_string(), ..Default::default() };
        copy_over(&src, &paths.root, self_exe.as_deref(), &mut sum)?;
        Ok(sum)
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn ensure_stopped() -> anyhow::Result<()> {
    if let Some(image) = LOCKING.iter().find(|i| super::process_running(i)) {
        anyhow::bail!("Сначала выключите защиту: запущен {image}, он держит файлы, которые нужно обновить");
    }
    Ok(())
}

pub fn download(rel: &Release) -> anyhow::Result<Vec<u8>> {
    Ok(agent(300).get(&rel.zip_url).call()?.body_mut().with_config().limit(MAX_DOWNLOAD).read_to_vec()?)
}

pub fn install(paths: &Paths, rel: &Release, bytes: &[u8]) -> anyhow::Result<Summary> {
    ensure_stopped()?;
    install_zip(paths, bytes, &rel.version)
}

fn collect_old(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let child = rel.join(e.file_name());
        if t.is_dir() {
            if !SKIP_DIRS.contains(&parts(&child)[0].as_str()) {
                collect_old(&e.path(), &child, out);
            }
        } else if t.is_file() && e.file_name().to_string_lossy().to_lowercase().ends_with(".old") {
            out.push(e.path());
        }
    }
}

pub fn cleanup_old(paths: &Paths) -> usize {
    let _ = fs::remove_dir_all(paths.root.join(TMP_DIR));
    let mut found = Vec::new();
    collect_old(&paths.root, Path::new(""), &mut found);
    found.iter().filter(|p| fs::remove_file(p).is_ok()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hz-su-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put_file(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn read(root: &Path, rel: &str) -> String {
        fs::read_to_string(root.join(rel)).unwrap()
    }

    fn build_zip(prefix: &str, entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in entries {
                z.start_file(format!("{prefix}{name}"), o).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn release_json_parsed() {
        let json = r#"{"tag_name":"v2.0.2","assets":[
            {"name":"notes.txt","browser_download_url":"https://x/notes.txt"},
            {"name":"other.zip","browser_download_url":"https://x/other.zip"},
            {"name":"Holy-Zapret-2.0.2.zip","browser_download_url":"https://x/Holy-Zapret-2.0.2.zip"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "2.0.2");
        assert_eq!(r.zip_url, "https://x/Holy-Zapret-2.0.2.zip");
        let r = parse_release(r#"{"tag_name":"2.1","assets":[{"browser_download_url":"https://x/a.zip"}]}"#).unwrap();
        assert_eq!((r.version.as_str(), r.zip_url.as_str()), ("2.1", "https://x/a.zip"));
        assert!(parse_release(r#"{"tag_name":"v1","assets":[]}"#).is_err());
        assert!(parse_release(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse_release("not json").is_err());
        let _: fn() -> anyhow::Result<Release> = latest;
        let _: fn(&Release) -> anyhow::Result<Vec<u8>> = download;
        let _: fn(&Paths, &Release, &[u8]) -> anyhow::Result<Summary> = install;
    }

    #[test]
    fn versions_compared_numerically() {
        assert!(is_newer("2.0.9", "2.0.10"));
        assert!(!is_newer("2.0.10", "2.0.9"));
        assert!(!is_newer("2.0.2", "2.0.2"));
        assert!(!is_newer("2.0", "2.0.0"));
        assert!(is_newer("2.0", "2.0.1"));
        assert!(is_newer("v2.0.2", "2.0.3"));
        assert!(is_newer("1.9.9", "v2.0.0"));
        assert!(!is_newer("abc", "abc"));
        assert!(is_newer("abc", "2.0.0"));
        assert!(is_newer("2.0.0-beta", "2.0.0"));
        assert!(!is_newer("2.0.2", ""));
    }

    #[test]
    fn user_data_is_protected() {
        for p in [
            "exp-list/list-general-user.txt",
            "lists/ipset-all-user.txt",
            "lists/zapret-hosts-user.txt",
            "Lists/My-Sites.txt",
            "bin/game_filter.enabled",
            "flowseal/bin/winws.exe",
            "flowseal/general.bat",
            "launcher/src/main.rs",
            ".git/config",
            ".idea/misc.xml",
            ".gitignore",
            "launcher/.gitignore",
            ".update.tmp/bin/HolyZapret.exe",
            "bin/HolyZapret.exe.old",
        ] {
            assert!(protected(Path::new(p)), "{p}");
        }
        for p in [
            "bin/HolyZapret.exe",
            "bin/WinDivert64.sys",
            "manager.bat",
            "general.bat",
            "lists/list-general.txt",
            "lists/check_list.txt",
            "flowseal_strat/general.bat",
            "bin/HolyZapret-old.exe",
            "HolyZapret Launcher.exe",
        ] {
            assert!(!protected(Path::new(p)), "{p}");
        }
        assert_eq!(rule(Path::new("lists/list-general-user.txt")), Rule::Keep);
        assert_eq!(rule(Path::new("lists/my-sites.txt")), Rule::Keep);
        assert_eq!(rule(Path::new("bin/game_filter.enabled")), Rule::Skip);
    }

    #[test]
    fn unsafe_names_rejected() {
        assert_eq!(safe_rel("a/./b\\c.txt"), Some(PathBuf::from("a").join("b").join("c.txt")));
        assert_eq!(safe_rel("../x"), None);
        assert_eq!(safe_rel("a/../../x"), None);
        assert_eq!(safe_rel("C:/Windows/x"), None);
        assert_eq!(safe_rel("/"), None);
    }

    fn release_entries() -> Vec<(&'static str, &'static str)> {
        vec![
            ("manager.bat", "set \"LOCAL_VERSION=9.9.9\""),
            ("bin/HolyZapret.exe", "new-engine"),
            ("bin/WinDivert.dll", "new-dll"),
            ("bin/game_filter.enabled", "all"),
            ("lists/list-general.txt", "new-list"),
            ("lists/list-general-user.txt", "release-user"),
            ("lists/my-sites.txt", "release-sites"),
            ("exp-list/list-exclude-user.txt", ""),
            ("flowseal/bin/winws.exe", "release-flowseal"),
            ("launcher/src/main.rs", "release-src"),
            (".gitignore", "release-ignore"),
            ("bin/stale.old", "x"),
            ("new/added.txt", "added"),
            ("../escape.txt", "evil"),
        ]
    }

    fn local_install(root: &Path) {
        put_file(root, "manager.bat", "set \"LOCAL_VERSION=1.0.0\"");
        put_file(root, "bin/HolyZapret.exe", "old-engine");
        put_file(root, "bin/WinDivert.dll", "old-dll");
        put_file(root, "lists/list-general.txt", "new-list");
        put_file(root, "lists/list-general-user.txt", "mine");
        put_file(root, "lists/my-sites.txt", "my-sites");
        put_file(root, "flowseal/bin/winws.exe", "my-flowseal");
        put_file(root, "launcher/src/main.rs", "my-src");
        put_file(root, ".gitignore", "my-ignore");
        put_file(root, "utils/mine.txt", "only-local");
    }

    #[test]
    fn update_replaces_files_and_keeps_user_data() {
        let base = temp("e2e");
        let root = base.join("HolyZapret");
        local_install(&root);
        let paths = Paths { root: root.clone() };
        let zip = build_zip("", &release_entries());

        let sum = install_zip(&paths, &zip, "9.9.9").unwrap();
        assert_eq!(
            sum,
            Summary { version: "9.9.9".into(), updated: 3, added: 2, unchanged: 1, preserved: 2, self_replaced: false }
        );
        assert!(sum.to_string().contains("9.9.9"));
        assert_eq!(read(&root, "manager.bat"), "set \"LOCAL_VERSION=9.9.9\"");
        assert_eq!(read(&root, "bin/HolyZapret.exe"), "new-engine");
        assert_eq!(read(&root, "bin/WinDivert.dll"), "new-dll");
        assert_eq!(read(&root, "new/added.txt"), "added");
        assert_eq!(read(&root, "exp-list/list-exclude-user.txt"), "");
        assert_eq!(read(&root, "lists/list-general-user.txt"), "mine");
        assert_eq!(read(&root, "lists/my-sites.txt"), "my-sites");
        assert_eq!(read(&root, "flowseal/bin/winws.exe"), "my-flowseal");
        assert_eq!(read(&root, "launcher/src/main.rs"), "my-src");
        assert_eq!(read(&root, ".gitignore"), "my-ignore");
        assert_eq!(read(&root, "utils/mine.txt"), "only-local");
        assert!(!root.join("bin/game_filter.enabled").exists());
        assert!(!root.join("bin/stale.old").exists());
        assert!(!base.join("escape.txt").exists());
        assert!(!root.join(TMP_DIR).exists());

        put_file(&root, "bin/game_filter.enabled", "mode=tcp");
        let sum = install_zip(&paths, &zip, "9.9.9").unwrap();
        assert_eq!((sum.updated, sum.added, sum.preserved), (0, 0, 3));
        assert_eq!(read(&root, "bin/game_filter.enabled"), "mode=tcp");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn nested_release_folder_and_locked_file() {
        let root = temp("nested");
        local_install(&root);
        let paths = Paths { root: root.clone() };
        let zip = build_zip("Holy-Zapret-9.9.9/", &release_entries());
        let lock = fs::OpenOptions::new().read(true).share_mode(1 | 4).open(root.join("bin/WinDivert.dll")).unwrap();
        let sum = install_zip(&paths, &zip, "9.9.9").unwrap();
        assert_eq!(sum.updated, 3);
        assert_eq!(read(&root, "bin/WinDivert.dll"), "new-dll");
        assert_eq!(read(&root, "bin/WinDivert.dll.old"), "old-dll");
        assert_eq!(read(&root, "lists/list-general-user.txt"), "mine");
        assert!(!root.join("Holy-Zapret-9.9.9").exists());
        drop(lock);
        put_file(&root, "flowseal/keep.old", "x");
        assert_eq!(cleanup_old(&paths), 1);
        assert!(!root.join("bin/WinDivert.dll.old").exists());
        assert!(root.join("flowseal/keep.old").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn archive_without_engine_rejected() {
        let root = temp("bad");
        local_install(&root);
        let paths = Paths { root: root.clone() };
        let zip = build_zip("", &[("manager.bat", "x"), ("bin/other.exe", "x")]);
        assert!(install_zip(&paths, &zip, "9.9.9").is_err());
        assert_eq!(read(&root, "manager.bat"), "set \"LOCAL_VERSION=1.0.0\"");
        assert!(!root.join(TMP_DIR).exists());
        assert!(install_zip(&paths, b"garbage", "9.9.9").is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}


