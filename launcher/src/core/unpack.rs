use std::fs;
use std::path::{Path, PathBuf};

pub const UNPACKED_ARG: &str = "--unpacked";
pub const REOPEN_ARG: &str = "--reopened";
const LAUNCHER: &str = "HolyZapret Launcher.exe";

fn lower(p: &Path) -> String {
    p.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase()
}

pub fn from_archive() -> bool {
    let (Ok(exe), temp) = (std::env::current_exe(), std::env::temp_dir()) else { return false };
    let temp = fs::canonicalize(&temp).unwrap_or(temp);
    lower(&exe).starts_with(&lower(&temp))
}

pub fn zip_hint(exe: &Path) -> Option<String> {
    let dir = exe.parent()?.file_name()?.to_string_lossy().into_owned();
    let rest = dir.strip_prefix("Temp")?;
    let (n, name) = rest.split_once('_')?;
    (n.chars().all(|c| c.is_ascii_digit()) && name.to_lowercase().ends_with(".zip")).then(|| name.to_string())
}

fn search_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for d in ["Downloads", "Desktop", "Documents"] {
            v.push(home.join(d));
        }
    }
    if let Some(od) = std::env::var_os("OneDrive").map(PathBuf::from) {
        v.push(od.join("Desktop"));
        v.push(od.join("Documents"));
    }
    v
}

pub fn is_release_zip(p: &Path) -> bool {
    let Ok(f) = fs::File::open(p) else { return false };
    let Ok(z) = zip::ZipArchive::new(f) else { return false };
    let names: Vec<String> = z.file_names().filter_map(Result::ok).map(|n| n.replace('\\', "/").to_lowercase()).collect();
    let has = |tail: &str| names.iter().any(|n| n == tail || n.ends_with(&format!("/{tail}")));
    has("bin/holyzapret.exe") && has(&LAUNCHER.to_lowercase())
}

pub fn pick(candidates: &[(PathBuf, std::time::SystemTime)], hint: Option<&str>, version: &str) -> Option<PathBuf> {
    let named = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let wanted = hint.map(str::to_lowercase).unwrap_or_else(|| format!("holy-zapret-{version}.zip"));
    candidates.iter().filter(|(p, _)| named(p) == wanted).max_by_key(|(_, t)| *t).map(|(p, _)| p.clone())
}

pub fn find_zip(hint: Option<&str>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for d in search_dirs() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    candidates.push((p, t));
                }
            }
        }
    }
    let found = pick(&candidates, hint, env!("CARGO_PKG_VERSION"))?;
    is_release_zip(&found).then_some(found)
}

pub fn install(zip: Option<&Path>, target: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let bytes = match zip {
        Some(p) => fs::read(p)?,
        None => super::selfupdate::download(&super::selfupdate::latest()?)?,
    };
    let r = unpack_to(&bytes, target);
    if r.is_err() {
        let _ = fs::remove_dir_all(target);
    }
    r
}

pub fn unpack_to(bytes: &[u8], target: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    super::selfupdate::extract(bytes, target)?;
    let root = super::selfupdate::release_root(target)?;
    let exe = root.join(LAUNCHER);
    if !exe.is_file() {
        anyhow::bail!("в архиве нет {LAUNCHER}");
    }
    Ok((root, exe))
}

pub fn recycle(zip: &Path) -> bool {
    super::relocate::recycle(zip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn explorer_temp_names_and_pick() {
        let exe = Path::new(r"C:\Users\u\AppData\Local\Temp\Temp1_Holy-Zapret-3.0.3.zip\HolyZapret Launcher.exe");
        assert_eq!(zip_hint(exe).as_deref(), Some("Holy-Zapret-3.0.3.zip"));
        assert_eq!(zip_hint(Path::new(r"C:\Temp\7zO4A1\HolyZapret Launcher.exe")), None);
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let t1 = t0 + Duration::from_secs(50);
        let c = vec![
            (PathBuf::from(r"C:\d\Holy-Zapret-3.0.2.zip"), t1),
            (PathBuf::from(r"C:\d\Holy-Zapret-3.0.3.zip"), t0),
            (PathBuf::from(r"C:\d\photos.zip"), t1 + Duration::from_secs(9)),
        ];
        assert_eq!(pick(&c, Some("holy-zapret-3.0.3.zip"), "3.0.3"), Some(PathBuf::from(r"C:\d\Holy-Zapret-3.0.3.zip")));
        assert_eq!(pick(&c, None, "3.0.3"), Some(PathBuf::from(r"C:\d\Holy-Zapret-3.0.3.zip")));
        assert_eq!(pick(&c, None, "3.0.4"), None);
        assert_eq!(pick(&c, Some("Holy-Zapret-1.3.0.zip"), "3.0.3"), None);
    }

    #[test]
    fn release_zip_recognised() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("hz-unpack-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("Holy-Zapret-9.9.9.zip");
        let bad = dir.join("Holy-Zapret-fake.zip");
        let old = dir.join("Holy-Zapret-1.3.0.zip");
        for (p, names) in [
            (&ok, vec!["bin/HolyZapret.exe", "HolyZapret Launcher.exe"]),
            (&bad, vec!["readme.txt"]),
            (&old, vec!["bin/HolyZapret.exe", "manager.bat"]),
        ] {
            let mut z = zip::ZipWriter::new(fs::File::create(p).unwrap());
            for name in names {
                z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(b"x").unwrap();
            }
            z.finish().unwrap();
        }
        assert!(!is_release_zip(&old));
        assert!(is_release_zip(&ok));
        assert!(!is_release_zip(&bad));
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for name in ["Holy-Zapret-9.9.9/bin/HolyZapret.exe", "Holy-Zapret-9.9.9/HolyZapret Launcher.exe", "Holy-Zapret-9.9.9/general.bat"] {
                z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(b"x").unwrap();
            }
            z.finish().unwrap();
        }
        let (root, exe) = unpack_to(&buf.into_inner(), &dir.join("C_HolyZapret")).unwrap();
        assert!(root.join("bin").join("HolyZapret.exe").is_file() && root.join("general.bat").is_file());
        assert_eq!(exe, root.join("HolyZapret Launcher.exe"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
