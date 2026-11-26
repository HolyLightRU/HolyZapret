use super::home_ray::tile;
use super::{App, Tab};
use crate::core::diagnostics::Level;
use crate::core::service::ServiceState;
use crate::theme::{self, CYAN, GREEN, PINK, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

const STEPS: [(&str, &str); 3] = [
    ("Диагностика", "Проверим, не мешает ли что-то обходу"),
    ("Подбор", "Сравним приёмы на ваших сайтах"),
    ("Включение", "Поставим службу HolyZapret"),
];

#[derive(Default)]
pub struct WizardState {
    pub step: usize,
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

fn emblem(ui: &mut egui::Ui, step: usize, busy: bool) {
    let d = 92.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
    let c = rect.center();
    let r = d / 2.0 - 8.0;
    let p = ui.painter();
    for (k, a) in [(12.0, 0.03), (8.0, 0.05), (4.0, 0.08)] {
        p.circle_filled(c, r + k, theme::grad(0.5).gamma_multiply(a));
    }
    p.circle_filled(c, r, Color32::from_white_alpha(12));
    let n = 72;
    let tau = std::f32::consts::TAU;
    let lit_until = (step + 1) as f32 / STEPS.len() as f32;
    let spin = if busy { ui.input(|i| i.time) as f32 * 3.0 } else { 0.0 };
    for i in 0..n {
        let f = i as f32 / n as f32;
        let a0 = -tau / 4.0 + f * tau;
        let a1 = a0 + tau / n as f32 + 0.01;
        let x = f / lit_until;
        let col = if f < lit_until { theme::grad(if x < 0.5 { x * 2.0 } else { 2.0 - x * 2.0 }) } else { Color32::from_white_alpha(38) };
        p.line_segment([c + Vec2::angled(a0) * r, c + Vec2::angled(a1) * r], Stroke::new(3.5, col));
    }
    if busy {
        let a = -tau / 4.0 + spin;
        p.circle_filled(c + Vec2::angled(a) * r, 4.0, Color32::WHITE);
        ui.ctx().request_repaint();
    }
    let glyph = [theme::icon::DIAGNOSTIC, theme::icon::PICK, theme::icon::CHECK][step.min(2)];
    p.text(c, egui::Align2::CENTER_CENTER, glyph, theme::icon_font(28.0), Color32::WHITE);
}

fn step_tile(ui: &mut egui::Ui, i: usize, current: usize) -> egui::Response {
    let (name, hint) = STEPS[i];
    let (status, color) = match i.cmp(&current) {
        std::cmp::Ordering::Less => ("Готово", GREEN),
        std::cmp::Ordering::Equal => ("Сейчас", PINK),
        std::cmp::Ordering::Greater => ("Впереди", Color32::from_gray(130)),
    };
    let frame = egui::Frame::new()
        .fill(Color32::from_white_alpha(if i == current { 12 } else { 6 }))
        .stroke(Stroke::new(1.0, if i == current { PINK.gamma_multiply(0.55) } else { Color32::TRANSPARENT }))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 10));
    let resp = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.label(muted(format!("Шаг {}", i + 1)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(status).color(color).size(12.5));
                    if i < current {
                        ui.label(RichText::new(theme::icon::CHECK).font(theme::icon_font(11.0)).color(color));
                    }
                });
            });
            let tc = if i > current { Color32::from_gray(170) } else { Color32::WHITE };
            ui.label(RichText::new(name).size(18.0).strong().color(tc));
            ui.add(egui::Label::new(muted(hint)).truncate());
        })
        .response;
    if i < current { resp.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Вернуться к этому шагу") } else { resp }
}

fn banner(ui: &mut egui::Ui, color: Color32, glyph: &str, title: &str, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.07))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.25)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).font(theme::icon_font(18.0)).color(color));
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(title).size(15.0).strong().color(Color32::WHITE));
                    if !text.is_empty() {
                        ui.add(egui::Label::new(RichText::new(text).color(Color32::from_gray(185))).wrap());
                    }
                });
            });
        });
}

fn tiles(ui: &mut egui::Ui, items: &[(&str, String, &str, Color32)]) {
    ui.columns(items.len(), |cols| {
        for (col, (label, value, sub, color)) in cols.iter_mut().zip(items) {
            tile(col, label, value, sub, *color);
        }
    });
}

