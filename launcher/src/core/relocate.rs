use super::paths::Paths;
use super::service::{self, ServiceState};
use std::fs;
use std::path::{Path, PathBuf};

pub const MOVED_ARG: &str = "--moved-from";
const DIRS: [&str; 7] = ["bin", "lists", "exp-list", "utils", "scripts", "additional", "experimental"];
const ENGINES: [&str; 2] = ["RayOfLight.exe", "HolyZapret.exe"];

pub fn target_dir() -> PathBuf {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let base = PathBuf::from(format!("{drive}\\"));
    (1..)
        .map(|i| if i == 1 { base.join("HolyZapret") } else { base.join(format!("HolyZapret{i}")) })
        .find(|p| !p.exists() || fs::read_dir(p).map(|mut d| d.next().is_none()).unwrap_or(false))
        .unwrap_or_else(|| base.join("HolyZapret"))
}

pub fn installed() -> Option<PathBuf> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let dir = PathBuf::from(format!("{drive}\\HolyZapret"));
    let exe = dir.join("HolyZapret Launcher.exe");
    (exe.is_file() && ENGINES.iter().any(|e| dir.join("bin").join(e).is_file())).then_some(exe)
}

fn mentions_zapret(p: &Path) -> bool {
    fs::read(p).map(|b| String::from_utf8_lossy(&b).to_lowercase().contains("zapret")).unwrap_or(false)
}

fn has_strategy(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bat")) && mentions_zapret(&e.path())))
        .unwrap_or(false)
}

fn signed_dir(dir: &Path, name: &str) -> bool {
    match name {
        "bin" => ENGINES.iter().any(|e| dir.join(e).is_file()),
        "scripts" => dir.join("manager.bat").is_file() || has_strategy(dir),
        "lists" => dir.join("hosts-list.txt").is_file() || dir.join("ipset-all.txt").is_file(),
        "exp-list" => dir.join("ipset-all.txt").is_file() || dir.join("list-general.txt").is_file(),
        "utils" => dir.join("targets.txt").is_file(),
        _ => has_strategy(dir),
    }
}

fn ours(root: &Path, launcher: Option<&Path>) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    rd.flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                DIRS.iter().any(|d| d.eq_ignore_ascii_case(&name)) && signed_dir(&e.path(), &name)
            } else {
                ((name.ends_with(".bat") || name == "readme.md") && mentions_zapret(&e.path()))
                    || name == "holyzapret launcher.exe"
                    || launcher.is_some_and(|l| l.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(&name)))
            }
        })
        .map(|e| e.path())
        .collect()
}

fn copy_tree(src: &Path, dst: &Path) -> anyhow::Result<usize> {
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        return Ok(1);
    }
    fs::create_dir_all(dst)?;
    let mut n = 0;
    for e in fs::read_dir(src)? {
        let e = e?;
        let ty = e.file_type()?;
        if ty.is_dir() || ty.is_file() {
            n += copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
    }
    Ok(n)
}

pub fn copy_ours(root: &Path, target: &Path, launcher: Option<&Path>) -> anyhow::Result<usize> {
    fs::create_dir_all(target)?;
    let mut n = 0;
    for p in ours(root, launcher) {
        if let Some(name) = p.file_name() {
            n += copy_tree(&p, &target.join(name))?;
        }
    }
    Ok(n)
}

pub fn run(paths: &Paths, target: PathBuf) -> anyhow::Result<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe()?;
    let new_exe = target.join(exe.file_name().unwrap_or_default());
    let moved = Paths { root: target.clone() };
    let copied = (|| -> anyhow::Result<()> {
        copy_ours(&paths.root, &target, Some(&exe))?;
        if !new_exe.is_file() {
            fs::copy(&exe, &new_exe)?;
        }
        if !super::paths::is_root(&moved.root) {
            anyhow::bail!("в {} не оказалось папки bin с движком", target.display());
        }
        Ok(())
    })();
    if let Err(e) = copied {
        let _ = fs::remove_dir_all(&target);
        return Err(e);
    }
    let status = service::status(paths);
    if status.service != ServiceState::NotInstalled {
        if let Some(id) = status.active_strategy() {
            let find = |p: &Paths| super::strategy::discover(p).into_iter().find(|s| s.matches(id));
            let Some(new) = find(&moved) else {
                let _ = fs::remove_dir_all(&target);
                anyhow::bail!("в новой папке нет RayOfLight для службы ({id}), перенос отменён");
            };
            let _ = service::remove();
            if let Err(e) = service::install(&moved, &new) {
                if let Some(old) = find(paths) {
                    let _ = service::install(paths, &old);
                }
                let _ = fs::remove_dir_all(&target);
                return Err(e);
            }
        }
    }
    if super::autostart::enabled() {
        super::autostart::set_exe(true, &new_exe)?;
    }
    Ok((target, new_exe))
}

