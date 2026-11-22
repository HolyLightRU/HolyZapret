use super::{App, Msg, Tab, notify, result_msg, send};
use crate::core::gamefilter::{self, GameFilter, Mode};
use crate::core::service::ServiceState;
use crate::core::{flowseal, hosts, update};
use crate::log::Lvl;
use crate::theme::{self, CYAN, RED, YELLOW};
use eframe::egui::{self, Color32, RichText};

struct Action {
    label: String,
    primary: bool,
    enabled: bool,
}

fn act(label: impl Into<String>, primary: bool, enabled: bool) -> Action {
    Action { label: label.into(), primary, enabled }
}

fn tool_card(ui: &mut egui::Ui, glyph: &str, title: &str, desc: &str, status: Option<(String, Color32)>, actions: &[Action]) -> Option<usize> {
    let mut clicked = None;
    theme::card(ui, |ui| {
        let top = ui.min_rect().top();
        theme::row(ui, 30.0, |ui| {
            ui.label(RichText::new(glyph).font(theme::icon_font(17.0)).color(CYAN));
            ui.add_space(2.0);
            ui.label(RichText::new(title).size(15.5).strong());
        });
        ui.add(egui::Label::new(RichText::new(desc).color(Color32::from_gray(165)).size(12.5)).wrap());
        if let Some((text, color)) = &status {
            ui.label(RichText::new(text).color(*color).size(12.5));
        }
        let used = ui.min_rect().bottom() - top;
        if used < CARD_BODY {
            ui.add_space(CARD_BODY - used);
        }
        theme::row(ui, 36.0, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (i, a) in actions.iter().enumerate() {
                    let r = ui.add_enabled_ui(a.enabled, |ui| {
                        if a.primary { ui.add(theme::Primary(a.label.clone())) } else { ui.button(&a.label) }
                    });
                    if r.inner.clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    });
    clicked
}

const CARD_BODY: f32 = 104.0;

fn setting_row(ui: &mut egui::Ui, value: &mut bool, enabled: bool, title: &str, sub: &str) -> bool {
    let mut changed = false;
    theme::row(ui, 44.0, |ui| {
        ui.vertical(|ui| {
            ui.label(title);
            ui.label(RichText::new(sub).weak().size(12.5));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                changed = theme::toggle(ui, value).changed();
            });
        });
    });
    changed
}

impl App {
    fn flowseal_in_use(&self) -> bool {
        self.status.as_ref().is_some_and(|s| {
            s.service != ServiceState::NotInstalled
                && s.active_strategy().is_some_and(|n| n.to_lowercase().starts_with("flowseal\\"))
        })
    }

