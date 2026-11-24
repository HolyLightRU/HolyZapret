use std::fs;
use std::path::{Path, PathBuf};

fn file() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("HolyZapret").join("crash.txt")
}

fn describe(info: &std::panic::PanicHookInfo, version: &str, when: &str, thread: &str, trace: &str) -> String {
    let trace: Vec<&str> = trace.lines().take(80).collect();
    format!("Версия: {version}\nВремя: {when}\nПоток: {thread}\n{info}\n\nСтек:\n{}\n", trace.join("\n"))
}

fn write_to(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, text);
}

fn take_from(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let _ = fs::remove_file(path);
    Some(text).filter(|t| !t.trim().is_empty())
}

pub fn install() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("фоновый").to_string();
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        write_to(&file(), &describe(info, env!("CARGO_PKG_VERSION"), &crate::log::now_stamp(), &thread, &trace));
        prev(info);
    }));
}

pub fn take() -> Option<String> {
    take_from(&file())
}

pub fn headline(text: &str) -> String {
    let mut lines = text.lines().skip_while(|l| !l.starts_with("panicked at")).filter(|l| !l.trim().is_empty());
    let place = lines.next().unwrap_or_default().trim_start_matches("panicked at ").trim_end_matches(':');
    let what = lines.next().unwrap_or_default();
    match (place.is_empty(), what.is_empty() || what.starts_with("Стек")) {
        (true, _) => "место ошибки не записалось".into(),
        (false, true) => place.to_string(),
        (false, false) => format!("{what} ({place})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_once_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("hz-crash-{}", std::process::id()));
        let f = dir.join("sub").join("crash.txt");
        assert_eq!(take_from(&f), None);
        write_to(&f, "Версия: 3.1.4\npanicked at src/app.rs:10:5:\nindex out of bounds\n\nСтек:\n0: x\n");
        let t = take_from(&f).unwrap();
        assert!(t.contains("index out of bounds"));
        assert_eq!(take_from(&f), None);
        assert_eq!(headline(&t), "index out of bounds (src/app.rs:10:5)");
        assert_eq!(headline("Версия: 1\npanicked at src/a.rs:1:1:\n\nСтек:\n"), "src/a.rs:1:1");
        assert_eq!(headline("мусор"), "место ошибки не записалось");
        let _ = fs::remove_dir_all(&dir);
    }
}
