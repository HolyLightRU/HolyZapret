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
    WarpWithoutZapret,
    ResetAll,
    MoveFolder,
    DeleteOld,
    StopSharing,
    QuitSharing,
    WarpBlocksCheck,
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
            Confirm::ResetAll => (
                "Сбросить настройки и списки?",
                "Вернутся настройки по умолчанию, очистятся «Мои домены», «Мои IP», исключения, мои сайты, правила для сайтов, игровой фильтр, результаты подбора и настройки Telegram-прокси. Установленная служба и автозапуск останутся, служба перезапустится с чистыми списками.",
                "Сбросить",
            ),
            Confirm::WarpBlocksCheck => (
                "Сначала отключите WARP",
                "Пока WARP пропускает весь трафик через Cloudflare, сайты открываются при любом приёме, и подбор с проверками покажут, что подходит всё. Отключите WARP, запустите подбор, а после него включите WARP обратно.",
                "Отключить WARP",
            ),
            Confirm::StopSharing => ("Выключить раздачу на телефон?", "SHARE", "Выключить"),
            Confirm::QuitSharing => ("Выйти из HolyZapret?", "SHARE", "Выйти"),
            Confirm::DeleteOld => (
                "Удалить старую папку HolyZapret?",
                "OLD",
                "Удалить",
            ),
            Confirm::MoveFolder => (
                "Перенести HolyZapret?",
                "Скопирую папку в новую без кириллицы на системном диске, переустановлю оттуда службу и автозапуск и перезапущу лаунчер. Старая папка останется, её можно будет удалить вручную. Ярлыки на рабочем столе нужно будет пересоздать.",
                "Перенести",
            ),
        };
        let old_text;
        let text = if c == Confirm::DeleteOld {
            let old = self.old_folder.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            old_text = format!(
                "HolyZapret уже работает из новой папки. В корзину отправятся только файлы HolyZapret из {old}: списки, движок и старый лаунчер. Ваши файлы в этой папке не тронутся, а пустая папка удалится."
            );
            old_text.as_str()
        } else if matches!(c, Confirm::StopSharing | Confirm::QuitSharing) {
            let n = crate::core::lanproxy::control::snapshot().map(|s| s.clients).unwrap_or(0);
            let who = if n > 0 { format!("К раздаче подключались устройства: {n}. ") } else { String::new() };
            old_text = if c == Confirm::StopSharing {
                format!("{who}Пока лаунчер открыт, телефоны с автонастройкой прокси (PAC) пойдут в интернет напрямую. На телефонах, где прокси указан вручную, Wi-Fi перестанет работать. Чтобы вернуть интернет на телефоне: Настройки → Wi-Fi → ваша сеть → Изменить → Дополнительно → Прокси → Нет.")
            } else {
                format!("{who}После выхода раздача пропадёт полностью, и Wi-Fi на этих телефонах перестанет работать, пока лаунчер снова не запустят или пока на телефоне не уберут прокси. Чтобы вернуть интернет на телефоне: Настройки → Wi-Fi → ваша сеть → Изменить → Дополнительно → Прокси → Нет.")
            };
            old_text.as_str()
        } else {
            text
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
        if answer.is_none() && super::modal_close(ctx, &modal) {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.confirm = None;
                match c {
                    Confirm::PowerOff | Confirm::RemoveService => {
                        self.spawn(ctx, |_, tx| match service::remove() {
                            Ok(m) => notify(tx, Lvl::Ok, m),
                            Err(m) => notify(tx, Lvl::Err, m),
                        });
                    }
                    Confirm::WarpWithoutZapret => self.warp_do(ctx, WarpAction::Connect),
                    Confirm::ResetAll => self.reset_all(ctx),
                    Confirm::MoveFolder => self.move_folder(ctx),
                    Confirm::DeleteOld => self.delete_old_folder(ctx),
                    Confirm::StopSharing => match crate::core::lanproxy::control::enable(false) {
                        Ok(m) => self.toast(ctx, Lvl::Ok, m),
                        Err(e) => self.toast(ctx, Lvl::Err, e.to_string()),
                    },
                    Confirm::QuitSharing => crate::tray::exit(),
                    Confirm::WarpBlocksCheck => self.warp_do(ctx, WarpAction::Disconnect),
                }
            }
            Some(false) => {
                self.confirm = None;
                if c == Confirm::DeleteOld {
                    self.old_folder = None;
                }
            }
            None => {}
        }
    }
}