pub(crate) fn recycle(p: &Path) -> bool {
    use windows_sys::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW};
    use std::os::windows::ffi::OsStrExt;
    let mut from: Vec<u16> = p.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    unsafe {
        let mut op: SHFILEOPSTRUCTW = std::mem::zeroed();
        op.wFunc = FO_DELETE;
        op.pFrom = from.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;
        SHFileOperationW(&mut op) == 0 && op.fAnyOperationsAborted == 0
    }
}

fn same(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

pub fn can_remove_old(old: &Path, current: &Path) -> bool {
    ENGINES.iter().any(|e| old.join("bin").join(e).is_file())
        && ENGINES.iter().any(|e| current.join("bin").join(e).is_file())
        && !same(old, current)
        && !current.starts_with(old)
        && old.parent().is_some()
}

pub fn remove_old(old: &Path, current: &Path) -> anyhow::Result<(usize, bool)> {
    if !can_remove_old(old, current) {
        anyhow::bail!("{} не похожа на старую папку HolyZapret", old.display());
    }
    let mut failed = Vec::new();
    let entries = ours(old, None);
    for p in &entries {
        if !recycle(p) {
            failed.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        }
    }
    if !failed.is_empty() {
        anyhow::bail!("не удалось удалить: {}. Возможно, они ещё заняты, попробуйте позже", failed.join(", "));
    }
    let empty = fs::read_dir(old).map(|mut d| d.next().is_none()).unwrap_or(false);
    if empty {
        let _ = fs::remove_dir(old);
    }
    Ok((entries.len(), empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_only_holyzapret_files() {
        let base = std::env::temp_dir().join(format!("hz-move-{}", std::process::id()));
        let src = base.join("Загрузки");
        let dst = base.join("dst");
        fs::create_dir_all(src.join("bin")).unwrap();
        fs::create_dir_all(src.join("exp-list")).unwrap();
        fs::create_dir_all(src.join("Фото с отпуска")).unwrap();
        fs::create_dir_all(src.join("utils")).unwrap();
        fs::create_dir_all(src.join("scripts")).unwrap();
        fs::write(src.join("scripts").join("manager.bat"), "set \"LOCAL_VERSION=3.2.1\"").unwrap();
        fs::write(src.join("bin").join("RayOfLight.exe"), "x").unwrap();
        fs::write(src.join("exp-list").join("ipset-all.txt"), "1.1.1.1").unwrap();
        fs::write(src.join("exp-list").join("list-general-user.txt"), "a.com").unwrap();
        fs::write(src.join("general.bat"), "\"%BIN%HolyZapret.exe\" --wf-tcp=443").unwrap();
        fs::write(src.join("backup.bat"), "robocopy D:\\ E:\\").unwrap();
        fs::write(src.join("README.md"), "# Мой проект").unwrap();
        fs::write(src.join("utils").join("notes.txt"), "моё").unwrap();
        fs::write(src.join("HolyZapret Launcher.exe"), "z").unwrap();
        fs::write(src.join("диплом.docx"), "важно").unwrap();
        fs::write(src.join("Фото с отпуска").join("1.jpg"), "p").unwrap();
        assert_eq!(copy_ours(&src, &dst, None).unwrap(), 6);
        assert!(dst.join("bin").join("RayOfLight.exe").is_file() && dst.join("scripts").join("manager.bat").is_file());
        assert_eq!(fs::read_to_string(dst.join("exp-list").join("list-general-user.txt")).unwrap(), "a.com");
        assert!(!dst.join("диплом.docx").exists() && !dst.join("Фото с отпуска").exists());
        assert!(!dst.join("backup.bat").exists() && !dst.join("README.md").exists() && !dst.join("utils").exists());
        assert!(can_remove_old(&src, &dst));
        assert!(!can_remove_old(&dst, &dst));
        assert!(!can_remove_old(&base.join("нет"), &dst));
        fs::remove_dir_all(&base).unwrap();
    }
}
