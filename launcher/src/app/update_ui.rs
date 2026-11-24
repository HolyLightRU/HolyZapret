use super::{App, Msg, Tab, notify, result_msg};
use crate::core::fetch::Progress;
use crate::core::tgproxy::control::human;
use crate::core::{selfupdate, service};
use crate::log::Lvl;
use crate::theme::{self, CYAN, PINK, RED};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::sync::Arc;

#[derive(Default)]
pub struct UpdateUi {
    pub open: bool,
    pub dismissed: Option<String>,
    pub running: bool,
    pub stage: String,
    pub error: Option<String>,
    pub progress: Arc<Progress>,
}

impl UpdateUi {
    pub fn status(&self) -> Option<String> {
        if !self.running {
            return None;
        }
        let (done, total) = self.progress.get();
        Some(if total > 0 && done < total {
            format!("Скачано {} из {}", human(done), human(total))
        } else {
            format!("{}…", self.stage)
        })
    }
}

fn bar(ui: &mut egui::Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), Color32::from_white_alpha(18));
    if frac > 0.005 {
        let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac.clamp(0.0, 1.0), rect.height()));
        theme::gradient_rect(ui.painter(), fill, 4.0, PINK, CYAN);
    }
}

fn newer(p: &crate::core::paths::Paths, rel: &selfupdate::Release) -> bool {
    !crate::core::TEST_BUILD && crate::core::update::local_version(p).is_some_and(|l| selfupdate::is_newer(&l, &rel.version))
}

fn engine_free(p: &crate::core::paths::Paths) -> bool {
    let s = service::status(p);
    let only_service = s.active_strategy().is_some() && s.foreign_engines.is_empty();
    !s.engine_running || only_service
}

fn install_quietly(p: &crate::core::paths::Paths, rel: &selfupdate::Release, tx: &std::sync::mpsc::Sender<Msg>) -> anyhow::Result<()> {
    let bytes = selfupdate::download(rel, None)?;
    let paused = service::pause_ours(p);
    let r = selfupdate::install(p, rel, &bytes);
    if let Some(paused) = paused {
        if let Err(e) = service::resume(p, paused) {
            notify(tx, Lvl::Err, format!("Служба после автообновления: {e}"));
        }
    }
    let sum = r?;
    notify(tx, Lvl::Ok, format!("Автообновление: {sum}"));
    let exe = std::env::current_exe().ok().filter(|_| sum.self_replaced);
    match exe {
        Some(exe) => {
            let dir = exe.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
            std::process::Command::new(&exe)
                .args([crate::core::unpack::REOPEN_ARG, crate::core::unpack::UPDATED_ARG, crate::core::autostart::TRAY_ARG])
                .current_dir(dir)
                .spawn()?;
            crate::tray::exit()
        }
        None => {
            let _ = tx.send(Msg::Updated);
            crate::tray::inform("HolyZapret обновлён", &format!("Установлена версия {}", rel.version));
            Ok(())
        }
    }
}

pub(super) fn update_loop(
    p: crate::core::paths::Paths,
    auto: Arc<std::sync::atomic::AtomicBool>,
    ui_busy: Arc<std::sync::atomic::AtomicBool>,
    tx: std::sync::mpsc::Sender<Msg>,
    ctx: egui::Context,
) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    const EVERY: Duration = Duration::from_secs(6 * 3600);
    const IDLE: u64 = 10 * 60;
    const RETRY: Duration = Duration::from_secs(3600);
    let mut checked: Option<Instant> = None;
    let mut found: Option<selfupdate::Release> = None;
    let mut wait_until: Option<Instant> = None;
    loop {
        if checked.is_none_or(|t| t.elapsed() > EVERY) {
            checked = Some(Instant::now());
            let r = selfupdate::latest();
            if let Ok(rel) = &r {
                found = Some(rel.clone());
            }
            let _ = tx.send(Msg::Version(r.map_err(|e| e.to_string())));
            ctx.request_repaint();
        }
        let fresh = found.as_ref().filter(|r| newer(&p, r));
        if let Some(rel) = fresh {
            if !crate::tray::window_visible() && crate::core::settings::notified_version().as_deref() != Some(rel.version.as_str()) {
                crate::core::settings::set_notified_version(&rel.version);
                let text = if auto.load(Ordering::Relaxed) {
                    "Обновлю сам, когда компьютером не будут пользоваться"
                } else {
                    "Откройте лаунчер, чтобы обновиться"
                };
                crate::tray::inform(&format!("Вышла HolyZapret {}", rel.version), text);
            }
        }
        let ready = fresh.is_some()
            && auto.load(Ordering::Relaxed)
            && !crate::tray::window_visible()
            && !ui_busy.load(Ordering::Relaxed)
            && wait_until.is_none_or(|t| Instant::now() >= t)
            && crate::core::health::idle_secs() >= IDLE;
        if let Some(rel) = fresh.filter(|_| ready).cloned() {
            if engine_free(&p) && crate::tray::claim() {
                if let Err(e) = install_quietly(&p, &rel, &tx) {
                    notify(&tx, Lvl::Err, format!("Автообновление не удалось: {e}"));
                    wait_until = Some(Instant::now() + RETRY);
                }
                crate::tray::release();
                ctx.request_repaint();
            }
        }
        std::thread::sleep(Duration::from_secs(60));
    }
}

impl App {
    pub(super) fn open_update(&mut self) {
        self.update.open = true;
        if !self.update.running {
            self.update.error = None;
        }
    }

