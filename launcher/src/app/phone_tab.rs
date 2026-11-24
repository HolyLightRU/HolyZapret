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

fn qr(ui: &mut egui::Ui, text: &str, side: f32) {
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

impl App {
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
