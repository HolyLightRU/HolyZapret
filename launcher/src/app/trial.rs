use super::{App, Msg};
use crate::core::service::{self, ServiceState};
use crate::core::{gamefilter, picker};
use crate::log::Lvl;
use crate::theme::{self, CYAN};
use eframe::egui::{self, Color32, RichText};
use std::process::Child;

pub struct Trial {
    pub child: Child,
    pub id: String,
    pub name: String,
    pub restore_service: bool,
    pub had_divert: bool,
}

impl App {
    pub(super) fn start_trial(&mut self, ctx: &egui::Context) {
        if self.trial.is_some() {
            return;
        }
        let Some(s) = self.selected.and_then(|i| self.strategies.get(i)).cloned() else {
            self.toast(ctx, Lvl::Info, "Выберите стратегию в списке");
            return;
        };
        self.spawn(ctx, move |p, tx| {
            let restore = service::query(service::SERVICE) == ServiceState::Running;
            let had_divert = service::query("WinDivert") != ServiceState::NotInstalled;
            if restore {
                crate::core::run_hidden("net", &["stop", service::SERVICE]);
            }
            let r = picker::start_trial(p, &s, &gamefilter::load(p)).map(|child| Trial {
                child,
                id: s.id.clone(),
                name: s.name.clone(),
                restore_service: restore,
                had_divert,
            });
            if r.is_err() && restore {
                let _ = service::start();
            }
            let _ = tx.send(Msg::TrialStarted(r.map_err(|e| e.to_string())));
        });
    }

    pub(super) fn stop_trial(&mut self, ctx: &egui::Context) {
        let Some(t) = self.trial.take() else { return };
        self.spawn(ctx, move |_, tx| {
            let name = t.name.clone();
            let restore = t.restore_service;
            let had_divert = t.had_divert;
            picker::stop_trial(t.child);
            if !restore && !had_divert {
                let _ = service::unload_windivert();
            }
            let mut msg = format!("Пробный запуск «{name}» остановлен");
            if restore {
                match service::start() {
                    Ok(()) => msg.push_str(", служба снова запущена"),
                    Err(e) => msg.push_str(&format!(", но службу не удалось запустить: {e}")),
                }
            }
            super::notify(tx, Lvl::Info, msg);
        });
    }

    pub(super) fn stop_trial_now(&mut self) {
        if let Some(t) = self.trial.take() {
            picker::stop_trial(t.child);
            if t.restore_service {
                let _ = service::start();
            } else if !t.had_divert {
                let _ = service::unload_windivert();
            }
        }
    }

    pub(super) fn trial_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((id, name)) = self.trial.as_ref().map(|t| (t.id.clone(), t.name.clone())) else { return };
        let mut stop = false;
        let mut install = false;
        theme::card(ui, |ui| {
            theme::row(ui, 44.0, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!self.busy, |ui| {
                        if ui.add(theme::Primary("Установить службой".into())).clicked() {
                            install = true;
                        }
                        if ui.button("Остановить").clicked() {
                            stop = true;
                        }
                    });
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(RichText::new(theme::icon::PICK).font(theme::icon_font(18.0)).color(CYAN));
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(format!("Пробный запуск: {name}")).strong()).truncate());
                            ui.add(
                                egui::Label::new(
                                    RichText::new("Обход работает, пока открыт лаунчер. Служба не установлена")
                                        .color(Color32::from_gray(165))
                                        .size(12.5),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });
        });
        if stop {
            self.stop_trial(ctx);
        }
        if install {
            let idx = self.strategies.iter().position(|s| s.id == id);
            self.stop_trial_now();
            if let Some(i) = idx {
                self.selected = Some(i);
            }
            self.install_selected(ctx);
        }
    }
}
