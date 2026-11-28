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
    more: bool,
    faq: [bool; 3],
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
    RichText::new(text.into()).color(Color32::from_gray(175)).size(13.0)
}

fn stat(ui: &mut egui::Ui, value: &str, caption: &str, size: f32, divider: bool) {
    let r = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(20.0 - size);
            ui.label(RichText::new(value).font(theme::semibold(size)).strong());
            ui.label(muted(caption).size(12.5));
        })
        .response
        .rect;
    if divider {
        ui.add_space(14.0);
        let x = ui.cursor().left();
        ui.painter().vline(x, r.top() + 4.0..=r.bottom() - 4.0, egui::Stroke::new(1.0, Color32::from_white_alpha(14)));
        ui.add_space(14.0);
    }
}

fn step(ui: &mut egui::Ui, n: &str, title: &str, text: &str) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 11.0, theme::PINK.gamma_multiply(0.18));
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, n, theme::semibold(12.0), Color32::from_rgb(245, 181, 218));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(2.0);
            ui.label(RichText::new(title).size(14.0).color(Color32::from_gray(232)));
            ui.add(egui::Label::new(muted(text).size(12.5)).wrap());
        });
    });
}

impl App {
    pub(super) fn tg_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.tg_card(ui, ctx);
            ui.add_space(2.0);
            let client = self.tg.client.as_ref().map_or_else(|| "Telegram".to_string(), |c| c.name.clone());
            theme::card(ui, |ui| {
                theme::row(ui, 32.0, |ui| {
                    ui.label(RichText::new("Как подключить").font(theme::semibold(15.0)).strong());
                    theme::chip(ui, &format!("найден {client}"), Color32::from_gray(170));
                });
                ui.add_space(2.0);
                step(ui, "1", "Включите прокси и нажмите «Открыть в Telegram»", "В Telegram появится окно с прокси 127.0.0.1, нажмите «Подключить»");
                ui.add_space(4.0);
                step(ui, "2", "Оставьте лаунчер работать", "Прокси живёт внутри лаунчера: его можно свернуть в трей, автозапуск — в «Настройках»");
                ui.add_space(6.0);
                theme::fold_row(ui, false, "Окно в Telegram не появилось", &mut self.tg.faq[0]);
                if self.tg.faq[0] {
                    ui.add(egui::Label::new(muted("Скопируйте ссылку, отправьте её себе в «Избранное» и нажмите по ней.")).wrap());
                    ui.add_space(4.0);
                }
                theme::fold_row(ui, false, "Не грузятся фото или видео", &mut self.tg.faq[1]);
                if self.tg.faq[1] {
                    ui.add(
                        egui::Label::new(muted(
                            "Проверьте, что включён запасной путь через Cloudflare. Если не помогло, укажите свой домен по инструкции tg-ws-proxy (раздел CfProxy) в «Дополнительно».",
                        ))
                        .wrap(),
                    );
                    ui.add_space(4.0);
                }
                theme::fold_row(ui, false, "Telegram пишет «Подключение» и не подключается", &mut self.tg.faq[2]);
                if self.tg.faq[2] {
                    ui.add(
                        egui::Label::new(muted(
                            "Нажмите «Открыть в Telegram» ещё раз: после смены secret или порта старый прокси перестаёт работать. В настройках типа подключения Telegram отключите IPv6: прокси его не поддерживает. Если в вашем регионе не помогает, включите «Telegram через WARP».",
                        ))
                        .wrap(),
                    );
                    ui.add_space(4.0);
                }
            });
        });
    }

    pub(super) fn tg_reload(&mut self) {
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
        let via_warp = tgwarp::enabled();
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                theme::emblem(ui, snap.is_some(), theme::icon::TELEGRAM, 76.0);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 60.0).max(200.0));
                    ui.add_space(6.0);
                    let title = match (&snap, saved.enabled) {
                        (Some(_), _) => "Telegram-прокси работает",
                        (None, true) => "Telegram-прокси не запущен",
                        (None, false) => "Telegram-прокси выключен",
                    };
                    ui.label(RichText::new(title).font(theme::semibold(20.0)).strong().color(Color32::WHITE));
                    let sub = if snap.is_some() {
                        "Telegram Desktop идёт через WebSocket к web.telegram.org"
                    } else {
                        "Ускоряет Telegram Desktop без сторонних серверов"
                    };
                    ui.add(egui::Label::new(muted(sub)).truncate().show_tooltip_when_elided(false))
                        .on_hover_text("Трафик Telegram Desktop идёт через WebSocket к web.telegram.org, без сторонних серверов. Работает, пока открыт лаунчер, в том числе в трее. Основан на tg-ws-proxy от Flowseal.");
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        match (&snap, saved.enabled) {
                            (Some(_), _) => theme::chip(ui, "работает", GREEN),
                            (None, true) => theme::chip(ui, "не запущен", RED),
                            (None, false) => theme::chip(ui, "выключен", Color32::from_gray(170)),
                        };
                        if via_warp {
                            theme::chip(ui, "через WARP", CYAN);
                        }
                        if saved.config.lan {
                            theme::chip(ui, "для телефонов", CYAN);
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut on = snap.is_some();
                    if theme::toggle(ui, &mut on).changed() {
                        act = Some(Act::Toggle(on));
                    }
                });
            });
            if let Some(s) = &snap {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    stat(ui, &s.active.to_string(), "подключений сейчас", 20.0, true);
                    stat(ui, &s.total.to_string(), "всего за сеанс", 20.0, true);
                    stat(ui, &format!("↓ {}", control::human(s.down)), &format!("↑ {}", control::human(s.up)), 20.0, true);
                    let mut path = format!("WebSocket {} · CF {}", s.via_ws, s.via_cf);
                    if s.via_tcp > 0 {
                        path.push_str(&format!(" · напрямую {}", s.via_tcp));
                    }
                    if s.via_warp > 0 || via_warp {
                        path.push_str(&format!(" · WARP {}", s.via_warp));
                    }
                    stat(ui, &path, "каким путём", 14.5, false);
                });
                let working = s.via_ws + s.via_cf + s.via_tcp + s.via_warp > 0;
                if s.failed > 0 && working && s.bad_secret == 0 {
                    ui.add(egui::Label::new(muted(format!("{}. Это не мешает: Telegram работает через остальные дата-центры", s.last_error)).size(12.5)).wrap());
                } else if s.bad_secret > 0 || s.failed > 0 {
                    ui.add_space(4.0);
                    theme::soft_note(ui, if s.failed > 0 { RED } else { YELLOW }, &s.last_error, None);
                }
            } else if saved.enabled {
                ui.add_space(6.0);
                theme::soft_note(ui, RED, "Прокси включён, но не запустился: проверьте «Настройки → Журнал»", None);
            }
            if !self.tg.foreign.is_empty() {
                ui.add_space(6.0);
                let names = self.tg.foreign.join(", ");
                if theme::soft_note(ui, YELLOW, &format!("Запущен оригинальный TgWsProxy ({names}), он занимает тот же порт"), Some(("Закрыть его", true))) {
                    act = Some(Act::CloseForeign);
                }
            }
            ui.add_space(8.0);
            let y = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, Color32::from_white_alpha(12)));
            ui.add_space(6.0);
            theme::row(ui, 40.0, |ui| {
                let note = format!("Прокси 127.0.0.1:{} · работает, пока открыт лаунчер", saved.config.port);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(snap.is_some(), |ui| {
                        if ui.add(theme::Primary("Открыть в Telegram".into())).on_hover_text("Telegram предложит подключить этот прокси").clicked() {
                            act = Some(Act::Open);
                        }
                    });
                    if crate::theme::button(ui, "Скопировать ссылку").clicked() {
                        act = Some(Act::Copy);
                    }
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(muted(&note).size(12.5)).truncate());
                    });
                });
            });
        });
        if saved.config.lan && snap.is_some() {
            if let Some(link) = control::phone_link() {
                ui.add_space(2.0);
                theme::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        super::phone_tab::qr(ui, &link, 120.0);
                        ui.add_space(16.0);
                        ui.vertical(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Подключить телефон").font(theme::semibold(15.0)).strong());
                            ui.add(
                                egui::Label::new(muted(
                                    "Наведите камеру телефона: Telegram предложит подключить прокси, нажмите «Подключить». Телефон должен быть в той же сети, что и компьютер.",
                                ))
                                .wrap(),
                            );
                            ui.add(egui::Label::new(muted(&link).size(12.0).color(Color32::from_gray(140))).truncate());
                        });
                    });
                });
            }
        }
        ui.add_space(2.0);
        let warp = self.warp.clone().filter(|w| w.installed);
        theme::card(ui, |ui| {
            ui.label(RichText::new("Настройки").font(theme::semibold(15.0)).strong());
            ui.add_space(2.0);
            let mut on = via_warp;
            if theme::setting_row(
                ui,
                true,
                "Telegram через WARP",
                "Если Telegram не подключается даже через прокси",
                "Трафик Telegram пойдёт через Cloudflare WARP, а всё остальное как обычно через обход. Касается и Telegram на телефоне через раздачу.",
                &mut on,
                warp.is_some() || via_warp,
            ) {
                act = Some(Act::Warp(on));
            }
            match &warp {
                None => {
                    theme::soft_note(ui, YELLOW, "Нужен Cloudflare WARP: скачайте его в «Настройках»", None);
                    ui.add_space(4.0);
                }
                Some(w) if via_warp && (w.mode != Mode::Proxy || (w.conn != Conn::Connected && !tgwarp::proxy_up())) => {
                    if theme::soft_note(ui, YELLOW, "WARP должен быть подключён в режиме прокси", Some(("Подключить WARP", !self.busy))) {
                        act = Some(Act::WarpProxy);
                    }
                    ui.add_space(4.0);
                }
                _ => {}
            }
            let mut lan = saved.config.lan;
            if theme::setting_row(
                ui,
                false,
                "Для телефонов в домашней сети",
                "Telegram на телефоне подключится без VPN",
                "Telegram на телефоне подключается к этому прокси сам, без VPN и приложений: нужен только прокси в настройках Telegram. Путь тот же, что у Telegram на компьютере, в том числе через WARP. Подключиться могут только устройства из вашей сети.",
                &mut lan,
                true,
            ) {
                act = Some(Act::Lan(lan));
            }
            let mut cf = saved.config.cloudflare;
            if theme::setting_row(
                ui,
                false,
                "Запасной путь через Cloudflare",
                "Если WebSocket недоступен: помогает с фото и видео",
                "",
                &mut cf,
                true,
            ) {
                act = Some(Act::Cloudflare(cf));
            }
            if self.advanced {
                theme::fold_row(ui, false, "Дополнительно: порт, свой домен, новый secret", &mut self.tg.more);
                if self.tg.more {
                    theme::row(ui, 38.0, |ui| {
                        theme::row_label(ui, "Порт", 50.0);
                        ui.add(egui::TextEdit::singleline(&mut self.tg.port).desired_width(72.0));
                        ui.add_space(10.0);
                        theme::row_label(ui, "Свой домен", 80.0);
                        let dirty = self.tg.port != saved.config.port.to_string() || self.tg.domain != saved.config.cf_domain;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(dirty, crate::theme::GlassButton::new("Сохранить")).clicked() {
                                act = Some(Act::Save);
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.tg.domain).hint_text("необязательно, для Cloudflare").desired_width(ui.available_width()));
                            });
                        });
                    });
                    theme::row(ui, 38.0, |ui| {
                        ui.label(muted("После смены secret прокси нужно добавить в Telegram заново").size(12.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if crate::theme::button(ui, "Новый secret").clicked() {
                                act = Some(Act::NewSecret);
                            }
                        });
                    });
                }
            }
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
                    self.toast(ctx, Lvl::Info, "Telegram снова идёт как обычно. WARP остался подключён, отключить его можно в «Настройках»");
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
