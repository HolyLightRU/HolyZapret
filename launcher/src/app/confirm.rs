use super::warp_ui::WarpAction;
use super::{App, notify};
use crate::core::service;
use crate::log::Lvl;
use crate::theme::RED;
use eframe::egui::{self, Color32, CornerRadius, RichText};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    PowerOff,
    RemoveService,
    RemoveFlowseal,
    Update,
    WarpWithoutZapret,
}

impl App {
    pub(super) fn draw_confirm(&mut self, ctx: &egui::Context) {
        let Some(c) = self.confirm else { return };
        let (title, text, action) = match c {
            Confirm::PowerOff => (
                "Выключить защиту?",
                "Служба zapret будет остановлена и удалена. Включить обратно можно кнопкой старта.",
                "Выключить",
            ),
            Confirm::RemoveService => (
                "Удалить службу?",
                "Служба zapret и драйвер WinDivert будут остановлены и удалены, обход перестанет работать.",
                "Удалить",
            ),
            Confirm::WarpWithoutZapret => (
                "Подключить WARP без защиты?",
                "Защита HolyZapret выключена. Без неё в России WARP обычно не может подключиться. Лучше сначала включить защиту кнопкой старта.",
                "Подключить",
            ),
            Confirm::Update => (
                "Обновить HolyZapret?",
                "Скачаю последний релиз с GitHub и заменю стратегии, списки и бинарники. Свои списки, игровой фильтр, папка flowseal и launcher не трогаются.",
                "Обновить",
            ),
            Confirm::RemoveFlowseal => (
                "Удалить стратегии Flowseal?",
                "Папка flowseal со стратегиями, движком и списками будет удалена. Скачать её снова можно в Инструментах.",
                "Удалить",
            ),
        };
        let mut answer = None;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        let modal = egui::Modal::new(egui::Id::NULL.with("confirm")).frame(frame).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(title).size(17.0).strong());
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(text).color(Color32::from_gray(190))).wrap());
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let danger = egui::Button::new(RichText::new(action).color(Color32::WHITE)).fill(RED.gamma_multiply(0.7));
                if ui.add(danger).clicked() {
                    answer = Some(true);
                }
                if ui.button("Отмена").clicked() {
                    answer = Some(false);
                }
            });
        });
        if answer.is_none() && modal.should_close() {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.confirm = None;
                match c {
                    Confirm::PowerOff | Confirm::RemoveService => {
                        self.spawn(ctx, |_, tx| notify(tx, Lvl::Ok, service::remove()));
                    }
                    Confirm::RemoveFlowseal => self.flowseal_action(ctx, false),
                    Confirm::Update => self.run_update(ctx),
                    Confirm::WarpWithoutZapret => self.warp_do(ctx, WarpAction::Connect),
                }
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }
}
