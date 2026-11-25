use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};

pub const ENGINES: [&str; 2] = ["HolyZapret.exe", "winws.exe"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub name: String,
    pub path: Option<PathBuf>,
}

fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

pub fn list(names: &[&str]) -> Vec<Proc> {
    find(|name| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

pub fn windowed() -> std::collections::HashSet<String> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
    };
    unsafe extern "system" fn visit(h: HWND, l: LPARAM) -> i32 {
        unsafe {
            let pids = &mut *(l as *mut Vec<u32>);
            let tool = GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0;
            if IsWindowVisible(h) != 0 && GetWindowTextLengthW(h) > 0 && GetWindow(h, GW_OWNER).is_null() && !tool {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, &mut pid);
                pids.push(pid);
            }
        }
        1
    }
    let mut pids: Vec<u32> = Vec::new();
    unsafe {
        EnumWindows(Some(visit), &mut pids as *mut Vec<u32> as LPARAM);
    }
    let names = names();
    pids.iter().filter_map(|p| names.get(p)).map(|n| n.to_lowercase()).filter(|n| !SHELL.contains(&n.as_str())).collect()
}

const SHELL: [&str; 7] = [
    "textinputhost.exe",
    "applicationframehost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "lockapp.exe",
    "explorer.exe",
];

pub fn names() -> std::collections::HashMap<u32, String> {
    let mut out = std::collections::HashMap::new();
    each(|pid, name| {
        out.insert(pid, name.to_string());
    });
    out
}

fn each(mut f: impl FnMut(u32, &str)) {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            f(e.th32ProcessID, &String::from_utf16_lossy(&e.szExeFile[..len]));
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
}

pub fn find(pred: impl Fn(&str) -> bool) -> Vec<Proc> {
    let mut out = Vec::new();
    each(|pid, name| {
        if pred(name) {
            out.push(Proc { name: name.to_string(), path: image_path(pid) });
        }
    });
    out
}

pub fn running(name: &str) -> bool {
    !list(&[name]).is_empty()
}

pub fn under(path: &Path, root: &Path) -> bool {
    let a = path.to_string_lossy().to_lowercase().replace('/', "\\");
    let a = a.trim_start_matches(r"\\?\").to_string();
    let mut b = root.to_string_lossy().to_lowercase().replace('/', "\\");
    b = b.trim_start_matches(r"\\?\").to_string();
    if !b.ends_with('\\') {
        b.push('\\');
    }
    a.starts_with(&b)
}

pub fn exe_from_command(cmd: &str) -> Option<PathBuf> {
    let c = cmd.trim();
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().filter(|s| !s.is_empty()).map(PathBuf::from);
    }
    let lower = c.to_lowercase();
    lower.find(".exe").map(|i| PathBuf::from(&c[..i + 4]))
}

pub fn install_dir(engine: &Path) -> PathBuf {
    let dir = engine.parent().map(Path::to_path_buf).unwrap_or_default();
    match dir.file_name() {
        Some(n) if n.to_string_lossy().eq_ignore_ascii_case("bin") => dir.parent().map(Path::to_path_buf).unwrap_or(dir),
        _ => dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_parsed() {
        assert_eq!(
            exe_from_command(r#""C:\zapret discord\bin\winws.exe" --wf-tcp=80"#),
            Some(PathBuf::from(r"C:\zapret discord\bin\winws.exe"))
        );
        assert_eq!(exe_from_command(r"C:\z\bin\winws.exe --x"), Some(PathBuf::from(r"C:\z\bin\winws.exe")));
        assert_eq!(exe_from_command(""), None);
    }

    #[test]
    fn folders() {
        assert!(under(Path::new(r"C:\HZ\flowseal\bin\winws.exe"), Path::new(r"C:\hz")));
        assert!(under(Path::new(r"\\?\C:\HZ\bin\HolyZapret.exe"), Path::new(r"C:\HZ\")));
        assert!(!under(Path::new(r"C:\HZ2\bin\winws.exe"), Path::new(r"C:\HZ")));
        assert_eq!(install_dir(Path::new(r"C:\zapret\bin\winws.exe")), PathBuf::from(r"C:\zapret"));
    }

    #[test]
    fn this_process_is_listed() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().into_owned();
        let found = list(&[&name]);
        assert!(found.iter().any(|p| p.path.as_deref().is_some_and(|x| x.file_name() == me.file_name())));
    }
}
