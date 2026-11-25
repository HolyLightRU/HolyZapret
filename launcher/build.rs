use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};
use std::path::PathBuf;

fn resources() -> PathBuf {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts: Vec<u16> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).take(3).collect();
    parts.resize(3, 0);
    let comma = format!("{},{},{},0", parts[0], parts[1], parts[2]);
    let test = std::env::var_os("HZ_CHANNEL").is_some();
    let product = if test { "HolyZapret (тестовая сборка)" } else { "HolyZapret" };
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("assets").join("icon.ico").display().to_string().replace('\\', "\\\\");
    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {comma}
PRODUCTVERSION {comma}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041904B0"
    BEGIN
      VALUE "CompanyName", "HolyLightRU"
      VALUE "FileDescription", "HolyZapret — лаунчер"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "HolyZapret Launcher"
      VALUE "LegalCopyright", "HolyLightRU, GPL-3.0-or-later"
      VALUE "OriginalFilename", "HolyZapret Launcher.exe"
      VALUE "ProductName", "{product}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x419, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.rc");
    std::fs::write(&out, rc).expect("не удалось записать app.rc");
    out
}

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_resource::compile(resources(), embed_resource::NONE).manifest_optional().unwrap();
        let level = if std::env::var_os("HZ_TEST").is_some() {
            ExecutionLevel::AsInvoker
        } else {
            ExecutionLevel::RequireAdministrator
        };
        embed_manifest(new_manifest("HolyZapret.Launcher").requested_execution_level(level))
        .expect("не удалось встроить манифест");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=HZ_TEST");
    println!("cargo:rerun-if-env-changed=HZ_CHANNEL");
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
