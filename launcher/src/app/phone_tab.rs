use super::App;
use crate::core::lanproxy::diag::{self, Finding, Fix, Level};
use crate::core::lanproxy::{control, page};
use crate::core::tgproxy::control::human;
use crate::log::Lvl;
use crate::theme::{self, CYAN, GREEN, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2};

#[derive(Default)]
pub struct PhoneState {
    port: String,
    loaded: bool,
    qr_mode: usize,
    net: std::sync::Arc<std::sync::Mutex<Option<control::NetProfile>>>,
    net_at: Option<std::time::Instant>,
    diag: std::sync::Arc<std::sync::Mutex<Option<Vec<Finding>>>>,
    diag_running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    gw: std::sync::Arc<std::sync::Mutex<Option<crate::core::gateway::Iface>>>,
    gw_at: Option<std::time::Instant>,
    gw_on: Option<bool>,
    gw_live: Option<(bool, bool, Vec<crate::core::ray::AppActivity>)>,
    gw_live_at: Option<std::time::Instant>,
    gw_page: bool,
    folds: [bool; 4],
    help: [bool; 3],
}

fn fix_label(f: &Fix) -> &'static str {
    match f {
        Fix::MakePrivate(_) => "Сделать частной",
        Fix::RemoveBlocks(_) => "Удалить правила",
        Fix::AddRule => "Добавить правило",
    }
}

fn finding_row(ui: &mut egui::Ui, f: &Finding, enabled: bool) -> bool {
    let col = match f.level {
        Level::Ok => GREEN,
        Level::Info => Color32::from_gray(160),
        Level::Warn => YELLOW,
        Level::Bad => RED,
    };
    let mut clicked = false;
    theme::row(ui, 30.0, |ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, col);
        ui.label(RichText::new(&f.title).color(Color32::from_gray(228)).size(14.0));
        if let Some(fix) = &f.fix {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = ui.add_enabled(enabled, crate::theme::GlassButton::new(fix_label(fix))).clicked();
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.add(egui::Label::new(muted(&f.text)).wrap());
    });
    ui.add_space(6.0);
    clicked
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(175)).size(13.0)
}

pub(super) fn qr(ui: &mut egui::Ui, text: &str, side: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), Color32::WHITE);
    let Ok(code) = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium) else { return };
    let n = code.size();
    let quiet = 3;
    let cell = ((side / (n + quiet * 2) as f32).floor()).max(1.0);
    let used = cell * n as f32;
    let origin = rect.center() - Vec2::splat(used / 2.0);
    let ink = Color32::from_rgb(24, 16, 30);
    for y in 0..n {
        for x in 0..n {
            if code.get_module(x, y) {
                let min = origin + Vec2::new(x as f32 * cell, y as f32 * cell);
                p.rect_filled(egui::Rect::from_min_size(min, Vec2::splat(cell)), CornerRadius::ZERO, ink);
            }
        }
    }
}

fn step(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 10.0, theme::PINK.gamma_multiply(0.18));
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, n.to_string(), theme::semibold(11.5), Color32::from_rgb(245, 181, 218));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(text).color(Color32::from_gray(220)).size(13.5)).wrap());
        });
    });
    ui.add_space(2.0);
}

fn stat(ui: &mut egui::Ui, value: &str, caption: &str, divider: bool) {
    let r = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(value).font(theme::semibold(19.0)).strong());
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

fn mode_tile(ui: &mut egui::Ui, glyph: &str, on: bool, title: &str, sub: &str, state: (&str, Color32), tip: &str, enabled: bool) -> Option<bool> {
    let mut out = None;
    theme::card(ui, |ui| {
        let r = ui
            .horizontal(|ui| {
                theme::emblem(ui, on, glyph, 58.0);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 56.0).max(80.0));
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(title).font(theme::semibold(15.0)).strong().color(Color32::WHITE));
                    ui.add(egui::Label::new(muted(sub)).truncate());
                    theme::chip(ui, state.0, state.1);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut v = on;
                    if ui.add_enabled_ui(enabled, |ui| theme::toggle(ui, &mut v)).inner.changed() {
                        out = Some(v);
                    }
                });
            })
            .response;
        r.on_hover_text(tip);
    });
    out
}

