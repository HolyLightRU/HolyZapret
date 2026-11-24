use super::{App, Tab};
use crate::core::diagnostics::Level;
use crate::core::service::ServiceState;
use crate::theme::{self, CYAN, GREEN, PINK, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Sense, Vec2};

const STEPS: [&str; 3] = ["Диагностика", "Подбор", "Установка"];

#[derive(Default)]
pub struct WizardState {
    pub step: usize,
}

fn stepper(ui: &mut egui::Ui, current: usize) {
    theme::row(ui, 48.0, |ui| {
        for (i, title) in STEPS.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            let p = ui.painter();
            if i <= current {
                theme::gradient_rect(p, rect, 15.0, PINK, CYAN);
            } else {
                p.circle_stroke(rect.center(), 14.0, egui::Stroke::new(1.5, Color32::from_white_alpha(60)));
            }
            let text = if i < current { theme::icon::CHECK.to_string() } else { (i + 1).to_string() };
            let font = if i < current { theme::icon_font(13.0) } else { FontId::proportional(14.0) };
            let col = if i <= current { Color32::from_rgb(30, 18, 36) } else { Color32::from_gray(170) };
            p.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, col);
            ui.add_space(2.0);
            let tc = if i == current { Color32::WHITE } else { Color32::from_gray(150) };
            ui.label(RichText::new(*title).color(tc));
            if i + 1 < STEPS.len() {
                ui.add_space(6.0);
                let (line, _) = ui.allocate_exact_size(Vec2::new(48.0, 2.0), Sense::hover());
                let c = if i < current { PINK } else { Color32::from_white_alpha(40) };
                ui.painter().rect_filled(line, CornerRadius::same(1), c);
                ui.add_space(6.0);
            }
        }
    });
}

impl App {
    pub(super) fn open_wizard(&mut self) {
        self.wizard.step = 0;
        self.tab = Tab::Wizard;
    }

    fn finish_wizard(&mut self) {
        self.wizard_done = true;
        self.tab = Tab::Main;
    }

