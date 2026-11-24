use super::{App, Msg, plural, result_msg};
use crate::core::diagnostics::{self, Finding, Level};
use crate::theme::{self, GREEN, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::collections::HashSet;

#[derive(Default)]
pub struct DiagState {
    pub findings: Vec<Finding>,
    pub ran: bool,
    pub running: bool,
    open: HashSet<String>,
}

impl App {
    pub(super) fn run_diag(&mut self, ctx: &egui::Context) {
        let Some(p) = self.paths.clone() else { return };
        self.diag.running = true;
        self.diag.ran = true;
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Diag(diagnostics::run(&p)));
            ctx2.request_repaint();
        });
    }

    pub(super) fn apply_fix(&mut self, ctx: &egui::Context, fix: diagnostics::Fix) {
        if fix == diagnostics::Fix::MoveFolder {
            self.confirm = Some(super::confirm::Confirm::MoveFolder);
            return;
        }
        self.diag.running = true;
        self.spawn(ctx, move |p, tx| {
            result_msg(tx, diagnostics::apply(&fix), "Не удалось исправить");
            let _ = tx.send(Msg::Diag(diagnostics::run(p)));
        });
    }

    pub(super) fn diag_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let views = vec!["Система".to_string(), "Программа".to_string()];
        theme::segmented(ui, &views, &mut self.check.view);
        ui.add_space(8.0);
        if self.check.view == 1 {
            self.check_tab(ui, ctx);
            return;
        }
        if !self.diag.ran && !self.diag.running && self.paths.is_some() {
            self.run_diag(ctx);
        }
        let bad = self.diag.findings.iter().filter(|f| f.level == Level::Bad).count();
        let warn = self.diag.findings.iter().filter(|f| f.level == Level::Warn).count();
        let mut rerun = false;
        let mut fix = None;
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Диагностика").size(17.0).strong());
                ui.add_space(4.0);
                if self.diag.running {
                    ui.spinner();
                    ui.label(RichText::new("Проверяю…").weak());
                } else if !self.diag.findings.is_empty() {
                    if bad > 0 {
                        theme::chip(ui, &format!("{bad} {}", plural(bad, "проблема", "проблемы", "проблем")), RED);
                    }
                    if warn > 0 {
                        theme::chip(ui, &format!("{warn} {}", plural(warn, "предупреждение", "предупреждения", "предупреждений")), YELLOW);
                    }
                    if bad + warn == 0 {
                        theme::chip(ui, "Всё в порядке", GREEN);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!self.diag.running, |ui| {
                        if ui.add(theme::Primary("Проверить снова".into())).clicked() {
                            rerun = true;
                        }
                    });
                });
            });
            let h = (ui.available_height() - 96.0).max(120.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                let issues: Vec<Finding> = {
                    let mut v: Vec<Finding> = self.diag.findings.iter().filter(|f| f.level != Level::Ok).cloned().collect();
                    v.sort_by_key(|f| f.level != Level::Bad);
                    v
                };
                if !issues.is_empty() {
                    ui.label(RichText::new("Требует внимания").color(Color32::from_gray(150)).size(12.5));
                    for f in &issues {
                        if let Some(x) = self.issue_row(ui, f) {
                            fix = Some(x);
                        }
                    }
                    ui.add_space(6.0);
                }
                let oks: Vec<&Finding> = self.diag.findings.iter().filter(|f| f.level == Level::Ok).collect();
                if !oks.is_empty() {
                    ui.label(RichText::new(format!("В порядке · {}", oks.len())).color(Color32::from_gray(150)).size(12.5));
                    egui::Grid::new("diag_ok").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                        for (i, f) in oks.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(theme::icon::CHECK).font(theme::icon_font(12.0)).color(GREEN));
                                ui.label(RichText::new(&f.title).color(Color32::from_gray(190)).size(13.5));
                            });
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                }
            });
        });
        let mut clear = false;
        theme::card(ui, |ui| {
            theme::row(ui, 46.0, |ui| {
                ui.label(RichText::new(theme::icon::CHAT).font(theme::icon_font(18.0)).color(Color32::from_gray(200)));
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Кэш Discord").strong());
                    ui.label(
                        RichText::new("Закроет Discord и очистит Cache, Code Cache и GPUCache у Stable, PTB, Canary и Development")
                            .weak()
                            .size(12.5),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("Очистить")).clicked() {
                        clear = true;
                    }
                });
            });
        });
        if rerun {
            self.run_diag(ctx);
        }
        if let Some(f) = fix {
            if !self.busy {
                self.apply_fix(ctx, f);
            }
        }
        if clear {
            self.spawn(ctx, |_, tx| result_msg(tx, diagnostics::clear_discord_cache(), "Кэш Discord"));
        }
    }

    pub(super) fn issue_row(&mut self, ui: &mut egui::Ui, f: &Finding) -> Option<diagnostics::Fix> {
        let color = if f.level == Level::Bad { RED } else { YELLOW };
        let glyph = if f.level == Level::Bad { theme::icon::ERROR } else { theme::icon::WARNING };
        let expandable = !f.hint.is_empty() || f.link.is_some();
        let open = self.diag.open.contains(&f.title);
        let mut toggle = false;
        let mut fix = None;
        egui::Frame::new()
            .fill(color.gamma_multiply(0.07))
            .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.25)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::row(ui, 34.0, |ui| {
                    ui.label(RichText::new(glyph).font(theme::icon_font(15.0)).color(color));
                    ui.add_space(2.0);
                    let title = ui.add(egui::Label::new(RichText::new(&f.title).color(Color32::WHITE)).sense(egui::Sense::click()));
                    if expandable && title.clicked() {
                        toggle = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if expandable {
                            let g = if open { theme::icon::CHEVRON_UP } else { theme::icon::CHEVRON_DOWN };
                            if theme::icon_button(ui, g, if open { "Свернуть" } else { "Подробнее" }).clicked() {
                                toggle = true;
                            }
                        }
                        if let Some(x) = &f.fix {
                            ui.add_enabled_ui(!self.busy, |ui| {
                                if ui.add(theme::Primary(x.label().into())).clicked() {
                                    fix = Some(x.clone());
                                }
                            });
                        }
                    });
                });
                if open {
                    ui.add_space(2.0);
                    if !f.hint.is_empty() {
                        ui.label(RichText::new(&f.hint).color(Color32::from_gray(185)));
                    }
                    if let Some(url) = &f.link {
                        ui.hyperlink_to("Подробнее на GitHub", url);
                    }
                    ui.add_space(6.0);
                }
            });
        if toggle {
            if open {
                self.diag.open.remove(&f.title);
            } else {
                self.diag.open.insert(f.title.clone());
            }
        }
        fix
    }
}