enum Act {
    Toggle(bool),
    Copy(String, &'static str),
    Port,
    MakePrivate(u32),
    Diag(Option<Fix>),
    Protect,
}

impl PhoneState {
    pub(super) fn gateway_iface(&self) -> Option<crate::core::gateway::Iface> {
        self.gw.lock().unwrap().clone()
    }
}

fn kv(ui: &mut egui::Ui, first: bool, key: &str, value: &str, copy: bool) -> bool {
    if !first {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    let mut clicked = false;
    theme::row(ui, 32.0, |ui| {
        theme::row_label(ui, key, 160.0);
        let strong = copy;
        let text = if strong { RichText::new(value).font(theme::semibold(14.5)).strong() } else { RichText::new(value).color(Color32::from_gray(210)).size(14.0) };
        ui.label(text);
        if copy {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, theme::icon::COPY, "Скопировать").clicked() {
                    clicked = true;
                }
            });
        }
    });
    clicked
}

struct GwView {
    on: bool,
    ray_ok: bool,
    iface: Option<crate::core::gateway::Iface>,
    devices: Vec<crate::core::ray::AppActivity>,
    page_addr: Option<String>,
    problem: Option<(String, Option<&'static str>)>,
}

impl App {
    fn gateway_view(&mut self, ctx: &egui::Context) -> GwView {
        use crate::core::{gateway, ray, settings};
        let on = *self.phone.gw_on.get_or_insert_with(settings::ray_gateway);
        if self.phone.gw_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(15)) {
            self.phone.gw_at = Some(std::time::Instant::now());
            let gw = self.phone.gw.clone();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let i = gateway::iface();
                *gw.lock().unwrap() = i;
                ctx2.request_repaint();
            });
        }
        if on && self.phone.gw_live_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(2)) {
            self.phone.gw_live_at = Some(std::time::Instant::now());
            self.phone.gw_live = self.paths.as_ref().and_then(ray::read_live).map(|(l, age)| {
                let devices = ray::by_app(&l).into_iter().filter(|a| a.app.starts_with(ray::DEVICE)).collect();
                (l.running && age.as_secs() < 10, l.gateway, devices)
            });
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
        if on && !self.phone.gw_page {
            self.phone.gw_page = true;
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let _ = control::ensure_page();
                ctx2.request_repaint();
            });
        }
        let page_addr = if on { control::page_address() } else { None };
        let iface = self.phone.gw.lock().unwrap().clone();
        let (running, gw_active, devices) = self.phone.gw_live.clone().unwrap_or_default();
        let ray_ok = running && gw_active;
        let foreign = self.status.as_ref().and_then(|s| s.foreign_dir().map(|d| d.display().to_string()));
        let installed = self.status.as_ref().is_some_and(|s| s.service != crate::core::service::ServiceState::NotInstalled);
        let problem: Option<(String, Option<&'static str>)> = if !on || ray_ok {
            None
        } else if let Some(d) = &foreign {
            Some((format!("Служба zapret сейчас установлена из другой папки: {d}. Шлюз работает только со службой из этой папки, поэтому устройства выходят в интернет без обхода."), Some("Установить службу отсюда")))
        } else if !installed {
            Some(("Защита не включена: устройства выходят в интернет через компьютер, но без обхода.".into(), Some("Включить защиту")))
        } else if running {
            Some(("RayOfLight работает, но без шлюза: служба установлена до того, как режим включили. Переустановите её, и обход заработает для устройств.".into(), Some("Переустановить службу")))
        } else {
            Some(("RayOfLight сейчас не работает: устройства выходят в интернет через компьютер, но без обхода.".into(), None))
        };
        GwView { on, ray_ok, iface, devices, page_addr, problem }
    }

    fn gateway_toggle(&mut self, ctx: &egui::Context, v: bool) {
        use crate::core::{gateway, settings};
        settings::set_ray_gateway(v);
        self.phone.gw_on = Some(v);
        self.phone.gw_at = None;
        self.phone.gw_page = false;
        let installed = self.status.as_ref().is_some_and(|s| s.service != crate::core::service::ServiceState::NotInstalled);
        let strategy = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from)).and_then(|id| self.strategies.iter().find(|s| s.matches(&id)).cloned());
        self.spawn(ctx, move |p, tx| {
            match gateway::apply(v) {
                Ok(m) => super::notify(tx, Lvl::Ok, m),
                Err(e) => super::notify(tx, Lvl::Err, format!("Пересылка на сетевой карте не изменилась: {e}")),
            }
            if let (true, Some(s)) = (installed, strategy) {
                super::result_msg(tx, crate::core::service::refresh(p, &s), "Не удалось переустановить службу");
            }
        });
    }

    fn gateway_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, view: &GwView) {
        use crate::core::{gateway, ray};
        let mut fix = false;
        let mut copy = None;
        theme::card(ui, |ui| {
            ui.label(RichText::new("Компьютер как шлюз: что указать в настройках сети устройства").font(theme::semibold(15.0)).strong());
            if let Some((text, button)) = &view.problem {
                ui.add_space(4.0);
                if theme::soft_note(ui, YELLOW, text, button.map(|b| (b, !self.busy))) {
                    fix = true;
                }
            }
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                let qr_w = if view.page_addr.is_some() { 150.0 } else { 0.0 };
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - qr_w).max(260.0));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match &view.iface {
                        Some(i) => {
                            kv(ui, true, "IP-адрес", "тот, что устройство получило само", false);
                            if kv(ui, false, "Маска подсети", &gateway::mask(i.prefix), true) {
                                copy = Some(gateway::mask(i.prefix));
                            }
                            if kv(ui, false, "Шлюз («Роутер»)", &i.ip, true) {
                                copy = Some(i.ip.clone());
                            }
                            kv(ui, false, "DNS", "8.8.8.8 и 1.1.1.1", false);
                            kv(ui, false, "Прокси", "не использовать", false);
                            let state = if i.forwarding { "пересылка включена" } else { "пересылка включится вместе с RayOfLight" };
                            ui.add_space(6.0);
                            ui.label(muted(format!("Сетевая карта «{}», роутер {}, {state}", i.alias, i.router)).size(12.5));
                        }
                        None => {
                            ui.label(muted("Ищу сетевую карту с выходом в интернет…"));
                        }
                    }
                });
                if let Some(a) = &view.page_addr {
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        qr(ui, &format!("http://{a}/gateway"), 128.0);
                        ui.add_space(2.0);
                        ui.add(egui::Label::new(muted("инструкция на телефоне").size(12.0)).wrap())
                            .on_hover_text(format!("Наведите камеру телефона: откроется пошаговая настройка для PlayStation, Xbox, Switch и телевизоров с этими значениями. Или откройте в браузере: http://{a}/gateway"));
                    });
                }
            });
            ui.add_space(8.0);
            if view.devices.is_empty() {
                ui.label(muted("Устройств через шлюз пока не видно: они появятся здесь, когда откроют сайт из списков").size(12.5));
            } else {
                ui.label(muted(format!("Через шлюз · {}", view.devices.len())));
                for d in &view.devices {
                    let name = d.app.trim_start_matches(ray::DEVICE).to_string();
                    let hosts = d.hosts.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                    let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 32.0));
                    ui.painter().rect_filled(rect, CornerRadius::same(10), Color32::from_white_alpha(6));
                    theme::row(ui, 32.0, |ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(theme::icon::GAME).font(theme::icon_font(14.0)).color(CYAN));
                        ui.label(RichText::new(name).size(13.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(8.0);
                            ui.add(egui::Label::new(muted(format!("{} · {hosts}", d.handled)).size(12.5)).truncate());
                        });
                    });
                }
            }
            ui.add_space(4.0);
            theme::fold_row(ui, false, "Важно знать: компьютер должен быть включён, как вернуть настройки", &mut self.phone.folds[3]);
            if self.phone.folds[3] {
                ui.add(
                    egui::Label::new(muted(
                        "Компьютер должен быть включён: без него у устройства не будет интернета. Если роутер выдаст компьютеру другой адрес, шлюз на устройстве нужно поменять, поэтому адрес компьютера лучше закрепить в настройках роутера. Вернуть как было: в настройках сети устройства выберите «Автоматически». Не работает в гостевой сети Wi-Fi и если роутер изолирует устройства друг от друга.",
                    ))
                    .wrap(),
                );
                ui.add_space(4.0);
            }
        });
        if let Some(c) = copy {
            ctx.copy_text(c);
            self.toast(ctx, Lvl::Ok, "Скопировано");
        }
        if fix {
            match self.ray_current() {
                Some(s) => {
                    self.phone.gw_live_at = None;
                    self.spawn(ctx, move |p, tx| {
                        super::result_msg(tx, crate::core::service::install(p, &s), "Не удалось установить службу");
                    });
                }
                None => self.toast(ctx, Lvl::Err, "Нет файла bin\\RayOfLight.exe: скачайте HolyZapret заново"),
            }
        }
    }

    pub(super) fn phone_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.phone.loaded {
            self.phone.port = control::load().port.to_string();
            self.phone.help[0] = true;
            self.phone.loaded = true;
        }
        let snap = control::snapshot();
        if snap.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        let addr = control::address();
        if snap.is_some() && self.phone.net_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(30)) {
            self.phone.net_at = Some(std::time::Instant::now());
            let net = self.phone.net.clone();
            std::thread::spawn(move || {
                let p = control::net_profile();
                *net.lock().unwrap() = p;
            });
        }
        let public = self.phone.net.lock().unwrap().clone().filter(|p| p.public && snap.is_some());
        let fw_error = control::firewall_error().filter(|_| snap.is_some());
        let protected = self.status.as_ref().is_some_and(|s| s.engine_running);
        let gw = self.gateway_view(ctx);
        let mut act = None;
        let mut gw_toggle = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.columns(2, |c| {
                for col in c.iter_mut() {
                    col.spacing_mut().item_spacing.x = 10.0;
                }
                let share_state = match &snap {
                    Some(s) if s.clients > 0 => (format!("работает · устройств {}", s.clients), GREEN),
                    Some(_) => ("работает".to_string(), GREEN),
                    None => ("выключена".to_string(), Color32::from_gray(170)),
                };
                if let Some(v) = mode_tile(
                    &mut c[0],
                    theme::icon::PHONE,
                    snap.is_some(),
                    "Раздача на телефон",
                    "Телефоны и планшеты",
                    (&share_state.0, share_state.1),
                    "Телефоны и другие устройства в вашей домашней сети получают обход HolyZapret через этот компьютер: без root, VPN-серверов и приложений на компьютере. Доступ только из локальной сети.",
                    true,
                ) {
                    act = Some(Act::Toggle(v));
                }
                let gw_state = match (gw.on, gw.ray_ok) {
                    (true, true) if !gw.devices.is_empty() => (format!("работает · устройств {}", gw.devices.len()), GREEN),
                    (true, true) => ("работает".to_string(), GREEN),
                    (true, false) => ("без обхода".to_string(), YELLOW),
                    (false, _) => ("выключен".to_string(), Color32::from_gray(170)),
                };
                if let Some(v) = mode_tile(
                    &mut c[1],
                    theme::icon::TV,
                    gw.on,
                    "Компьютер как шлюз",
                    "Приставки и телевизоры",
                    (&gw_state.0, gw_state.1),
                    "Для приставок и телевизоров, где прокси не помогает: PlayStation, Xbox, Switch, телевизоры. В настройках сети устройства этот компьютер указывается как шлюз, и всё, что открывает устройство, идёт через обход.",
                    !self.busy,
                ) {
                    gw_toggle = Some(v);
                }
            });
            ui.spacing_mut().item_spacing.x = 10.0;
            if let (Some(s), Some(a)) = (&snap, &addr) {
                ui.add_space(2.0);
                theme::card(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            let v2 = page::v2ray_link(a);
                            let code = if self.phone.qr_mode == 1 { v2 } else { format!("http://{a}/") };
                            qr(ui, &code, 150.0);
                            ui.add_space(6.0);
                            theme::segmented(ui, &["Страница".to_string(), "v2rayNG".to_string()], &mut self.phone.qr_mode);
                        });
                        ui.add_space(16.0);
                        ui.vertical(|ui| {
                            ui.label(muted("Адрес раздачи"));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(a).font(theme::semibold(22.0)).strong().color(Color32::WHITE));
                                if theme::icon_button(ui, theme::icon::COPY, "Скопировать адрес").clicked() {
                                    act = Some(Act::Copy(a.clone(), "Адрес скопирован"));
                                }
                            });
                            let hint = if self.phone.qr_mode == 1 {
                                "В v2rayNG: плюс в правом верхнем углу → «Сканировать QR-код», наведите на этот код. Затем выберите сервер HolyZapret и нажмите запуск"
                            } else {
                                "Наведите камеру телефона: откроется пошаговая настройка для Android, iPhone, телевизора и PlayStation"
                            };
                            ui.add(egui::Label::new(muted(hint)).wrap());
                            if self.phone.qr_mode == 1 {
                                let v2 = page::v2ray_link(a);
                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new(muted(&v2).size(12.0)).truncate());
                                    if theme::icon_button(ui, theme::icon::COPY, "Скопировать ссылку").clicked() {
                                        act = Some(Act::Copy(v2.clone(), "Ссылка для v2rayNG скопирована"));
                                    }
                                });
                            }
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                stat(ui, &s.clients.to_string(), "устройств", true);
                                stat(ui, &s.active.to_string(), &format!("соединений · всего {}", s.total), true);
                                stat(ui, &format!("↓ {}", human(s.down)), &format!("↑ {}", human(s.up)), false);
                            });
                            if s.failed > 0 && !s.last_error.is_empty() {
                                ui.add(egui::Label::new(muted(format!("Последняя ошибка: {}", s.last_error)).size(12.5)).truncate());
                            }
                        });
                    });
                    if !protected {
                        ui.add_space(6.0);
                        if theme::soft_note(ui, YELLOW, "Защита сейчас выключена: телефон получит интернет через компьютер, но без обхода", Some(("Включить защиту", !self.busy))) {
                            act = Some(Act::Protect);
                        }
                    }
                    if let Some(p) = &public {
                        ui.add_space(6.0);
                        let name = if p.name.is_empty() { String::new() } else { format!(" «{}»", p.name) };
                        let text = format!("Windows считает сеть{name} общественной и не пускает к компьютеру другие устройства. Если это ваша домашняя сеть, сделайте её частной. В кафе и гостиницах так делать не стоит.");
                        if theme::soft_note(ui, RED, &text, Some(("Сделать сеть частной", true))) {
                            act = Some(Act::MakePrivate(p.index));
                        }
                    }
                    if let Some(e) = &fw_error {
                        ui.add_space(6.0);
                        theme::soft_note(ui, RED, &format!("Windows не дал открыть порт раздачи в брандмауэре, телефон может не подключиться. Ответ Windows: {e}"), None);
                    }
                    ui.add_space(6.0);
                    let r = ui
                        .scope(|ui| {
                            theme::soft_note(ui, YELLOW, "Пока на телефоне включён этот прокси, его Wi-Fi работает только с включённым компьютером и открытым HolyZapret", None);
                        })
                        .response;
                    r.on_hover_text("Чтобы вернуть интернет на телефоне: Настройки → Wi-Fi → ваша сеть → Изменить → Дополнительно → Прокси → Нет. С приложением v2rayNG такой проблемы нет: выключили его, и интернет идёт напрямую.");
                    ui.add_space(4.0);
                    let devices = control::devices();
                    if !devices.is_empty() {
                        theme::fold_row(ui, false, &format!("Что запрашивают устройства · {}", devices.len()), &mut self.phone.folds[0]);
                        if self.phone.folds[0] {
                            for (ip, d) in &devices {
                                ui.label(RichText::new(control::device_title(ip, d)).font(theme::semibold(14.0)).strong());
                                if let Some(a) = d.agents.first() {
                                    ui.add(egui::Label::new(muted(format!("Программа: {a}")).size(12.5)).truncate());
                                }
                                for e in d.errors.iter().take(2) {
                                    ui.add(egui::Label::new(RichText::new(format!("Ошибка запроса: {e}")).color(RED).size(12.5)).wrap());
                                }
                                for v in d.visits.iter().take(12) {
                                    let text = control::visit_line(v);
                                    let label = if v.error.is_some() { RichText::new(text).color(YELLOW).size(12.5) } else { muted(text).size(12.5) };
                                    ui.add(egui::Label::new(label).truncate());
                                }
                                if d.visits.len() > 12 {
                                    ui.label(muted(format!("и ещё {}, все — в отчёте для разработчика", d.visits.len() - 12)).size(12.5));
                                }
                                ui.add_space(6.0);
                            }
                        }
                    }
                    let running = self.phone.diag_running.load(std::sync::atomic::Ordering::Relaxed);
                    let found = self.phone.diag.lock().unwrap().clone();
                    theme::fold_row(ui, false, "Телефон не подключается?", &mut self.phone.folds[1]);
                    if self.phone.folds[1] {
                        theme::row(ui, 34.0, |ui| {
                            ui.add(egui::Label::new(muted("Проверю, отвечает ли раздача, тип сети, брандмауэр Windows, антивирус и VPN. Несколько секунд")).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if running {
                                    ui.spinner();
                                } else if ui.add_enabled(!self.busy, crate::theme::GlassButton::new(if found.is_some() { "Проверить ещё раз" } else { "Проверить" })).clicked() {
                                    act = Some(Act::Diag(None));
                                }
                            });
                        });
                        if let Some(list) = &found {
                            ui.add_space(4.0);
                            for item in list {
                                if finding_row(ui, item, !running && !self.busy) {
                                    act = Some(Act::Diag(item.fix.clone()));
                                }
                            }
                        }
                    }
                    theme::fold_row(ui, false, "Дополнительно: порт", &mut self.phone.folds[2]);
                    if self.phone.folds[2] {
                        theme::row(ui, 36.0, |ui| {
                            theme::row_label(ui, "Порт", 50.0);
                            ui.add(egui::TextEdit::singleline(&mut self.phone.port).desired_width(72.0));
                            if crate::theme::button(ui, "Сохранить").clicked() {
                                act = Some(Act::Port);
                            }
                        });
                    }
                });
            }
            if gw.on {
                ui.add_space(2.0);
                self.gateway_card(ui, ctx, &gw);
            }
            ui.add_space(2.0);
            let a = addr.clone().unwrap_or_else(|| "адрес_компьютера:порт".into());
            theme::card(ui, |ui| {
                ui.label(RichText::new("Как подключить").font(theme::semibold(15.0)).strong());
                ui.add(egui::Label::new(muted("Устройство должно быть в той же сети, что и компьютер")).wrap());
                ui.add_space(2.0);
                theme::fold_row(ui, true, "Телефон через приложение v2rayNG — рекомендуем", &mut self.phone.help[0]);
                if self.phone.help[0] {
                    step(ui, 1, "Включите раздачу и установите v2rayNG из Google Play или с GitHub");
                    step(ui, 2, "Переключите QR-код на «v2rayNG». В приложении: плюс → «Сканировать QR-код». Или плюс → «Вручную: SOCKS» с адресом и портом раздачи без пароля");
                    step(ui, 3, "Выберите сервер HolyZapret, нажмите запуск внизу и разрешите VPN");
                    ui.add(egui::Label::new(muted("Через компьютер пойдёт всё, включая звонки Discord. Выключили v2rayNG — интернет на телефоне снова напрямую.").size(12.5)).wrap());
                    ui.add_space(6.0);
                }
                theme::fold_row(ui, false, "Телефон через прокси в настройках Wi-Fi, без приложений", &mut self.phone.help[1]);
                if self.phone.help[1] {
                    step(ui, 1, "Настройки → Wi-Fi → ваша сеть → Изменить → Дополнительно → Прокси → Автоматическая настройка (PAC)");
                    step(ui, 2, &format!("Адрес PAC: http://{a}/proxy.pac. С ним при выключенной раздаче телефон идёт в интернет напрямую, пока лаунчер открыт"));
                    step(ui, 3, &format!("Или Прокси → Вручную: имя хоста и порт из адреса {a}"));
                    ui.add(egui::Label::new(muted("Так работают браузер и большинство приложений, но не звонки.").size(12.5)).wrap());
                    ui.add_space(6.0);
                }
                theme::fold_row(ui, false, "PlayStation, Xbox, Switch, телевизор, iPhone", &mut self.phone.help[2]);
                if self.phone.help[2] {
                    step(ui, 1, "Приставки и телевизоры: включите «Компьютер как шлюз» и впишите значения из его карточки в настройки сети устройства. Пошаговая инструкция — по QR-коду в той же карточке");
                    step(ui, 2, "iPhone и Android TV: включите раздачу и откройте страницу из QR-кода, там пошагово для каждого устройства");
                    ui.add_space(6.0);
                }
            });
        });
        if let Some(v) = gw_toggle {
            self.gateway_toggle(ctx, v);
        }
        match act {
            Some(Act::Toggle(false)) => self.confirm = Some(super::confirm::Confirm::StopSharing),
            Some(Act::Toggle(true)) => match control::enable(true) {
                Ok(m) => self.toast(ctx, Lvl::Ok, m),
                Err(e) => self.toast(ctx, Lvl::Err, format!("Раздача не запустилась: {e}")),
            },
            Some(Act::Copy(a, msg)) => {
                ctx.copy_text(a);
                self.toast(ctx, Lvl::Ok, msg);
            }
            Some(Act::Port) => match self.phone.port.trim().parse::<u16>().ok().filter(|p| *p >= 1024) {
                Some(p) => match control::set_port(p) {
                    Ok(()) => self.toast(ctx, Lvl::Ok, format!("Порт раздачи: {p}")),
                    Err(e) => self.toast(ctx, Lvl::Err, format!("Не удалось сменить порт: {e}")),
                },
                None => self.toast(ctx, Lvl::Err, "Порт должен быть числом от 1024 до 65535"),
            },
            Some(Act::Diag(fix)) => {
                let flag = self.phone.diag_running.clone();
                if !flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let out = self.phone.diag.clone();
                    let (tx, ctx2) = (self.tx.clone(), ctx.clone());
                    if fix.is_some() {
                        self.phone.net_at = None;
                    }
                    std::thread::spawn(move || {
                        if let Some(f) = fix {
                            match diag::apply(&f) {
                                Ok(m) => super::notify(&tx, Lvl::Ok, m),
                                Err(e) => super::notify(&tx, Lvl::Err, format!("Не получилось: {e}")),
                            }
                        }
                        let r = diag::run();
                        *out.lock().unwrap() = Some(r);
                        flag.store(false, std::sync::atomic::Ordering::SeqCst);
                        ctx2.request_repaint();
                    });
                }
            }
            Some(Act::MakePrivate(index)) => {
                let net = self.phone.net.clone();
                self.spawn(ctx, move |_, tx| match control::make_private(index) {
                    Ok(()) => {
                        if let Some(p) = net.lock().unwrap().as_mut() {
                            p.public = false;
                        }
                        super::notify(tx, Lvl::Ok, "Сеть стала частной, телефон теперь может подключиться к компьютеру");
                    }
                    Err(e) => super::notify(tx, Lvl::Err, format!("Не удалось сделать сеть частной: {e}")),
                });
            }
            Some(Act::Protect) => self.install_general(ctx),
            None => {}
        }
    }
}
