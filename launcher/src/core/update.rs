use super::paths::Paths;
use std::fs;

fn parse_version(text: &str) -> Option<String> {
    let marker = "set \"LOCAL_VERSION=";
    let start = text.find(marker)? + marker.len();
    let end = text[start..].find('"')?;
    Some(text[start..start + end].to_string())
}

fn script(paths: &Paths, name: &str) -> std::path::PathBuf {
    let s = super::strategy::scripts_dir(paths);
    if s.is_dir() { s.join(name) } else { paths.root.join(name) }
}

pub fn local_version(paths: &Paths) -> Option<String> {
    let text = fs::read_to_string(script(paths, "manager.bat")).or_else(|_| fs::read_to_string(paths.root.join("manager.bat"))).ok()?;
    parse_version(&text)
}

