use super::{App, Msg};
use crate::core::tgproxy::{self, control, route};
use crate::core::tgwarp;
use crate::core::warp::{Conn, Mode};
use crate::log::Lvl;
use crate::theme::{self, CYAN, GREEN, RED, YELLOW};
use eframe::egui::{self, Color32, RichText};

#[derive(Default)]
pub struct TgState {
    pub foreign: Vec<String>,
    pub client: Option<control::Client>,
    saved: Option<control::Saved>,
    port: String,
    domain: String,
}

impl TgState {
    pub fn forget(&mut self) {
        self.saved = None;
    }

    pub fn detect() -> TgState {
        TgState { client: control::desktop_client(), ..Default::default() }
    }
}

pub fn foreign_names() -> Vec<String> {
    let mut v: Vec<String> = control::foreign().into_iter().map(|p| p.name).collect();
    v.sort();
    v.dedup();
    v
}

enum Act {
    Toggle(bool),
    CloseForeign,
    Open,
    Copy,
    NewSecret,
    Save,
    Cloudflare(bool),
    Warp(bool),
    WarpProxy,
    Lan(bool),
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

fn tip(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(13.5));
    ui.add(egui::Label::new(muted(text)).wrap());
}

impl App {
    pub(super) fn tg_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.tg_card(ui, ctx);
            let client = self.tg.client.as_ref().map_or_else(|| "Telegram".to_string(), |c| c.name.clone());
            theme::card(ui, |ui| {
                theme::row(ui, 32.0, |ui| {
                    ui.label(RichText::new(theme::icon::INFO).font(theme::icon_font(17.0)).color(CYAN));
                    ui.add_space(2.0);
                    ui.label(RichText::new("Как подключить").size(15.5).strong());
                    theme::chip(ui, &format!("Найден {client}"), Color32::from_gray(160));
                });
                tip(
                    ui,
                    "1. Включите прокси и нажмите «Открыть в Telegram»",
                    "Telegram покажет окно с прокси 127.0.0.1, нажмите «Подключить». Если окно не появилось, скопируйте ссылку, отправьте её себе в «Избранное» и нажмите по ней.",
                );
                tip(
                    ui,
                    "2. Оставьте лаунчер работать",
                    "Прокси живёт внутри лаунчера: его можно свернуть в трей, а включить автозапуск в «Инструментах». Если лаунчер закрыт, Telegram не подключится.",
                );
                tip(
                    ui,
                    "Не грузятся фото или видео",
                    "Проверьте, что включён запасной путь через Cloudflare. Если не помогло, укажите свой домен по инструкции tg-ws-proxy (раздел CfProxy).",
                );
                tip(
                    ui,
                    "Telegram пишет «Подключение» и не подключается",
                    "Нажмите «Открыть в Telegram» ещё раз: после смены secret или порта старый прокси в Telegram перестаёт работать. Также в настройках типа подключения Telegram отключите подключение через IPv6: прокси его не поддерживает.",
                );
                ui.add_space(4.0);
            });
        });
    }

    fn tg_reload(&mut self) {
        let s = control::load();
        self.tg.port = s.config.port.to_string();
        self.tg.domain = s.config.cf_domain.clone();
        self.tg.saved = Some(s);
    }

    pub(super) fn tg_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.tg.saved.is_none() {
            self.tg_reload();
        }
        let Some(saved) = self.tg.saved.clone() else { return };
        let snap = control::snapshot();
        if snap.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        let mut act = None;
        theme::card(ui, |ui| {
            theme::row(ui, 32.0, |ui| {
                theme::icon_label(ui, theme::icon::TELEGRAM, 17.0, CYAN);
                ui.add_space(2.0);
                ui.label(RichText::new("Telegram-прокси").size(15.5).strong());
                match (&snap, saved.enabled) {
                    (Some(_), _) => theme::chip(ui, "Работает", GREEN),
                    (None, true) => theme::chip(ui, "Не запущен", RED),
                    (None, false) => theme::chip(ui, "Выключен", Color32::from_gray(160)),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut on = snap.is_some();
                    if theme::toggle(ui, &mut on).changed() {
                        act = Some(Act::Toggle(on));
                    }
                });
            });
            ui.add(
                egui::Label::new(muted(
                    "Ускоряет Telegram Desktop: трафик идёт через WebSocket к web.telegram.org, без сторонних серверов. Работает, пока открыт лаунчер, в том числе в трее. Основан на tg-ws-proxy от Flowseal.",
                ))
                .wrap(),
            );
            let warp = self.warp.clone().filter(|w| w.installed);
            let via_warp = tgwarp::enabled();
            theme::row(ui, 34.0, |ui| {
                ui.label(RichText::new("Telegram через WARP").color(Color32::from_gray(225)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut on = via_warp;
                    ui.add_enabled_ui(warp.is_some() || via_warp, |ui| {
                        if theme::toggle(ui, &mut on).changed() {
                            act = Some(Act::Warp(on));
                        }
                    });
                });
            });
            ui.add(
                egui::Label::new(muted(
                    "Если в вашем регионе Telegram не подключается даже через этот прокси, его трафик пойдёт через Cloudflare WARP, а всё остальное как обычно через обход. Касается и Telegram на телефоне через раздачу.",
                ))
                .wrap(),
            );
            match &warp {
                None => {
                    ui.label(RichText::new("Нужен Cloudflare WARP: скачайте его в Настройках").color(YELLOW).size(12.5));
                }
                Some(w) if via_warp && (w.mode != Mode::Proxy || (w.conn != Conn::Connected && !tgwarp::proxy_up())) => {
                    theme::row(ui, 34.0, |ui| {
                        ui.label(RichText::new("WARP должен быть подключён в режиме прокси").color(YELLOW).size(12.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(!self.busy, egui::Button::new("Подключить WARP")).clicked() {
                                act = Some(Act::WarpProxy);
                            }
                        });
                    });
                }
                _ => {}
            }
            theme::row(ui, 34.0, |ui| {
                ui.label(RichText::new("Для телефонов в домашней сети").color(Color32::from_gray(225)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut on = saved.config.lan;
                    if theme::toggle(ui, &mut on).changed() {
                        act = Some(Act::Lan(on));
                    }
                });
            });
            ui.add(
                egui::Label::new(muted(
                    "Telegram на телефоне подключается к этому прокси сам, без VPN и приложений: нужен только прокси в настройках Telegram. Путь тот же, что у Telegram на компьютере, в том числе через WARP, если он включён выше. Подключиться могут только устройства из вашей сети.",
                ))
                .wrap(),
            );
            if let (true, Some(link)) = (saved.config.lan, control::phone_link()) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    super::phone_tab::qr(ui, &link, 132.0);
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Наведите камеру телефона").strong());
                        ui.add(
                            egui::Label::new(muted(
                                "Откроется Telegram и предложит подключить прокси, нажмите «Подключить». Телефон должен быть в той же сети, что и компьютер.",
                            ))
                            .wrap(),
                        );
                        ui.add(egui::Label::new(muted(&link)).truncate());
                    });
                });
            }
            if !self.tg.foreign.is_empty() {
                theme::row(ui, 34.0, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Закрыть его").clicked() {
                            act = Some(Act::CloseForeign);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let names = self.tg.foreign.join(", ");
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("Запущен оригинальный TgWsProxy ({names}), он занимает тот же порт"))
                                        .color(YELLOW)
                                        .size(12.5),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            }
            if let Some(s) = &snap {
                ui.label(muted(format!(
                    "Сейчас подключений: {} · всего {}: WebSocket {}, Cloudflare {}, напрямую {}{} · ↑ {} ↓ {}",
                    s.active,
                    s.total,
                    s.via_ws,
                    s.via_cf,
                    s.via_tcp,
                    if s.via_warp > 0 || via_warp { format!(", через WARP {}", s.via_warp) } else { String::new() },
                    control::human(s.up),
                    control::human(s.down)
                )));
                let working = s.via_ws + s.via_cf + s.via_tcp + s.via_warp > 0;
                if s.failed > 0 && working && s.bad_secret == 0 {
                    ui.add(egui::Label::new(muted(format!("{}. Это не мешает: Telegram работает через остальные дата-центры", s.last_error))).wrap());
                } else if s.bad_secret > 0 || s.failed > 0 {
                    let col = if s.failed > 0 { RED } else { YELLOW };
                    ui.add(egui::Label::new(RichText::new(&s.last_error).color(col).size(12.5)).wrap());
                }
            } else if saved.enabled {
                ui.label(RichText::new("Прокси включён, но не запущен: проверьте журнал").color(RED).size(12.5));
            }
            if self.advanced {
                theme::row(ui, 38.0, |ui| {
                    theme::row_label(ui, "Порт", 50.0);
                    ui.add(egui::TextEdit::singleline(&mut self.tg.port).desired_width(64.0));
                    ui.add_space(10.0);
                    theme::row_label(ui, "Свой домен", 80.0);
                    let dirty = self.tg.port != saved.config.port.to_string() || self.tg.domain != saved.config.cf_domain;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(dirty, egui::Button::new("Сохранить")).clicked() {
                            act = Some(Act::Save);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.tg.domain)
                                    .hint_text("необязательно, для Cloudflare")
                                    .desired_width(ui.available_width()),
                            );
                        });
                    });
                });
                theme::row(ui, 32.0, |ui| {
                    let mut cf = saved.config.cloudflare;
                    if ui.checkbox(&mut cf, "Запасной путь через Cloudflare, если WebSocket недоступен").changed() {
                        act = Some(Act::Cloudflare(cf));
                    }
                });
            }
            theme::row(ui, 40.0, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(snap.is_some(), |ui| {
                        if ui.add(theme::Primary("Открыть в Telegram".into())).on_hover_text("Telegram предложит подключить этот прокси").clicked() {
                            act = Some(Act::Open);
                        }
                    });
                    if ui.button("Скопировать ссылку").clicked() {
                        act = Some(Act::Copy);
                    }
                    if self.advanced && ui.button("Новый secret").on_hover_text("После смены прокси нужно добавить в Telegram заново").clicked() {
                        act = Some(Act::NewSecret);
                    }
                });
            });
        });
        let Some(act) = act else { return };
        let link = saved.config.link();
        match act {
            Act::Toggle(on) => {
                match control::enable(on) {
                    Ok(m) => self.toast(ctx, Lvl::Ok, m),
                    Err(e) => self.toast(ctx, Lvl::Err, format!("Telegram-прокси не запустился: {e}")),
                }
                self.tg_reload();
            }
            Act::CloseForeign => {
                let (tx, ctx2) = (self.tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let n = control::close_foreign();
                    super::notify(&tx, if n > 0 { Lvl::Ok } else { Lvl::Err }, if n > 0 { "Оригинальный TgWsProxy закрыт" } else { "Не удалось закрыть TgWsProxy" });
                    let _ = tx.send(Msg::TgForeign(foreign_names()));
                    ctx2.request_repaint();
                });
            }
            Act::Open => {
                let client = self.tg.client.clone();
                std::thread::spawn(move || control::open_in_telegram(&link, client.as_ref()));
                self.toast(ctx, Lvl::Info, "Подтвердите подключение прокси в Telegram");
            }
            Act::Copy => {
                ctx.copy_text(link);
                self.toast(ctx, Lvl::Ok, "Ссылка на прокси скопирована");
            }
            Act::NewSecret => {
                let config = tgproxy::Config { secret: tgproxy::new_secret(), ..saved.config.clone() };
                self.tg_apply(ctx, config, "Secret сменён: добавьте прокси в Telegram заново кнопкой «Открыть в Telegram»");
            }
            Act::Warp(on) => {
                tgwarp::set(on);
                let ready = self.warp.as_ref().is_some_and(|w| w.mode == Mode::Proxy && (w.conn == Conn::Connected || tgwarp::proxy_up()));
                if on && !ready {
                    self.warp_proxy_on(ctx);
                } else if on {
                    self.toast(ctx, Lvl::Ok, "Telegram пойдёт через WARP, остальное как обычно через обход");
                } else if self.warp.as_ref().is_some_and(|w| w.conn == Conn::Connected) {
                    self.toast(ctx, Lvl::Info, "Telegram снова идёт как обычно. WARP остался подключён, отключить его можно в Настройках");
                } else {
                    self.toast(ctx, Lvl::Info, "Telegram снова идёт как обычно");
                }
            }
            Act::WarpProxy => self.warp_proxy_on(ctx),
            Act::Lan(on) => {
                let config = tgproxy::Config { lan: on, ..saved.config.clone() };
                let note = if !on {
                    "Telegram-прокси снова только для этого компьютера"
                } else if snap.is_some() {
                    "Прокси открыт для домашней сети: наведите камеру телефона на QR-код"
                } else {
                    "Прокси будет доступен в домашней сети, когда вы его включите"
                };
                self.tg_apply(ctx, config, note);
            }
            Act::Cloudflare(cf) => {
                let config = tgproxy::Config { cloudflare: cf, ..saved.config.clone() };
                self.tg_apply(ctx, config, if cf { "Запасной путь через Cloudflare включён" } else { "Запасной путь через Cloudflare выключен" });
            }
            Act::Save => {
                let Some(port) = self.tg.port.trim().parse::<u16>().ok().filter(|p| *p >= 1024) else {
                    self.toast(ctx, Lvl::Err, "Порт должен быть числом от 1024 до 65535");
                    return;
                };
                let domain = self.tg.domain.trim();
                let domain = if domain.is_empty() {
                    String::new()
                } else if let Some(d) = route::normalize_domain(domain) {
                    d
                } else {
                    self.toast(ctx, Lvl::Err, "Домен выглядит неправильно, нужен вид example.com");
                    return;
                };
                let config = tgproxy::Config { port, cf_domain: domain, ..saved.config.clone() };
                let note = if port != saved.config.port { "Порт сменён: добавьте прокси в Telegram заново" } else { "Настройки прокси сохранены" };
                self.tg_apply(ctx, config, note);
            }
        }
    }

    fn tg_apply(&mut self, ctx: &egui::Context, config: tgproxy::Config, ok: &str) {
        match control::apply(config) {
            Ok(()) => self.toast(ctx, Lvl::Ok, ok),
            Err(e) => self.toast(ctx, Lvl::Err, format!("Telegram-прокси не перезапустился: {e}")),
        }
        self.tg_reload();
    }
}