    fn ensure_flowseal_latest(&mut self, ctx: &egui::Context) {
        if self.flowseal_latest.is_none() && !self.flowseal_checking {
            self.flowseal_checking = true;
            let (tx, ctx2) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Msg::FlowsealLatest(flowseal::latest().map_err(|e| e.to_string())));
                ctx2.request_repaint();
            });
        }
    }

    fn run_hosts(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, |p, tx| {
            send(tx, Lvl::Info, "Обновляю hosts-list…");
            if let Err(e) = hosts::download_list(p) {
                send(tx, Lvl::Err, format!("Не удалось скачать список, использую локальный: {e}"));
            }
            match hosts::apply(p) {
                Ok(lines) => {
                    let n = lines.len();
                    for l in lines {
                        send(tx, Lvl::Info, l);
                    }
                    notify(tx, Lvl::Ok, format!("hosts обновлён, записей: {n}"));
                }
                Err(e) => notify(tx, Lvl::Err, format!("Ошибка записи hosts: {e}")),
            }
        });
    }

    fn run_auto_update(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, |p, tx| {
            result_msg(
                tx,
                update::update_auto_update(p).map(|_| "auto_update.bat обновлён".to_string()),
                "Ошибка загрузки auto_update.bat",
            );
            let _ = tx.send(Msg::Rescan);
        });
    }

    pub(super) fn autostart_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut on = self.autostart.unwrap_or(false);
        let ready = self.autostart.is_some() && !self.busy;
        if setting_row(ui, &mut on, ready, "Запускать вместе с Windows", "Лаунчер стартует свёрнутым в трей и следит за защитой") {
            self.autostart = None;
            self.spawn(ctx, move |_, tx| {
                let r = crate::core::autostart::set(on).map(|_| {
                    if on { "Лаунчер будет запускаться вместе с Windows".to_string() } else { "Автозапуск лаунчера выключен".to_string() }
                });
                result_msg(tx, r, "Автозапуск");
                let _ = tx.send(Msg::Autostart(crate::core::autostart::enabled()));
            });
        }
    }

    pub(super) fn run_update(&mut self, ctx: &egui::Context) {
        let Some(rel) = self.remote_release.clone() else {
            self.check_version(ctx);
            return;
        };
        if self.trial.is_some() || self.status.as_ref().is_some_and(|s| s.engine_running) {
            self.toast(ctx, Lvl::Err, "Сначала выключите защиту и пробный запуск: движок держит файлы, которые нужно обновить");
            return;
        }
        self.spawn(ctx, move |p, tx| {
            send(tx, Lvl::Info, format!("Скачиваю HolyZapret {}…", rel.version));
            match crate::core::selfupdate::apply(p, &rel) {
                Ok(sum) => {
                    let lvl = Lvl::Ok;
                    notify(tx, lvl, sum.to_string());
                }
                Err(e) => notify(tx, Lvl::Err, format!("Обновление не удалось: {e}")),
            }
            let _ = tx.send(Msg::Updated);
        });
    }

    pub(super) fn flowseal_action(&mut self, ctx: &egui::Context, install: bool) {
        if self.flowseal_in_use() {
            self.toast(ctx, Lvl::Err, "Служба сейчас использует стратегию Flowseal, сначала удалите службу");
            return;
        }
        if install {
            let cached = self.flowseal_latest.clone();
            self.spawn(ctx, move |p, tx| {
                let rel = match cached {
                    Some(r) => Ok(r),
                    None => flowseal::latest(),
                };
                let r = rel.and_then(|rel| {
                    send(tx, Lvl::Info, format!("Скачиваю Flowseal {}…", rel.version));
                    flowseal::install(p, &rel).map(|n| format!("Flowseal {} установлен, стратегий: {n}", rel.version))
                });
                result_msg(tx, r, "Не удалось установить Flowseal");
                let _ = tx.send(Msg::Rescan);
            });
        } else {
            self.spawn(ctx, |p, tx| {
                result_msg(tx, flowseal::uninstall(p).map(|_| "Стратегии Flowseal удалены".to_string()), "Не удалось удалить Flowseal");
                let _ = tx.send(Msg::Rescan);
            });
        }
    }

    pub(super) fn tools_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.ensure_flowseal_latest(ctx);
        let free = !self.busy && self.paths.is_some();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut clicks = [None; 4];
            let mut update_offered = false;
            ui.columns(2, |cols| {
                clicks[0] = tool_card(
                    &mut cols[0],
                    theme::icon::GLOBE,
                    "Файл hosts",
                    "Скачивает hosts-list с GitHub и прописывает записи в hosts. Старый файл сохраняется как hosts.bak",
                    None,
                    &[act("Обновить", false, free)],
                );
                let newer = self.update_available();
                let local = self.local_version.clone().unwrap_or_else(|| "?".into());
                let (ver_text, ver_color) = match (&newer, &self.remote_version) {
                    (Some(r), _) => (format!("{local} → {r}"), YELLOW),
                    (None, _) if crate::core::TEST_BUILD => ("Тестовая сборка: обновления выключены".to_string(), YELLOW),
                    (None, Some(_)) => (format!("Установлена {local}, последняя"), Color32::from_gray(150)),
                    (None, None) => match &self.version_error {
                        Some(e) => (format!("Не удалось проверить: {e}"), RED),
                        None => ("Проверяю…".to_string(), Color32::from_gray(150)),
                    },
                };
                let mut actions = vec![act("Проверить", false, true)];
                if let Some(r) = &newer {
                    actions.insert(0, act(format!("Обновить до {r}"), true, free));
                }
                clicks[1] = tool_card(
                    &mut cols[1],
                    theme::icon::REFRESH,
                    "Обновления HolyZapret",
                    "Скачивает последний релиз с GitHub и обновляет стратегии, списки и бинарники",
                    Some((ver_text, ver_color)),
                    &actions,
                );
                update_offered = newer.is_some();
            });
            ui.columns(2, |cols| {
                clicks[2] = tool_card(
                    &mut cols[0],
                    theme::icon::DOWNLOAD,
                    "auto_update.bat",
                    "Загружает свежую версию этой стратегии с GitHub",
                    None,
                    &[act("Обновить", false, free)],
                );
                let inst = self.flowseal_version.clone();
                let latest = self.flowseal_latest.as_ref().map(|r| r.version.clone());
                let (fs_text, fs_color) = match (&inst, &latest) {
                    (Some(i), Some(l)) if i == l => (format!("Установлена {i}, последняя"), Color32::from_gray(150)),
                    (Some(i), Some(l)) => (format!("{i} → {l}"), YELLOW),
                    (Some(i), None) => (format!("Установлена {i}"), Color32::from_gray(150)),
                    (None, Some(l)) => (format!("Не установлены, на GitHub {l}"), Color32::from_gray(150)),
                    (None, None) if self.flowseal_checking => ("Проверяю версию…".to_string(), Color32::from_gray(150)),
                    (None, None) => ("Не установлены".to_string(), Color32::from_gray(150)),
                };
                let main = match (&inst, &latest) {
                    (None, _) => "Скачать".to_string(),
                    (Some(i), Some(l)) if i != l => "Обновить".to_string(),
                    _ => "Переустановить".to_string(),
                };
                let mut actions = vec![act(main, inst.is_none() || inst != latest, free)];
                if inst.is_some() {
                    actions.push(act("Удалить", false, free));
                }
                clicks[3] = tool_card(
                    &mut cols[1],
                    theme::icon::LISTS,
                    "Стратегии Flowseal",
                    "Оригинальные стратегии Flowseal со своим движком winws.exe в папке flowseal",
                    Some((fs_text, fs_color)),
                    &actions,
                );
            });
            self.warp_card(ui, ctx);
            if self.tg.client.is_none() {
                self.tg_card(ui, ctx);
            }
            self.settings_card(ui, ctx);
            if clicks[0].is_some() {
                self.run_hosts(ctx);
            }
            match (clicks[1], update_offered) {
                (Some(0), true) => self.confirm = Some(super::confirm::Confirm::Update),
                (Some(_), _) => self.check_version(ctx),
                _ => {}
            }
            if clicks[2].is_some() {
                self.run_auto_update(ctx);
            }
            match clicks[3] {
                Some(0) => self.flowseal_action(ctx, true),
                Some(1) => self.confirm = Some(super::confirm::Confirm::RemoveFlowseal),
                _ => {}
            }
        });
    }

    fn settings_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.game_edit.is_none() {
            if let Some(s) = &self.status {
                self.game_edit = Some((s.game.clone(), s.game.tcp.clone(), s.game.udp.clone()));
            }
        }
        theme::card(ui, |ui| {
            theme::row(ui, 32.0, |ui| {
                ui.label(RichText::new(theme::icon::SETTINGS).font(theme::icon_font(17.0)).color(CYAN));
                ui.add_space(2.0);
                ui.label(RichText::new("Настройки").size(15.5).strong());
            });
            let has_tray = self.tray;
            let mut tray = self.close_to_tray;
            if setting_row(ui, &mut tray, has_tray, "Сворачивать в трей при закрытии", "Лаунчер продолжит работать в фоне, выход через меню в трее") {
                self.close_to_tray = tray;
            }
            self.autostart_row(ui, ctx);
            let mut flow = self.flow;
            if setting_row(ui, &mut flow, true, "Живые градиенты", "Цвета акцентов медленно перетекают, пока окно лаунчера активно") {
                self.flow = flow;
            }
            let mut adv = self.advanced;
            if setting_row(ui, &mut adv, true, "Продвинутый режим", "Вкладка «Списки»: свои домены и IP, поиск доменов по ссылке") {
                self.advanced = adv;
                if !adv && self.tab == Tab::Lists {
                    self.tab = Tab::Main;
                }
            }
            theme::row(ui, 44.0, |ui| {
                ui.vertical(|ui| {
                    ui.label("Мастер настройки");
                    ui.label(RichText::new("Диагностика, подбор и установка стратегии за три шага").weak().size(12.5));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Открыть").clicked() {
                        self.open_wizard();
                    }
                });
            });
            ui.separator();
            let Some((gf, tcp_in, udp_in)) = self.game_edit.clone() else { return };
            theme::row(ui, 44.0, |ui| {
                ui.vertical(|ui| {
                    ui.label("Игровой фильтр");
                    ui.label(RichText::new("Порты для игр и других программ, применяется при установке службы").weak().size(12.5));
                });
            });
            let modes = [Mode::Disabled, Mode::All, Mode::Tcp, Mode::Udp];
            let labels: Vec<String> = ["Выключен", "TCP и UDP", "Только TCP", "Только UDP"].iter().map(|s| s.to_string()).collect();
            let mut idx = modes.iter().position(|m| *m == gf.mode).unwrap_or(0);
            if theme::segmented(ui, &labels, &mut idx) {
                let new = GameFilter { mode: modes[idx], ..gf.clone() };
                self.save_game_filter(ctx, new);
            }
            let mut tcp = tcp_in;
            let mut udp = udp_in;
            let mut save = false;
            theme::row(ui, 40.0, |ui| {
                theme::row_label(ui, "TCP", 40.0);
                ui.add(egui::TextEdit::singleline(&mut tcp).hint_text(gamefilter::DEFAULT_RANGE).desired_width(170.0));
                ui.add_space(12.0);
                theme::row_label(ui, "UDP", 40.0);
                ui.add(egui::TextEdit::singleline(&mut udp).hint_text(gamefilter::DEFAULT_RANGE).desired_width(170.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dirty = tcp != gf.tcp || udp != gf.udp;
                    if ui.add_enabled(dirty, egui::Button::new("Сохранить")).clicked() {
                        save = true;
                    }
                });
            });
            if let Some(e) = &self.game_err {
                ui.label(RichText::new(e).color(RED).size(12.5));
            } else {
                ui.label(RichText::new("Порты и диапазоны через запятую, например 1024-1934,1936-65535").weak().size(12.5));
            }
            if let Some(edit) = &mut self.game_edit {
                edit.1 = tcp.clone();
                edit.2 = udp.clone();
            }
            if save {
                match (gamefilter::validate(&tcp), gamefilter::validate(&udp)) {
                    (Some(t), Some(u)) => {
                        self.game_err = None;
                        self.save_game_filter(ctx, GameFilter { mode: gf.mode, tcp: t, udp: u });
                    }
                    (None, _) => self.game_err = Some("TCP: неверный диапазон портов".into()),
                    (_, None) => self.game_err = Some("UDP: неверный диапазон портов".into()),
                }
            }
        });
    }
}
