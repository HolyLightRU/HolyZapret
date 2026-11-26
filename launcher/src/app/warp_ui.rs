use super::{App, Msg, result_msg};
use crate::core::service::ServiceState;
use crate::core::warp::{self, Conn, Mode};
use crate::theme::{self, CYAN, GREEN, YELLOW};
use eframe::egui::{self, Color32, RichText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpAction {
    Connect,
    Disconnect,
    Mode(Mode),
}

fn conn_chip(ui: &mut egui::Ui, s: &warp::State) {
    if s.needs_setup {
        theme::chip(ui, "Нужна настройка", YELLOW);
        return;
    }
    match s.conn {
        Conn::Connected => theme::chip(ui, "Подключён", GREEN),
        Conn::Connecting => theme::chip(ui, "Подключается…", YELLOW),
        Conn::Disconnected => theme::chip(ui, "Отключён", Color32::from_gray(160)),
        Conn::Unknown => theme::chip(ui, "Состояние неизвестно", Color32::from_gray(160)),
    };
}

impl App {
    pub(super) fn warp_do(&mut self, ctx: &egui::Context, a: WarpAction) {
        self.spawn(ctx, move |_, tx| {
            let r = match a {
                WarpAction::Connect => warp::connect().map(|_| "WARP подключается".to_string()),
                WarpAction::Disconnect => warp::disconnect().map(|_| "WARP отключён".to_string()),
                WarpAction::Mode(m) => warp::set_mode(m).map(|_| format!("Режим WARP: {}", m.label())),
            };
            result_msg(tx, r, "WARP");
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = tx.send(Msg::Warp(warp::query()));
        });
    }

    pub(super) fn warp_proxy_on(&mut self, ctx: &egui::Context) {
        let mode = self.warp.as_ref().map(|w| w.mode);
        self.spawn(ctx, move |_, tx| {
            let r = (|| {
                if mode != Some(Mode::Proxy) {
                    warp::set_mode(Mode::Proxy)?;
                }
                warp::connect()?;
                Ok("WARP подключается в режиме прокси: через него пойдёт только Telegram, остальное как обычно через обход".to_string())
            })();
            result_msg(tx, r, "WARP");
            for _ in 0..6 {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let s = warp::query();
                let done = s.conn == Conn::Connected;
                let _ = tx.send(Msg::Warp(s));
                if done {
                    break;
                }
            }
        });
    }

    fn warp_connect(&mut self, ctx: &egui::Context) {
        let protected = self.status.as_ref().is_some_and(|s| s.service == ServiceState::Running);
        if protected {
            self.warp_do(ctx, WarpAction::Connect);
        } else {
            self.confirm = Some(super::confirm::Confirm::WarpWithoutZapret);
        }
    }

    fn warp_buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, s: &warp::State) {
        ui.add_enabled_ui(!self.busy, |ui| {
            if s.needs_setup {
                if ui.button("Открыть WARP").clicked() {
                    warp::open_app();
                }
            } else if matches!(s.conn, Conn::Connected | Conn::Connecting) {
                if ui.button("Отключить").clicked() {
                    self.warp_do(ctx, WarpAction::Disconnect);
                }
            } else if ui.button("Подключить").clicked() {
                self.warp_connect(ctx);
            }
        });
    }

    pub(super) fn warp_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(s) = self.warp.clone().filter(|w| w.installed) else { return };
        theme::card(ui, |ui| {
            theme::row(ui, 34.0, |ui| {
                ui.label(RichText::new(theme::icon::CLOUD).font(theme::icon_font(18.0)).color(CYAN));
                ui.add_space(2.0);
                ui.label(RichText::new("Cloudflare WARP").strong());
                conn_chip(ui, &s);
                ui.label(RichText::new(s.mode.label()).color(Color32::from_gray(150)).size(12.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.warp_buttons(ui, ctx, &s);
                });
            });
        });
    }

    pub(super) fn warp_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = self.warp.clone().unwrap_or_else(warp::State::missing);
        theme::card(ui, |ui| {
            theme::row(ui, 32.0, |ui| {
                ui.label(RichText::new(theme::icon::CLOUD).font(theme::icon_font(17.0)).color(CYAN));
                ui.add_space(2.0);
                ui.label(RichText::new("Cloudflare WARP").size(15.5).strong());
                if s.installed {
                    conn_chip(ui, &s);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if s.installed {
                        self.warp_buttons(ui, ctx, &s);
                    } else if ui.button("Скачать с сайта Cloudflare").clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(warp::DOWNLOAD_URL));
                    }
                });
            });
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Туннель Cloudflare. Обычно включается поверх защиты HolyZapret: без неё в России WARP почти никогда не подключается.",
                    )
                    .color(Color32::from_gray(165))
                    .size(12.5),
                )
                .wrap(),
            );
            if !s.installed {
                ui.label(RichText::new("WARP не установлен").color(Color32::from_gray(150)).size(12.5));
                return;
            }
            let modes = [Mode::Warp, Mode::Doh, Mode::Proxy];
            let labels: Vec<String> = ["Весь трафик", "Только DNS", "Прокси :40000"].iter().map(|x| x.to_string()).collect();
            let mut idx = modes.iter().position(|m| *m == s.mode).unwrap_or(usize::MAX);
            let before = idx;
            theme::row(ui, 40.0, |ui| {
                theme::row_label(ui, "Режим", 70.0);
                theme::segmented(ui, &labels, &mut idx);
            });
            if idx != before && idx < modes.len() && !self.busy {
                self.warp_do(ctx, WarpAction::Mode(modes[idx]));
            }
            if s.mode == Mode::Proxy {
                ui.label(
                    RichText::new(format!(
                        "SOCKS5-прокси на 127.0.0.1:{}: укажите его в Telegram или браузере, остальной трафик идёт как обычно",
                        warp::PROXY_PORT
                    ))
                    .color(Color32::from_gray(150))
                    .size(12.5),
                );
            }
        });
    }
}
