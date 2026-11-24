use super::{App, Tab};
use crate::theme;
use eframe::egui::{self, Color32, CornerRadius, RichText};

impl App {
    pub(super) fn draw_crash(&mut self, ctx: &egui::Context) {
        let Some(text) = self.crash.clone() else { return };
        if self.crash_seen || self.tab == Tab::Wizard || self.confirm.is_some() || self.unpack.is_some() || self.whats_new.is_some() || self.update.open {
            return;
        }
        let mut save = false;
        let mut close = false;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        let modal = egui::Modal::new(egui::Id::NULL.with("crash")).frame(frame).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Лаунчер в прошлый раз закрылся с ошибкой").size(17.0).strong());
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Это ошибка в самом лаунчере, обход и служба от неё не зависят. Сохраните отчёт и отправьте его разработчику: в нём будет место, где произошла ошибка.")
                        .color(Color32::from_gray(190)),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(Color32::from_black_alpha(60))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(crate::core::crash::headline(&text)).monospace().size(12.0).color(Color32::from_gray(200))).wrap());
                });
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::Primary("Сохранить отчёт".into())).clicked() {
                    save = true;
                }
                if ui.button("Закрыть").clicked() {
                    close = true;
                }
            });
        });
        if super::modal_close(ctx, &modal) {
            close = true;
        }
        if save {
            self.crash_seen = true;
            self.open_report();
        }
        if close {
            self.crash_seen = true;
        }
    }
}