    pub(super) fn wizard_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut next = false;
        let mut back = false;
        let mut skip = false;
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Первая настройка").size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Пропустить мастер").clicked() {
                        skip = true;
                    }
                });
            });
            stepper(ui, self.wizard.step);
            ui.separator();
            let h = (ui.available_height() - 56.0).max(160.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| match self.wizard.step {
                0 => self.wizard_diag(ui, ctx),
                1 => self.wizard_pick(ui, ctx),
                _ => self.wizard_install(ui, ctx),
            });
            theme::row(ui, 44.0, |ui| {
                if self.wizard.step > 0 && ui.button("Назад").clicked() {
                    back = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let last = self.wizard.step + 1 == STEPS.len();
                    let label = if last { "Завершить" } else { "Далее" };
                    let blocked = self.picker.running() || self.diag.running;
                    ui.add_enabled_ui(!blocked, |ui| {
                        if ui.add(theme::Primary(label.into())).clicked() {
                            next = true;
                        }
                    });
                });
            });
        });
        if skip {
            self.finish_wizard();
        } else if back {
            self.wizard.step -= 1;
        } else if next {
            if self.wizard.step + 1 == STEPS.len() {
                self.finish_wizard();
            } else {
                self.wizard.step += 1;
            }
        }
    }

    fn wizard_diag(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.diag.ran && !self.diag.running && self.paths.is_some() {
            self.run_diag(ctx);
        }
        ui.label(RichText::new("Проверяю, не мешает ли что-то обходу: службы Windows, другие программы обхода, настройки сети.").color(Color32::from_gray(185)));
        ui.add_space(6.0);
        if self.diag.running {
            theme::row(ui, 32.0, |ui| {
                ui.spinner();
                ui.label("Проверяю…");
            });
            return;
        }
        let issues: Vec<_> = self.diag.findings.iter().filter(|f| f.level != Level::Ok).cloned().collect();
        if issues.is_empty() {
            ui.label(RichText::new("Всё в порядке, можно идти дальше").color(GREEN));
        } else {
            let bad = issues.iter().filter(|f| f.level == Level::Bad).count();
            let text = if bad > 0 { "Есть проблемы: лучше исправить их до подбора" } else { "Есть предупреждения, но идти дальше можно" };
            ui.label(RichText::new(text).color(if bad > 0 { RED } else { YELLOW }));
            let mut fix = None;
            for f in &issues {
                if let Some(x) = self.issue_row(ui, f) {
                    fix = Some(x);
                }
            }
            if let Some(f) = fix {
                if !self.busy {
                    self.apply_fix(ctx, f);
                }
            }
        }
    }

    fn wizard_pick(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let n = self.picker.optimal_count(self.paths.as_ref(), &self.strategies);
        ui.label(
            RichText::new(format!(
                "Лаунчер по очереди запустит {n} проверенных стратегий и проверит Discord, YouTube и другие сайты. Это займёт около 2 минут."
            ))
            .color(Color32::from_gray(185)),
        );
        ui.add_space(6.0);
        if self.picker.running() {
            let (frac, label) = self.picker.progress();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(5), Color32::from_white_alpha(18));
            if frac > 0.01 {
                let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height()));
                theme::gradient_rect(ui.painter(), fill, 5.0, PINK, CYAN);
            }
            theme::row(ui, 30.0, |ui| {
                ui.spinner();
                ui.label(label);
            });
            return;
        }
        match self.picker.best_pick() {
            Some((None, _)) => {
                ui.label(RichText::new("Без обхода всё и так открывается: устанавливать службу не нужно").color(GREEN));
            }
            Some((Some(_), name)) => {
                ui.label(RichText::new(format!("Лучшая стратегия: {name}")).color(GREEN).size(16.0));
            }
            None => {
                ui.add_enabled_ui(!self.busy && self.paths.is_some(), |ui| {
                    if ui.add(theme::Primary("Начать подбор".into())).clicked() {
                        self.picker.set_quick();
                        self.start_pick(ctx);
                    }
                });
                ui.label(RichText::new("Можно пропустить и выбрать стратегию вручную на Главной").weak().size(12.5));
            }
        }
    }

    fn wizard_install(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let best = self.picker.best_pick();
        if let Some((None, _)) = best {
            ui.label(RichText::new("Обход не нужен: без стратегии сайты открываются не хуже. Мастер можно завершать").color(GREEN));
            return;
        }
        let target = best
            .and_then(|(p, _)| p)
            .and_then(|p| self.strategies.iter().position(|s| s.path == p))
            .or(self.selected)
            .or_else(|| self.strategies.iter().position(|s| s.name == "general"));
        let Some(idx) = target else {
            ui.label(RichText::new("Стратегии не найдены").color(RED));
            return;
        };
        let s = self.strategies[idx].clone();
        let installed = self
            .status
            .as_ref()
            .is_some_and(|st| st.service == ServiceState::Running && st.active_strategy().is_some_and(|n| s.matches(n)));
        if installed {
            ui.label(RichText::new(format!("Готово: защита работает со стратегией «{}»", s.name)).color(GREEN).size(16.0));
            ui.label(RichText::new("Служба запускается вместе с Windows. Лаунчер можно тоже запускать, чтобы видеть статус в трее:").weak());
            ui.add_space(6.0);
            self.autostart_row(ui, ctx);
            self.auto_update_row(ui);
            return;
        }
        ui.label(RichText::new(format!("Установим службу со стратегией «{}». Она будет запускаться вместе с Windows.", s.name)).color(Color32::from_gray(185)));
        ui.add_space(8.0);
        ui.add_enabled_ui(!self.busy, |ui| {
            if ui.add(theme::Primary(format!("Установить «{}»", s.name))).clicked() {
                self.selected = Some(idx);
                self.install_selected(ctx);
            }
        });
        if self.busy {
            theme::row(ui, 30.0, |ui| {
                ui.spinner();
                ui.label("Устанавливаю…");
            });
        }
    }
}
