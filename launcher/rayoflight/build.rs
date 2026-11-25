use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/rayoflight.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts: Vec<u16> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).take(3).collect();
    parts.resize(3, 0);
    let comma = format!("{},{},{},0", parts[0], parts[1], parts[2]);
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("assets").join("rayoflight.ico").display().to_string().replace('\\', "\\\\");
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
      VALUE "FileDescription", "RayOfLight — движок HolyZapret"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "RayOfLight"
      VALUE "LegalCopyright", "HolyLightRU, GPL-3.0-or-later"
      VALUE "OriginalFilename", "RayOfLight.exe"
      VALUE "ProductName", "HolyZapret RayOfLight"
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
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("rayoflight.rc");
    std::fs::write(&out, rc).expect("не удалось записать rayoflight.rc");
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
}
