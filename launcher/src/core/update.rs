use super::net::{self, RAW_BASE};
use super::paths::Paths;
use std::fs;

fn parse_version(text: &str) -> Option<String> {
    let marker = "set \"LOCAL_VERSION=";
    let start = text.find(marker)? + marker.len();
    let end = text[start..].find('"')?;
    Some(text[start..start + end].to_string())
}

pub fn local_version(paths: &Paths) -> Option<String> {
    parse_version(&fs::read_to_string(paths.root.join("manager.bat")).ok()?)
}

pub fn update_auto_update(paths: &Paths) -> anyhow::Result<()> {
    let text = net::get_text(&format!("{RAW_BASE}auto_update.bat"))?;
    fs::write(paths.root.join("auto_update.bat"), text)?;
    Ok(())
}
