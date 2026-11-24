use std::fs;
use std::path::{Path, PathBuf};

pub const UNPACKED_ARG: &str = "--unpacked";
pub const REOPEN_ARG: &str = "--reopened";
pub const UPDATED_ARG: &str = "--updated";
const LAUNCHER: &str = "HolyZapret Launcher.exe";

fn lower(p: &Path) -> String {
    p.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase()
}

fn archiver_dir(exe: &Path) -> bool {
    exe.ancestors().skip(1).filter_map(|d| d.file_name()).any(|n| {
        let n = n.to_string_lossy().to_lowercase();
        n.starts_with("rar$ex") || n.starts_with("rar$di") || (n.starts_with("7zo") && n.len() > 3) || zip_dir(&n)
    })
}

fn zip_dir(n: &str) -> bool {
    n.strip_prefix("temp")
        .and_then(|r| r.split_once('_'))
        .is_some_and(|(d, name)| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) && name.ends_with(".zip"))
}

pub fn from_archive() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = fs::canonicalize(&exe).unwrap_or(exe);
    if archiver_dir(&exe) {
        return true;
    }
    let temp = std::env::temp_dir();
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
        None => super::selfupdate::download(&super::selfupdate::latest()?, None)?,
    };
    let r = unpack_to(&bytes, target);
    if r.is_err() {
        let _ = fs::remove_dir_all(target);
    }
    r
}

pub fn copy_from(source: &Path, target: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe()?;
    let r = (|| {
        super::relocate::copy_ours(source, target, Some(&exe))?;
        let launcher = target.join(exe.file_name().unwrap_or_default());
        if !launcher.is_file() {
            fs::copy(&exe, &launcher)?;
        }
        if !target.join("bin").join("HolyZapret.exe").is_file() {
            anyhow::bail!("в {} не оказалось bin\\HolyZapret.exe", target.display());
        }
        Ok((target.to_path_buf(), launcher))
    })();
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
    fn copies_extracted_archive() {
        let base = std::env::temp_dir().join(format!("hz-copy-{}", std::process::id()));
        let (src, dst) = (base.join("Rar$EXa1.2"), base.join("HolyZapret"));
        fs::create_dir_all(src.join("bin")).unwrap();
        fs::write(src.join("bin").join("HolyZapret.exe"), "e").unwrap();
        fs::write(src.join("general.bat"), "\"%BIN%HolyZapret.exe\"").unwrap();
        let (root, exe) = copy_from(&src, &dst).unwrap();
        assert_eq!(root, dst);
        assert!(exe.is_file() && dst.join("general.bat").is_file() && dst.join("bin").join("HolyZapret.exe").is_file());
        let empty = base.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(copy_from(&empty, &base.join("HolyZapret2")).is_err());
        assert!(!base.join("HolyZapret2").exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn archiver_temp_folders() {
        for p in [
            r"C:\Users\D0F8~1\AppData\Local\Temp\Rar$EXa12345.6789\HolyZapret Launcher.exe",
            r"D:\tmp\Rar$EXa1.2\Holy-Zapret-3.1.0\HolyZapret Launcher.exe",
            r"C:\Temp\7zO4A1B2C3\HolyZapret Launcher.exe",
            r"C:\x\Temp1_Holy-Zapret-3.1.0.zip\HolyZapret Launcher.exe",
        ] {
            assert!(archiver_dir(Path::new(p)), "{p}");
        }
        for p in [r"C:\HolyZapret\HolyZapret Launcher.exe", r"C:\Users\u\Downloads\Temp\HolyZapret Launcher.exe", r"C:\7zip\HolyZapret Launcher.exe"] {
            assert!(!archiver_dir(Path::new(p)), "{p}");
        }
    }

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
