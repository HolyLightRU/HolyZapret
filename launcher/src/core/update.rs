use super::paths::Paths;
use std::fs;

pub const STUB_MAX: u64 = 4096;

pub fn local_version(_paths: &Paths) -> Option<String> {
    Some(env!("CARGO_PKG_VERSION").to_string())
}

pub fn drop_leftovers(paths: &Paths) -> usize {
    let mut n = 0;
    for p in [paths.root.join("scripts").join("manager.bat"), paths.root.join("manager.bat")] {
        let ours = fs::read(&p).is_ok_and(|b| b.windows(13).any(|w| w == b"LOCAL_VERSION"));
        if ours && fs::remove_file(&p).is_ok() {
            n += 1;
        }
    }
    let bin = paths.bin();
    let no_old_engine = fs::metadata(bin.join("HolyZapret.exe")).map(|m| m.len() <= STUB_MAX).unwrap_or(true);
    let dll = bin.join("cygwin1.dll");
    if no_old_engine && dll.is_file() && fs::remove_file(&dll).is_ok() {
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comes_from_the_launcher_and_old_files_are_dropped() {
        let root = std::env::temp_dir().join(format!("hz-leftovers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let paths = Paths { root: root.clone() };
        let mut bat = b"@echo off\r\nset \"LOCAL_VERSION=3.2.0\"\r\n".to_vec();
        bat.extend([0x8f, 0xe0, 0xa8]);
        fs::write(root.join("scripts").join("manager.bat"), &bat).unwrap();
        fs::write(root.join("manager.bat"), "echo my own script").unwrap();
        fs::write(root.join("bin").join("cygwin1.dll"), "dll").unwrap();
        fs::write(root.join("bin").join("HolyZapret.exe"), vec![0u8; 200_000]).unwrap();
        assert_eq!(local_version(&paths).as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(drop_leftovers(&paths), 1);
        assert!(!root.join("scripts").join("manager.bat").exists() && root.join("manager.bat").exists());
        assert!(root.join("bin").join("cygwin1.dll").exists(), "рядом настоящий старый движок");
        fs::write(root.join("bin").join("HolyZapret.exe"), "stub").unwrap();
        assert_eq!(drop_leftovers(&paths), 1);
        assert!(!root.join("bin").join("cygwin1.dll").exists() && root.join("bin").join("HolyZapret.exe").exists());
        assert_eq!(drop_leftovers(&paths), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