fn busy_row(ui: &mut egui::Ui, text: &str) {
    theme::row(ui, 32.0, |ui| {
        ui.spinner();
        ui.label(text);
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
        let step = self.wizard.step.min(STEPS.len() - 1);
        let working = self.picker.running() || self.diag.running || self.busy;
        let mut skip = false;
        let mut jump = None;
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                emblem(ui, step, working);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 190.0).max(200.0));
                    ui.add_space(14.0);
                    ui.label(RichText::new("Первая настройка").size(22.0).strong().color(Color32::WHITE));
                    ui.add(
                        egui::Label::new(RichText::new(format!("Три коротких шага, и защита включится. Шаг {} из 3: {}", step + 1, STEPS[step].0)).color(Color32::from_gray(160)))
                            .truncate(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Пропустить мастер").on_hover_text("Можно вернуться к нему во вкладке «Инструменты»").clicked() {
                        skip = true;
                    }
                });
            });
        });
        ui.add_space(8.0);
        ui.columns(STEPS.len(), |cols| {
            for (i, col) in cols.iter_mut().enumerate() {
                if step_tile(col, i, step).clicked() && !working {
                    jump = Some(i);
                }
            }
        });
        ui.add_space(8.0);
        let mut next = false;
        let mut back = false;
        theme::card(ui, |ui| {
            let h = (ui.available_height() - 60.0).max(160.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| match step {
                0 => self.wizard_diag(ui, ctx),
                1 => self.wizard_pick(ui, ctx),
                _ => self.wizard_install(ui, ctx),
            });
            ui.add_space(4.0);
            theme::row(ui, 44.0, |ui| {
                if step > 0 && ui.add_enabled(!working, egui::Button::new("Назад")).clicked() {
                    back = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let last = step + 1 == STEPS.len();
                    let label = if last { "Завершить" } else { "Далее" };
                    ui.add_enabled_ui(!(self.picker.running() || self.diag.running), |ui| {
                        if ui.add(theme::Primary(label.into())).clicked() {
                            next = true;
                        }
                    });
                });
            });
        });
        if skip {
            self.finish_wizard();
        } else if let Some(i) = jump {
            self.wizard.step = i;
        } else if back {
            self.wizard.step = step - 1;
        } else if next {
            if step + 1 == STEPS.len() {
                self.finish_wizard();
            } else {
                self.wizard.step = step + 1;
            }
        }
    }

    fn step_header(ui: &mut egui::Ui, title: &str, text: &str) {
        ui.label(RichText::new(title).size(17.0).strong());
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(text).color(Color32::from_gray(185))).wrap());
        ui.add_space(10.0);
    }

    fn wizard_diag(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.diag.ran && !self.diag.running && self.paths.is_some() {
            self.run_diag(ctx);
        }
        Self::step_header(ui, "Диагностика", "Проверяю службы Windows, другие программы обхода, VPN и настройки сети: всё, что может помешать обходу.");
        if self.diag.running {
            busy_row(ui, "Проверяю…");
            return;
        }
        let total = self.diag.findings.len();
        let warn = self.diag.findings.iter().filter(|f| f.level == Level::Warn).count();
        let bad = self.diag.findings.iter().filter(|f| f.level == Level::Bad).count();
        let ok = total - warn - bad;
        tiles(
            ui,
            &[
                ("Проверок", total.to_string(), "за пару секунд", Color32::WHITE),
                ("В порядке", ok.to_string(), "можно не трогать", GREEN),
                ("Предупреждений", warn.to_string(), "идти дальше можно", if warn > 0 { YELLOW } else { Color32::from_gray(170) }),
                ("Проблем", bad.to_string(), "лучше исправить", if bad > 0 { RED } else { Color32::from_gray(170) }),
            ],
        );
        ui.add_space(10.0);
        let issues: Vec<_> = self.diag.findings.iter().filter(|f| f.level != Level::Ok).cloned().collect();
        if issues.is_empty() {
            banner(ui, GREEN, theme::icon::CHECK, "Всё в порядке", "Ничего не мешает обходу. Нажмите «Далее».");
            return;
        }
        let mut fix = None;
        for f in &issues {
            if let Some(x) = self.issue_row(ui, f) {
                fix = Some(x);
            }
            ui.add_space(4.0);
        }
        if let Some(f) = fix {
            if !self.busy {
                self.apply_fix(ctx, f);
            }
        }
    }

    fn wizard_pick(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let n = self.picker.optimal_count(self.paths.as_ref(), &self.strategies);
        Self::step_header(
            ui,
            "Подбор приёма",
            "RayOfLight сам подбирает приём для каждого сайта и запоминает рабочий, поэтому этот шаг можно пропустить. Подбор сравнит приёмы на Discord, YouTube и других сайтах и покажет лучший.",
        );
        tiles(
            ui,
            &[
                ("Приёмов", n.to_string(), "сравнит по очереди", Color32::WHITE),
                ("Займёт", "≈ 2 мин".to_string(), "можно отойти", CYAN),
                ("Если пропустить", "Самоподстройка".to_string(), "RayOfLight подберёт приём сам", PINK),
            ],
        );
        ui.add_space(10.0);
        if self.picker.running() {
            let (frac, label) = self.picker.progress();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(5), Color32::from_white_alpha(18));
            if frac > 0.01 {
                let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height()));
                theme::gradient_rect(ui.painter(), fill, 5.0, PINK, CYAN);
            }
            busy_row(ui, &label);
            return;
        }
        match self.picker.best_pick() {
            Some((None, _)) => banner(ui, GREEN, theme::icon::CHECK, "Обход не нужен", "Без обхода всё и так открывается: службу можно не ставить."),
            Some((Some(_), name)) => banner(ui, GREEN, theme::icon::CHECK, &format!("Лучший вариант: {name}"), "Он и будет включён на следующем шаге."),
            None => {
                theme::row(ui, 40.0, |ui| {
                    ui.add_enabled_ui(!self.busy && self.paths.is_some(), |ui| {
                        if ui.add(theme::Primary("Начать подбор".into())).clicked() {
                            self.picker.set_quick();
                            self.start_pick(ctx);
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(muted("или «Далее»: тогда включится RayOfLight с самоподстройкой"));
                });
            }
        }
    }

    fn wizard_install(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let best = self.picker.best_pick();
        if let Some((None, _)) = best {
            Self::step_header(ui, "Включение", "");
            banner(ui, GREEN, theme::icon::CHECK, "Обход не нужен", "Без него сайты открываются не хуже. Мастер можно завершать.");
            return;
        }
        let target = best
            .and_then(|(p, _)| p)
            .and_then(|p| self.strategies.iter().position(|s| s.path == p))
            .or(self.selected)
            .or_else(|| self.strategies.iter().position(|s| s.id == crate::core::ray::DEFAULT_ID));
        let Some(idx) = target else {
            Self::step_header(ui, "Включение", "");
            banner(ui, RED, theme::icon::ERROR, "Нет файла bin\\RayOfLight.exe", "Скачайте HolyZapret заново.");
            return;
        };
        let s = self.strategies[idx].clone();
        let installed = self
            .status
            .as_ref()
            .is_some_and(|st| st.service == ServiceState::Running && st.foreign_dir().is_none() && st.active_strategy().is_some_and(|n| s.matches(n)));
        Self::step_header(ui, "Включение", "Служба HolyZapret запускается вместе с Windows, даже если лаунчер закрыт.");
        let mut press = false;
        ui.horizontal(|ui| {
            let resp = theme::power_button(ui, installed, self.busy).on_hover_text(if installed { "Защита уже включена" } else { "Включить защиту" });
            if resp.clicked() && !installed && !self.busy {
                press = true;
            }
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(17.0);
                let (title, color) = if installed {
                    ("Защита работает", GREEN)
                } else if self.busy {
                    ("Включаю…", Color32::WHITE)
                } else {
                    ("Нажмите кнопку, чтобы включить защиту", Color32::WHITE)
                };
                ui.label(RichText::new(title).size(19.0).strong().color(color));
                ui.add(egui::Label::new(RichText::new(crate::core::ray::install_label(&s)).color(Color32::from_gray(160))).truncate());
            });
        });
        if press {
            self.selected = Some(idx);
            self.install_selected(ctx);
        }
        if installed {
            ui.add_space(10.0);
            ui.label(muted("Лаунчер тоже можно запускать с Windows, чтобы видеть статус в трее:"));
            ui.add_space(4.0);
            self.autostart_row(ui, ctx);
            self.auto_update_row(ui);
        }
    }
}
