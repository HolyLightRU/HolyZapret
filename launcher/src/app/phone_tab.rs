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
        ui.label(RichText::new("●").color(col).size(13.0));
        ui.label(RichText::new(&f.title).color(Color32::from_gray(225)));
        if let Some(fix) = &f.fix {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = ui.add_enabled(enabled, egui::Button::new(fix_label(fix))).clicked();
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.add(egui::Label::new(muted(&f.text)).wrap());
    });
    ui.add_space(6.0);
    clicked
}

fn alert(ui: &mut egui::Ui, color: Color32, text: &str, button: Option<&str>) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(color).size(13.0)).wrap());
            if let Some(b) = button {
                ui.add_space(4.0);
                clicked = ui.button(b).clicked();
            }
        });
    clicked
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
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
        ui.label(RichText::new(format!("{n}.")).color(CYAN).strong());
        ui.add(egui::Label::new(RichText::new(text).color(Color32::from_gray(205))).wrap());
    });
}

enum Act {
    Toggle(bool),
    Copy(String, &'static str),
    Port,
    MakePrivate(u32),
    Diag(Option<Fix>),
}

impl PhoneState {
    pub(super) fn gateway_iface(&self) -> Option<crate::core::gateway::Iface> {
        self.gw.lock().unwrap().clone()
    }
}

fn kv(ui: &mut egui::Ui, key: &str, value: &str) -> bool {
    let mut copy = false;
    theme::row(ui, 28.0, |ui| {
        theme::row_label(ui, key, 170.0);
        ui.label(RichText::new(value).size(14.0).strong());
        if !value.contains(' ') && theme::icon_button(ui, theme::icon::COPY, "Скопировать").clicked() {
            copy = true;
        }
    });
    copy
}

impl App {
    fn gateway_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
        let problem: Option<(String, Option<&str>)> = if !on || ray_ok {
            None
        } else if let Some(d) = &foreign {
            Some((format!("Служба zapret сейчас установлена из другой папки: {d}. Шлюз работает только со службой из этой папки, поэтому устройства выходят в интернет без обхода."), Some("Установить general отсюда")))
        } else if !installed {
            Some(("Служба не установлена: устройства выходят в интернет через компьютер, но без обхода. Шлюз работает вместе с RayOfLight и стратегией general.".into(), Some("Установить general")))
        } else if running {
            Some(("RayOfLight работает, но без шлюза: служба установлена до того, как режим включили. Переустановите её, и обход заработает для устройств.".into(), Some("Переустановить службу")))
        } else {
            Some(("Обход для устройств работает вместе с RayOfLight и стратегией general. Сейчас RayOfLight не работает: устройства выходят в интернет через компьютер, но без обхода.".into(), None))
        };
        let mut fix = false;
        let mut toggle = None;
        let mut copy = None;
        theme::card(ui, |ui| {
            theme::row(ui, 32.0, |ui| {
                ui.label(RichText::new(theme::icon::GLOBE).font(theme::icon_font(17.0)).color(CYAN));
                ui.add_space(2.0);
                ui.label(RichText::new("Компьютер как шлюз").size(15.5).strong());
                match (on, ray_ok) {
                    (true, true) => theme::chip(ui, "Работает", GREEN),
                    (true, false) => theme::chip(ui, "Без обхода", YELLOW),
                    (false, _) => theme::chip(ui, "Выключен", Color32::from_gray(160)),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut v = on;
                    if ui.add_enabled_ui(!self.busy, |ui| theme::toggle(ui, &mut v)).inner.changed() {
                        toggle = Some(v);
                    }
                });
            });
            ui.add(
                egui::Label::new(muted(
                    "Для приставок и телевизоров, где прокси не помогает: PlayStation, Xbox, Switch, телевизоры. В настройках сети устройства этот компьютер указывается как шлюз, и всё, что открывает устройство, идёт через обход. Самому устройству прокси не нужен.",
                ))
                .wrap(),
            );
            if !on {
                return;
            }
            if let Some((text, button)) = &problem {
                ui.add_space(6.0);
                if alert(ui, YELLOW, text, *button) && !self.busy {
                    fix = true;
                }
            }
            ui.add_space(6.0);
            match &iface {
                Some(i) => {
                    ui.label(RichText::new("Что указать в настройках сети устройства (вручную)").strong());
                    kv(ui, "IP-адрес", "тот, что устройство получило само");
                    if kv(ui, "Маска подсети", &gateway::mask(i.prefix)) {
                        copy = Some(gateway::mask(i.prefix));
                    }
                    if kv(ui, "Шлюз («Роутер»)", &i.ip) {
                        copy = Some(i.ip.clone());
                    }
                    kv(ui, "DNS", "8.8.8.8 и 1.1.1.1");
                    kv(ui, "Прокси", "не использовать");
                    let state = if i.forwarding { "пересылка включена" } else { "пересылка выключена, включится вместе с RayOfLight" };
                    ui.label(muted(format!("Сетевая карта «{}», роутер {}, {state}", i.alias, i.router)));
                }
                None => {
                    ui.label(muted("Ищу сетевую карту с выходом в интернет…"));
                }
            }
            if let Some(a) = &page_addr {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    qr(ui, &format!("http://{a}/gateway"), 132.0);
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Инструкция на телефоне").strong());
                        ui.add(
                            egui::Label::new(muted(
                                "Наведите камеру телефона на код: откроется пошаговая настройка для PlayStation, Xbox, Switch и телевизоров с этими значениями и кнопками «Копировать».",
                            ))
                            .wrap(),
                        );
                        ui.label(muted(format!("Или откройте в браузере: http://{a}/gateway")));
                    });
                });
            }
            ui.add_space(6.0);
            if devices.is_empty() {
                ui.label(muted("Устройств через шлюз пока не видно. Они появятся здесь, когда откроют сайт из списков."));
            } else {
                ui.label(RichText::new(format!("Через шлюз: {}", devices.len())).strong());
                for d in &devices {
                    let name = d.app.trim_start_matches(ray::DEVICE);
                    let hosts = d.hosts.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
                    let text = format!("{name} · обработано {} · {hosts}", d.handled);
                    ui.add(egui::Label::new(muted(text)).truncate());
                }
            }
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(muted(
                    "Компьютер должен быть включён: без него у устройства не будет интернета. Если роутер выдаст компьютеру другой адрес, шлюз на устройстве нужно поменять, поэтому адрес компьютера лучше закрепить в настройках роутера. Вернуть как было: в настройках сети устройства выберите «Автоматически». Не работает в гостевой сети Wi-Fi и если роутер изолирует устройства друг от друга.",
                ))
                .wrap(),
            );
        });
        if let Some(c) = copy {
            ctx.copy_text(c);
            self.toast(ctx, Lvl::Ok, "Скопировано");
        }
        if fix {
            let own = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from)).and_then(|id| self.strategies.iter().find(|s| s.matches(&id)).cloned());
            let target = own.or_else(|| self.strategies.iter().find(|s| s.matches("general")).cloned());
            match target {
                Some(s) => {
                    self.phone.gw_live_at = None;
                    self.spawn(ctx, move |p, tx| {
                        super::result_msg(tx, crate::core::service::install(p, &s), "Не удалось установить службу");
                    });
                }
                None => self.toast(ctx, Lvl::Err, "Стратегия general не найдена в папке scripts"),
            }
        }
        if let Some(v) = toggle {
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
                    super::result_msg(tx, crate::core::service::install(p, &s), "Не удалось переустановить службу");
                }
            });
        }
    }

    pub(super) fn phone_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.phone.loaded {
            self.phone.port = control::load().port.to_string();
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
        let mut act = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::card(ui, |ui| {
                theme::row(ui, 32.0, |ui| {
                    ui.label(RichText::new(theme::icon::PHONE).font(theme::icon_font(17.0)).color(CYAN));
                    ui.add_space(2.0);
                    ui.label(RichText::new("Раздача на телефон").size(15.5).strong());
                    match &snap {
                        Some(_) => theme::chip(ui, "Работает", GREEN),
                        None => theme::chip(ui, "Выключена", Color32::from_gray(160)),
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
                        "Телефоны и другие устройства в вашей домашней сети получают обход HolyZapret через этот компьютер: без root, VPN-серверов и приложений на компьютере. Доступ только из локальной сети.",
                    ))
                    .wrap(),
                );
                if snap.is_some() && !protected {
                    ui.add_space(2.0);
                    ui.label(RichText::new("Защита сейчас выключена: телефон получит интернет через компьютер, но без обхода").color(YELLOW).size(12.5));
                }
                if let Some(p) = &public {
                    ui.add_space(6.0);
                    let name = if p.name.is_empty() { String::new() } else { format!(" «{}»", p.name) };
                    let text = format!(
                        "Windows считает сеть{name} общественной и не пускает к компьютеру другие устройства, поэтому телефон может не подключиться. Если это ваша домашняя сеть, сделайте её частной. В кафе и гостиницах так делать не стоит."
                    );
                    if alert(ui, RED, &text, Some("Сделать сеть частной")) {
                        act = Some(Act::MakePrivate(p.index));
                    }
                }
                if let Some(e) = &fw_error {
                    ui.add_space(6.0);
                    alert(ui, RED, &format!("Windows не дал открыть порт раздачи в брандмауэре, телефон может не подключиться. Ответ Windows: {e}"), None);
                }
                if snap.is_some() {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(YELLOW.gamma_multiply(0.12))
                        .stroke(egui::Stroke::new(1.0, YELLOW.gamma_multiply(0.45)))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new("Пока на телефоне включён этот прокси, его Wi-Fi работает только с включённым компьютером и открытым HolyZapret. Android не подскажет, что дело в прокси.")
                                        .color(YELLOW)
                                        .size(13.0),
                                )
                                .wrap(),
                            );
                            ui.add(egui::Label::new(muted("Чтобы вернуть интернет на телефоне: Настройки → Wi-Fi → ваша сеть → Изменить → Дополнительно → Прокси → Нет. С приложением v2rayNG такой проблемы нет: выключили его, и интернет идёт напрямую.")).wrap());
                        });
                }
                if let (Some(s), Some(a)) = (&snap, &addr) {
                    ui.add_space(8.0);
                    let labels = ["Страница настройки".to_string(), "Ссылка для v2rayNG".to_string()];
                    theme::segmented(ui, &labels, &mut self.phone.qr_mode);
                    ui.add_space(8.0);
                    let v2 = page::v2ray_link(a);
                    let code = if self.phone.qr_mode == 1 { v2.clone() } else { format!("http://{a}/") };
                    ui.horizontal(|ui| {
                        qr(ui, &code, 176.0);
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            ui.label(muted("Адрес прокси"));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(a).size(22.0).strong());
                                if theme::icon_button(ui, theme::icon::COPY, "Скопировать адрес").clicked() {
                                    act = Some(Act::Copy(a.clone(), "Адрес скопирован"));
                                }
                            });
                            ui.add_space(4.0);
                            let hint = if self.phone.qr_mode == 1 {
                                "В v2rayNG на телефоне: плюс в правом верхнем углу → «Сканировать QR-код», наведите на этот код. Затем выберите сервер HolyZapret и нажмите запуск. Через компьютер пойдёт всё, включая звонки Discord."
                            } else {
                                "Наведите камеру телефона на QR-код: откроется страница с настройкой для Android, iPhone, телевизора и PlayStation"
                            };
                            ui.add(egui::Label::new(RichText::new(hint).color(Color32::from_gray(205))).wrap());
                            if self.phone.qr_mode == 1 {
                                ui.horizontal(|ui| {
                                    ui.label(muted(&v2));
                                    if theme::icon_button(ui, theme::icon::COPY, "Скопировать ссылку").clicked() {
                                        act = Some(Act::Copy(v2.clone(), "Ссылка для v2rayNG скопирована"));
                                    }
                                });
                            }
                            ui.add_space(6.0);
                            ui.label(muted(format!(
                                "Устройств: {} · соединений сейчас {} · всего {} · ↑ {} ↓ {}",
                                s.clients,
                                s.active,
                                s.total,
                                human(s.up),
                                human(s.down)
                            )));
                            if s.failed > 0 && !s.last_error.is_empty() {
                                ui.add(egui::Label::new(muted(format!("Последняя ошибка: {}", s.last_error))).truncate());
                            }
                        });
                    });
                    let devices = control::devices();
                    if !devices.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(RichText::new(format!("Что запрашивают устройства: {}", devices.len())).size(14.0))
                            .id_salt("lan_devices")
                            .show(ui, |ui| {
                                for (ip, d) in &devices {
                                    ui.label(RichText::new(control::device_title(ip, d)).size(14.0).strong());
                                    if let Some(a) = d.agents.first() {
                                        ui.add(egui::Label::new(muted(format!("Программа: {a}"))).truncate());
                                    }
                                    for e in d.errors.iter().take(2) {
                                        ui.add(egui::Label::new(RichText::new(format!("Ошибка запроса: {e}")).color(RED).size(12.5)).wrap());
                                    }
                                    for v in d.visits.iter().take(12) {
                                        let text = control::visit_line(v);
                                        let label = if v.error.is_some() { RichText::new(text).color(YELLOW).size(12.5) } else { muted(text) };
                                        ui.add(egui::Label::new(label).truncate());
                                    }
                                    if d.visits.len() > 12 {
                                        ui.label(muted(format!("и ещё {}, все — в отчёте для разработчика", d.visits.len() - 12)));
                                    }
                                    ui.add_space(6.0);
                                }
                            });
                    }
                }
                ui.add_space(6.0);
                theme::row(ui, 36.0, |ui| {
                    theme::row_label(ui, "Порт", 50.0);
                    ui.add(egui::TextEdit::singleline(&mut self.phone.port).desired_width(64.0));
                    if ui.button("Сохранить").clicked() {
                        act = Some(Act::Port);
                    }
                });
            });
            if snap.is_some() {
                ui.add_space(10.0);
                let running = self.phone.diag_running.load(std::sync::atomic::Ordering::Relaxed);
                let found = self.phone.diag.lock().unwrap().clone();
                theme::card(ui, |ui| {
                    theme::row(ui, 32.0, |ui| {
                        ui.label(RichText::new(theme::icon::DIAGNOSTIC).font(theme::icon_font(17.0)).color(CYAN));
                        ui.add_space(2.0);
                        ui.label(RichText::new("Телефон не подключается?").size(15.5).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if running {
                                ui.spinner();
                            } else if ui.add_enabled(!self.busy, egui::Button::new(if found.is_some() { "Проверить ещё раз" } else { "Проверить" })).clicked() {
                                act = Some(Act::Diag(None));
                            }
                        });
                    });
                    ui.add(
                        egui::Label::new(muted(
                            "Лаунчер проверит, отвечает ли раздача, тип сети, брандмауэр Windows, антивирус и VPN, и подскажет, что мешает телефону. Проверка занимает несколько секунд.",
                        ))
                        .wrap(),
                    );
                    if let Some(list) = &found {
                        ui.add_space(6.0);
                        for item in list {
                            if finding_row(ui, item, !running && !self.busy) {
                                act = Some(Act::Diag(item.fix.clone()));
                            }
                        }
                    }
                });
            }
            ui.add_space(10.0);
            self.gateway_card(ui, ctx);
            ui.add_space(10.0);
            theme::card(ui, |ui| {
                theme::row(ui, 32.0, |ui| {
                    ui.label(RichText::new(theme::icon::INFO).font(theme::icon_font(17.0)).color(CYAN));
                    ui.add_space(2.0);
                    ui.label(RichText::new("Как подключить телефон").size(15.5).strong());
                });
                let a = addr.clone().unwrap_or_else(|| "адрес_компьютера:порт".into());
                ui.add(egui::Label::new(muted("Телефон должен быть в той же Wi-Fi сети, что и компьютер.")).wrap());
                ui.add_space(6.0);
                ui.label(RichText::new("Через приложение v2rayNG (рекомендуем)").strong());
                step(ui, 1, "Установите v2rayNG из Google Play или с GitHub.");
                step(ui, 2, "Переключите QR-код выше на «Ссылка для v2rayNG». В v2rayNG: плюс → «Сканировать QR-код». Или плюс → «Вручную: SOCKS» с адресом и портом раздачи без пароля.");
                step(ui, 3, "Выберите сервер HolyZapret, нажмите запуск внизу и разрешите VPN.");
                ui.add(egui::Label::new(muted("Через компьютер пойдёт всё, включая звонки Discord. Выключили v2rayNG — интернет на телефоне снова напрямую.")).wrap());
                ui.add_space(8.0);
                ui.label(RichText::new("Через прокси в настройках Wi-Fi, без приложений").strong());
                step(ui, 1, "Настройки → Wi-Fi → ваша сеть → Изменить → Дополнительно → Прокси → Автоматическая настройка (PAC).");
                step(ui, 2, &format!("Адрес PAC: http://{a}/proxy.pac. С ним при выключенной раздаче телефон идёт в интернет напрямую, пока лаунчер открыт."));
                step(ui, 3, &format!("Или Прокси → Вручную: имя хоста и порт из адреса {a}."));
                ui.add(egui::Label::new(muted("Так работают браузер и большинство приложений, но не звонки.")).wrap());
                ui.add_space(8.0);
                ui.add(egui::Label::new(muted("iPhone, телевизор с Android TV или PlayStation: откройте страницу из QR-кода и выберите устройство, там пошагово.")).wrap());
            });
        });
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
            None => {}
        }
    }
}
