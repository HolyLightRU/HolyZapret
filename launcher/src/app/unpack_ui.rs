use super::{App, Msg};
use crate::core::{relocate, unpack};
use crate::log::Lvl;
use crate::theme;
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::path::PathBuf;

pub struct UnpackState {
    pub zip: Option<PathBuf>,
    pub target: PathBuf,
    pub installed: Option<PathBuf>,
    pub delete_zip: bool,
    pub running: bool,
    pub error: Option<String>,
}

impl UnpackState {
    pub fn detect() -> Option<UnpackState> {
        if !unpack::from_archive() {
            return None;
        }
        let hint = std::env::current_exe().ok().and_then(|e| unpack::zip_hint(&e));
        let zip = unpack::find_zip(hint.as_deref());
        Some(UnpackState {
            delete_zip: zip.is_some(),
            zip,
            target: relocate::target_dir(),
            installed: relocate::installed(),
            running: false,
            error: None,
        })
    }
}

impl App {
    fn start_unpack(&mut self, ctx: &egui::Context) {
        let Some(u) = self.unpack.as_mut() else { return };
        u.running = true;
        u.error = None;
        let zip = u.zip.clone();
        let target = u.target.clone();
        let delete = u.delete_zip;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = unpack::install(zip.as_deref(), &target).map(|(root, exe)| {
                let recycled = delete && zip.as_deref().is_some_and(unpack::recycle);
                (root, exe, recycled)
            });
            let _ = tx.send(Msg::Unpacked(r.map_err(|e| e.to_string())));
            ctx.request_repaint();
        });
    }

    pub(super) fn on_unpacked(&mut self, ctx: &egui::Context, r: Result<(PathBuf, PathBuf, bool), String>) {
        match r {
            Ok((root, exe, _)) => {
                match std::process::Command::new(&exe).arg(unpack::UNPACKED_ARG).current_dir(&root).spawn() {
                    Ok(_) => crate::tray::exit(),
                    Err(e) => {
                        if let Some(u) = self.unpack.as_mut() {
                            u.running = false;
                        }
                        self.toast(ctx, Lvl::Err, format!("HolyZapret распакован в {}, но не запустился: {e}", root.display()));
                    }
                }
            }
            Err(e) => {
                if let Some(u) = self.unpack.as_mut() {
                    u.running = false;
                    u.error = Some(e);
                }
            }
        }
    }

    pub(super) fn draw_unpack(&mut self, ctx: &egui::Context) {
        let Some(u) = self.unpack.as_mut() else { return };
        let target = u.target.clone();
        let mut go = false;
        let mut open = None;
        let mut close = false;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        egui::Modal::new(egui::Id::NULL.with("unpack")).frame(frame).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new("HolyZapret запущен прямо из архива").size(17.0).strong());
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Так лаунчер не видит свои стратегии и движок. Распакую HolyZapret в {} и перезапущу его оттуда.",
                        target.display()
                    ))
                    .color(Color32::from_gray(190)),
                )
                .wrap(),
            );
            if let Some(exe) = &u.installed {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "HolyZapret уже установлен в {}. Лучше открыть его: он сам обновится до новой версии, а служба и ваши списки останутся на месте.",
                            exe.parent().map(|p| p.display().to_string()).unwrap_or_default()
                        ))
                        .color(theme::YELLOW),
                    )
                    .wrap(),
                );
            }
            ui.add_space(8.0);
            match &u.zip {
                Some(z) => {
                    ui.add(egui::Label::new(RichText::new(format!("Архив: {}", z.display())).color(Color32::from_gray(160)).size(12.5)).wrap());
                    ui.add_enabled(!u.running, egui::Checkbox::new(&mut u.delete_zip, "Удалить архив после распаковки"));
                }
                None => {
                    ui.add(
                        egui::Label::new(
                            RichText::new("Архив рядом не нашёлся, поэтому скачаю последнюю версию HolyZapret с GitHub")
                                .color(Color32::from_gray(160))
                                .size(12.5),
                        )
                        .wrap(),
                    );
                }
            }
            if let Some(e) = &u.error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(format!("Не получилось: {e}")).color(theme::RED).size(12.5)).wrap());
            }
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if u.running {
                    ui.spinner();
                    ui.label(RichText::new("Распаковываю…").color(Color32::from_gray(190)));
                } else if let Some(exe) = &u.installed {
                    if ui.add(theme::Primary("Открыть установленный".into())).clicked() {
                        open = Some(exe.clone());
                    }
                    if ui.button("Всё равно распаковать").clicked() {
                        go = true;
                    }
                    if ui.button("Закрыть").clicked() {
                        close = true;
                    }
                } else {
                    if ui.add(theme::Primary("Распаковать".into())).clicked() {
                        go = true;
                    }
                    if ui.button("Закрыть").clicked() {
                        close = true;
                    }
                }
            });
        });
        if let Some(exe) = open {
            let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
            match std::process::Command::new(&exe).arg(unpack::REOPEN_ARG).current_dir(dir).spawn() {
                Ok(_) => crate::tray::exit(),
                Err(e) => self.toast(ctx, Lvl::Err, format!("Не удалось открыть {}: {e}", exe.display())),
            }
        }
        if go {
            self.start_unpack(ctx);
        }
        if close {
            self.unpack = None;
        }
    }
}