    pub(super) fn run_update(&mut self, ctx: &egui::Context) {
        let Some(rel) = self.remote_release.clone() else {
            self.check_version(ctx);
            return;
        };
        if self.update.running || self.busy {
            return;
        }
        let only_service = self.status.as_ref().is_some_and(|s| s.active_strategy().is_some() && s.foreign_engines.is_empty());
        if self.trial.is_some() || (self.status.as_ref().is_some_and(|s| s.engine_running) && !only_service) {
            self.update.error = Some("Сначала выключите пробный запуск: движок держит файлы, которые нужно обновить".into());
            self.update.open = true;
            return;
        }
        let progress = self.update.progress.clone();
        progress.reset();
        self.update.running = true;
        self.update.error = None;
        self.update.stage = format!("Скачиваю HolyZapret {}", rel.version);
        let exe = std::env::current_exe().ok();
        self.spawn(ctx, move |p, tx| {
            let bytes = match selfupdate::download(&rel, Some(&progress)) {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(Msg::UpdateFailed(e.to_string()));
                    return;
                }
            };
            let _ = tx.send(Msg::UpdateStage("Устанавливаю".into()));
            let paused = service::pause_ours(p);
            let r = selfupdate::install(p, &rel, &bytes);
            if let Some(paused) = paused {
                result_msg(tx, service::resume(p, paused), "Служба после обновления");
            }
            match r {
                Ok(sum) => {
                    notify(tx, Lvl::Ok, sum.to_string());
                    match exe.filter(|_| sum.self_replaced) {
                        Some(exe) => {
                            let _ = tx.send(Msg::Restart(exe));
                        }
                        None => {
                            let _ = tx.send(Msg::Updated);
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Msg::UpdateFailed(e.to_string()));
                }
            }
        });
    }

    pub(super) fn restart_into(&mut self, ctx: &egui::Context, exe: std::path::PathBuf) {
        let dir = exe.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        match std::process::Command::new(&exe).arg(crate::core::unpack::REOPEN_ARG).current_dir(dir).spawn() {
            Ok(_) => crate::tray::exit(),
            Err(e) => {
                self.update.running = false;
                self.update.open = false;
                self.local_version = self.paths.as_ref().and_then(crate::core::update::local_version);
                self.toast(ctx, Lvl::Err, format!("Обновление установлено, но лаунчер не перезапустился: {e}. Закройте и откройте его вручную"));
            }
        }
    }

    pub(super) fn draw_update(&mut self, ctx: &egui::Context) {
        let available = self.update_available().filter(|_| self.paths.is_some());
        if let Some(v) = &available {
            let quiet = self.busy || self.picker.running || self.tab == Tab::Wizard || self.confirm.is_some() || self.unpack.is_some() || self.whats_new.is_some() || self.share.open;
            if !self.update.open && !self.update.running && self.update.dismissed.as_ref() != Some(v) && !quiet {
                self.update.open = true;
            }
        }
        if !self.update.open {
            return;
        }
        let Some(v) = available.or_else(|| self.update.running.then(|| self.remote_version.clone().unwrap_or_default())) else {
            self.update.open = false;
            return;
        };
        if self.update.running {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let local = self.local_version.clone().unwrap_or_else(|| "?".into());
        let mut go = false;
        let mut later = false;
        let mut hide = false;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        let u = &self.update;
        let modal = egui::Modal::new(egui::Id::NULL.with("update")).frame(frame).show(ctx, |ui| {
            ui.set_width(440.0);
            let title = if u.running { format!("Обновляю HolyZapret до {v}") } else { format!("Вышла HolyZapret {v}") };
            ui.label(RichText::new(title).size(17.0).strong());
            ui.add_space(4.0);
            if u.running {
                let (done, total) = u.progress.get();
                ui.label(RichText::new(format!("{}…", u.stage)).color(Color32::from_gray(200)));
                ui.add_space(6.0);
                if total > 0 && done < total {
                    bar(ui, done as f32 / total as f32);
                    ui.add_space(2.0);
                    ui.label(RichText::new(format!("Скачано {} из {}", human(done), human(total))).color(Color32::from_gray(160)).size(12.5));
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Это займёт несколько секунд").color(Color32::from_gray(160)).size(12.5));
                    });
                }
            } else {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Сейчас установлена {local}. Обновление скачается с GitHub и встанет поверх: ваши списки, правила и настройки сохранятся. Защита остановится на несколько секунд, а лаунчер сам перезапустится."
                        ))
                        .color(Color32::from_gray(190)),
                    )
                    .wrap(),
                );
            }
            if let Some(e) = &u.error {
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(format!("Не получилось: {e}")).color(RED).size(12.5)).wrap());
            }
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if u.running {
                    if ui.button("Скрыть").on_hover_text("Обновление продолжится, ход виден в Настройках").clicked() {
                        hide = true;
                    }
                } else {
                    let label = if u.error.is_some() { "Повторить" } else { "Обновить" };
                    if ui.add(theme::Primary(label.into())).clicked() {
                        go = true;
                    }
                    if ui.button("Позже").clicked() {
                        later = true;
                    }
                }
            });
        });
        if super::modal_close(ctx, &modal) && !self.update.running {
            later = true;
        }
        if go {
            self.run_update(ctx);
        }
        if later {
            self.update.open = false;
            self.update.dismissed = Some(v);
        }
        if hide {
            self.update.open = false;
        }
    }
}
