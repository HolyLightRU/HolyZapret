use super::paths::Paths;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const DIR: &str = "flowseal";
const TMP_DIR: &str = "flowseal.tmp";
const API: &str = "https://api.github.com/repos/Flowseal/zapret-discord-youtube/releases/latest";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub zip_url: String,
}

pub fn dir(paths: &Paths) -> PathBuf {
    paths.root.join(DIR)
}

pub fn version_in(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(".version")).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn installed(paths: &Paths) -> Option<String> {
    version_in(&dir(paths))
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(secs)))
        .user_agent("HolyZapret-Launcher")
        .build()
        .into()
}

pub fn parse_release(json: &str) -> anyhow::Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let version = v["tag_name"].as_str().ok_or_else(|| anyhow::anyhow!("в ответе GitHub нет версии"))?.to_string();
    let zip_url = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["browser_download_url"].as_str())
        .find(|u| u.ends_with(".zip"))
        .ok_or_else(|| anyhow::anyhow!("в релизе {version} нет zip-архива"))?
        .to_string();
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

fn wanted(rel: &Path) -> bool {
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [file] => file.ends_with(".bat") && !file.starts_with("service"),
        [folder, _] => folder == "bin" || folder == "lists",
        _ => false,
    }
}

pub fn extract(bytes: &[u8], target: &Path) -> anyhow::Result<usize> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut written = 0;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let Some(name) = f.enclosed_name() else { continue };
        let rel: PathBuf = name.components().skip(1).collect();
        if rel.as_os_str().is_empty() || !wanted(&rel) {
            continue;
        }
        let out = target.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        fs::write(&out, data)?;
        written += 1;
    }
    Ok(written)
}

fn copy_user_lists(from: &Path, to: &Path) {
    let Ok(rd) = fs::read_dir(from.join("lists")) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.ends_with("-user.txt") {
            let _ = fs::create_dir_all(to.join("lists"));
            let _ = fs::copy(e.path(), to.join("lists").join(e.file_name()));
        }
    }
}

pub fn count_strategies(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_lowercase();
                    n.ends_with(".bat") && !n.starts_with("service")
                })
                .count()
        })
        .unwrap_or(0)
}

pub fn install(paths: &Paths, rel: &Release) -> anyhow::Result<usize> {
    let bytes = agent(120)
        .get(&rel.zip_url)
        .call()?
        .body_mut()
        .with_config()
        .limit(100 * 1024 * 1024)
        .read_to_vec()?;
    let tmp = paths.root.join(TMP_DIR);
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| -> anyhow::Result<usize> {
        let files = extract(&bytes, &tmp)?;
        if !tmp.join("bin").join("winws.exe").is_file() {
            anyhow::bail!("в архиве нет bin\\winws.exe ({files} файлов)");
        }
        let n = count_strategies(&tmp);
        if n == 0 {
            anyhow::bail!("в архиве нет стратегий");
        }
        fs::write(tmp.join(".version"), &rel.version)?;
        let target = dir(paths);
        if target.exists() {
            copy_user_lists(&target, &tmp);
            fs::remove_dir_all(&target)?;
        }
        fs::rename(&tmp, &target)?;
        Ok(n)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

pub fn uninstall(paths: &Paths) -> anyhow::Result<()> {
    let d = dir(paths);
    if d.exists() {
        fs::remove_dir_all(d)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn release_json_parsed() {
        let json = r#"{"tag_name":"1.10.4","assets":[
            {"browser_download_url":"https://x/zapret-1.10.4.rar"},
            {"browser_download_url":"https://x/zapret-1.10.4.zip"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "1.10.4");
        assert_eq!(r.zip_url, "https://x/zapret-1.10.4.zip");
        assert!(parse_release(r#"{"tag_name":"1","assets":[]}"#).is_err());
    }

    #[test]
    fn only_strategies_bin_and_lists_are_extracted() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("pkg-1.0/general.bat", "x"),
                ("pkg-1.0/general (ALT).bat", "x"),
                ("pkg-1.0/service.bat", "x"),
                ("pkg-1.0/bin/winws.exe", "x"),
                ("pkg-1.0/lists/list-general.txt", "x"),
                ("pkg-1.0/utils/test zapret.ps1", "x"),
                ("pkg-1.0/bin/deep/evil.bin", "x"),
                ("pkg-1.0/../escape.bat", "x"),
            ] {
                z.start_file(name, o).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("hz-fs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let n = extract(buf.get_ref(), &dir).unwrap();
        assert_eq!(n, 4);
        assert!(dir.join("general (ALT).bat").exists());
        assert!(dir.join("bin/winws.exe").exists());
        assert!(dir.join("lists/list-general.txt").exists());
        assert!(!dir.join("service.bat").exists());
        assert!(!dir.join("utils").exists());
        assert_eq!(count_strategies(&dir), 2);
        fs::remove_dir_all(&dir).unwrap();
    }
}

