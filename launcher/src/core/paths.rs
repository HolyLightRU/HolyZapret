use std::path::PathBuf;

pub fn find_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent();
    while let Some(d) = dir {
        if d.join("bin").join("HolyZapret.exe").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
    pub fn engine(&self) -> PathBuf {
        self.bin().join("HolyZapret.exe")
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
