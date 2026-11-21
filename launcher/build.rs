use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_resource::compile("assets/app.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
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
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
