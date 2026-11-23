#![windows_subsystem = "windows"]

mod app;
mod core;
mod instance;
mod log;
mod theme;
mod tray;
mod window;

fn main() -> eframe::Result {
    let moved = std::env::args().any(|a| a == core::relocate::MOVED_ARG || a == core::unpack::UNPACKED_ARG || a == core::unpack::REOPEN_ARG);
    let mut owned = instance::acquire();
    for _ in 0..if moved { 50 } else { 0 } {
        if owned {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        owned = instance::acquire();
    }
    if !owned {
        instance::activate_existing();
        return Ok(());
    }
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([940.0, 640.0])
        .with_min_inner_size([780.0, 520.0])
        .with_title(format!("HolyZapret v{}", env!("CARGO_PKG_VERSION")))
        .with_transparent(true)
        .with_decorations(false)
        .with_visible(!std::env::args().any(|a| a == core::autostart::TRAY_ARG));
    if let Some(i) = &icon {
        viewport = viewport.with_icon(std::sync::Arc::new(i.clone()));
    }
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    eframe::run_native("HolyZapret", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, icon)))))
}
