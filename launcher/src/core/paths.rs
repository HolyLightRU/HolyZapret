use std::path::PathBuf;

pub fn find_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent();
    while let Some(d) = dir {
        if is_root(d) {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

pub fn is_root(d: &std::path::Path) -> bool {
    let bin = d.join("bin");
    ["RayOfLight.exe", "WinDivert.dll", "HolyZapret.exe"].iter().any(|f| bin.join(f).is_file())
}

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
    pub fn lists(&self) -> PathBuf {
        self.root.join("lists")
    }
    pub fn exp_lists(&self) -> PathBuf {
        self.root.join("exp-list")
    }
    pub fn check_list(&self) -> PathBuf {
        self.lists().join("check_list.txt")
    }
    pub fn game_flag(&self) -> PathBuf {
        self.bin().join("game_filter.enabled")
    }
}
