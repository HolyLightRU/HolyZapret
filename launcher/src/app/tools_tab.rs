use super::{App, Msg, Tab, notify, result_msg, send};
use crate::core::gamefilter::{self, GameFilter, Mode};
use crate::core::hosts;
use crate::log::Lvl;
use crate::theme::{self, CYAN, RED, YELLOW};
use eframe::egui::{self, Color32, RichText};

impl App {
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

    pub(super) fn autostart_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, first: bool) {
        let mut on = self.autostart.unwrap_or(false);
        let ready = self.autostart.is_some() && !self.busy;
        if theme::setting_row(ui, first, "Запускать вместе с Windows", "Стартует свёрнутым в трей и следит за защитой", "", &mut on, ready) {
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

    pub(super) fn auto_update_row(&mut self, ui: &mut egui::Ui, first: bool) {
        let mut on = self.auto_update;
        if theme::setting_row(ui, first, "Обновлять самому", "Когда лаунчер в трее и компьютер простаивает", "Когда лаунчер в трее и 10 минут никто не трогает компьютер, поставит новую версию", &mut on, true) {
            self.auto_update = on;
        }
    }

    pub(super) fn delete_old_folder(&mut self, ctx: &egui::Context) {
        let Some(old) = self.old_folder.take() else { return };
        self.spawn(ctx, move |p, tx| match crate::core::relocate::remove_old(&old, &p.root) {
            Ok((_, true)) => notify(tx, Lvl::Ok, format!("Старая папка {} удалена в корзину", old.display())),
            Ok((_, false)) => notify(
                tx,
                Lvl::Ok,
                format!("Файлы HolyZapret из {} отправлены в корзину, ваши остальные файлы остались на месте", old.display()),
            ),
            Err(e) => notify(tx, Lvl::Err, format!("Старую папку удалить не удалось: {e}")),
        });
    }

    pub(super) fn move_folder(&mut self, ctx: &egui::Context) {
        if self.picker.running() {
            self.toast(ctx, Lvl::Info, "Сначала остановите подбор");
            return;
        }
        let target = crate::core::relocate::target_dir();
        self.spawn(ctx, move |p, tx| {
            send(tx, Lvl::Info, format!("Переношу HolyZapret в {}…", target.display()));
            match crate::core::relocate::run(p, target) {
                Ok((root, exe)) => {
                    let _ = tx.send(Msg::Relocated(root, exe, p.root.clone()));
                }
                Err(e) => notify(tx, Lvl::Err, format!("Перенести не удалось: {e}")),
            }
        });
    }

    pub(super) fn reset_all(&mut self, ctx: &egui::Context) {
        if self.picker.running() {
            self.toast(ctx, Lvl::Info, "Сначала остановите подбор");
            return;
        }
        let installed = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from));
        let strategy = installed.and_then(|id| self.strategies.iter().find(|s| s.matches(&id)).cloned());
        self.spawn(ctx, move |p, tx| {
            match crate::core::reset::run(p) {
                Ok(n) => {
                    let mut msg = format!("Настройки и списки сброшены, очищено записей: {n}");
                    if let Some(s) = strategy {
                        match crate::core::service::install(p, &s) {
                            Ok(_) => msg.push_str(". Служба перезапущена с чистыми списками"),
                            Err(e) => msg.push_str(&format!(", но службу не удалось перезапустить: {e}")),
                        }
                    }
                    notify(tx, Lvl::Ok, msg);
                }
                Err(e) => notify(tx, Lvl::Err, format!("Сброс не удался: {e}")),
            }
            let _ = tx.send(Msg::ResetDone);
        });
    }

    pub(super) fn settings_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.tab == Tab::Lists && !self.advanced {
            self.tab = Tab::Tools;
        }
        self.settings_last = self.tab;
        let issues = self.diag.findings.iter().filter(|f| f.level != crate::core::diagnostics::Level::Ok).count();
        let mut tabs = vec![(Tab::Tools, "Основное".to_string()), (Tab::Diagnostics, if issues > 0 { format!("Диагностика · {issues}") } else { "Диагностика".to_string() })];
        if self.advanced {
            tabs.push((Tab::Lists, "Списки".to_string()));
        }
        tabs.push((Tab::Log, "Журнал".to_string()));
        let labels: Vec<String> = tabs.iter().map(|t| t.1.clone()).collect();
        let mut idx = tabs.iter().position(|t| t.0 == self.tab).unwrap_or(0);
        if theme::segmented(ui, &labels, &mut idx) {
            self.tab = tabs[idx].0;
            self.settings_last = self.tab;
        }
        ui.add_space(4.0);
        match self.tab {
            Tab::Diagnostics => self.diag_tab(ui, ctx),
            Tab::Lists => self.lists_tab(ui, ctx),
            Tab::Log => self.log_tab(ui, ctx),
            _ => self.tools_tab(ui, ctx),
        }
    }

    pub(super) fn tools_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.update_card(ui, ctx);
            ui.add_space(2.0);
            self.ray_card(ui, ctx);
            ui.add_space(2.0);
            self.launcher_card(ui, ctx);
            ui.add_space(2.0);
            self.warp_card(ui, ctx);
            if self.tg.client.is_none() {
                ui.add_space(2.0);
                self.tg_card(ui, ctx);
            }
            ui.add_space(2.0);
            self.service_card(ui, ctx);
        });
    }

    fn update_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let free = !self.busy && self.paths.is_some();
        let newer = self.update_available();
        let local = self.local_version.clone().unwrap_or_else(|| "?".into());
        let (ver_text, ver_color) = match (&newer, &self.remote_version) {
            (Some(_), _) if self.update.running => (self.update.status().unwrap_or_default(), YELLOW),
            (Some(r), _) if crate::core::TEST_BUILD => (format!("тестовая сборка, есть релиз {r}"), YELLOW),
            (Some(r), _) => (format!("доступна новая версия {r}"), YELLOW),
            (None, Some(r)) if crate::core::TEST_BUILD => (format!("тестовая сборка новее релиза {r}"), YELLOW),
            (None, None) if crate::core::TEST_BUILD && self.version_error.is_none() => ("тестовая сборка, проверяю релизы…".to_string(), YELLOW),
            (None, Some(_)) => ("последняя версия".to_string(), Color32::from_gray(175)),
            (None, None) => match &self.version_error {
                Some(e) => (format!("не удалось проверить: {e}"), RED),
                None => ("проверяю…".to_string(), Color32::from_gray(175)),
            },
        };
        let mut check = false;
        let mut update = false;
        theme::card(ui, |ui| {
            theme::row(ui, 40.0, |ui| {
                ui.label(RichText::new(theme::icon::REFRESH).font(theme::icon_font(17.0)).color(CYAN));
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(format!("HolyZapret {local}")).font(theme::semibold(15.0)).strong().color(Color32::WHITE));
                    ui.label(RichText::new(ver_text).color(ver_color).size(12.5))
                        .on_hover_text("Скачивает последний релиз с GitHub и обновляет движок, списки и лаунчер");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(r) = &newer {
                        let label = if crate::core::TEST_BUILD { format!("Перейти на релиз {r}") } else { format!("Обновить до {r}") };
                        ui.add_enabled_ui(free, |ui| {
                            if ui.add(theme::Primary(label)).clicked() {
                                update = true;
                            }
                        });
                    }
                    if crate::theme::button(ui, "Проверить").clicked() {
                        check = true;
                    }
                    if crate::theme::button(ui, "★ GitHub")
                        .on_hover_text("Исходный код и все релизы. Если HolyZapret помогает, поставьте звезду: так его проще найти другим")
                        .clicked()
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(super::whats_new::REPO));
                    }
                });
            });
        });
        if update {
            self.open_update();
        } else if check {
            self.check_version(ctx);
        }
    }

    fn launcher_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        theme::card(ui, |ui| {
            ui.label(RichText::new("Лаунчер").font(theme::semibold(15.0)).strong());
            ui.add_space(2.0);
            self.autostart_row(ui, ctx, true);
            let has_tray = self.tray;
            let mut tray = self.close_to_tray;
            if theme::setting_row(ui, false, "Сворачивать в трей при закрытии", "Выход — через меню в трее", "Лаунчер продолжит работать в фоне", &mut tray, has_tray) {
                self.close_to_tray = tray;
            }
            self.auto_update_row(ui, false);
            let mut watch = self.watch;
            if theme::setting_row(ui, false, "Следить за работой обхода", "Раз в 20 минут проверяет YouTube и Discord", "Сообщит в трее, если обход перестал работать", &mut watch, true) {
                self.watch = watch;
            }
            let mut auto = self.auto_pick;
            if theme::setting_row(
                ui,
                false,
                "Подбирать приём самому",
                "Если обход сломался и компьютер простаивает",
                "Если обход сломался и 15 минут никто не трогает компьютер, подберёт приём получше. Работает вместе со слежением",
                &mut auto,
                self.watch,
            ) {
                self.auto_pick = auto;
            }
            let mut flow = self.flow;
            if theme::setting_row(ui, false, "Живые градиенты", "Цвета акцентов медленно перетекают", "Только пока окно лаунчера активно", &mut flow, true) {
                self.flow = flow;
            }
            let mut adv = self.advanced;
            if theme::setting_row(ui, false, "Продвинутый режим", "Раздел «Списки» и дополнительные поля", "Свои домены и IP, поиск доменов по ссылке, порт и домен Telegram-прокси", &mut adv, true) {
                self.advanced = adv;
                if !adv && self.tab == Tab::Lists {
                    self.tab = Tab::Tools;
                }
            }
        });
    }

    fn service_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let free = !self.busy && self.paths.is_some();
        let mut hosts = false;
        let mut discord = false;
        theme::card(ui, |ui| {
            ui.label(RichText::new("Обслуживание").font(theme::semibold(15.0)).strong());
            ui.add_space(2.0);
            action_row(ui, true, "Поделиться настройкой", "Приём, свои домены и IP, правила и игровой фильтр одним кодом", false, |ui| {
                if ui.add_enabled(self.paths.is_some(), crate::theme::GlassButton::new("Скопировать мой код")).clicked() {
                    self.copy_share_code(ctx);
                }
                if ui.add_enabled(!self.busy, crate::theme::GlassButton::new("Код от друга")).clicked() {
                    self.share.open = true;
                }
            });
            action_row(ui, false, "Файл hosts", "Скачать hosts-list с GitHub, старый сохранится как hosts.bak", false, |ui| {
                if ui.add_enabled(free, crate::theme::GlassButton::new("Обновить")).clicked() {
                    hosts = true;
                }
            });
            action_row(ui, false, "Кэш Discord", "Закроет Discord и очистит Cache, Code Cache и GPUCache", false, |ui| {
                if ui.add_enabled(!self.busy, crate::theme::GlassButton::new("Очистить")).clicked() {
                    discord = true;
                }
            });
            action_row(ui, false, "Мастер первой настройки", "Диагностика, подбор и включение защиты за три шага", false, |ui| {
                if crate::theme::button(ui, "Открыть").clicked() {
                    self.open_wizard();
                }
            });
            action_row(ui, false, "Сбросить мои настройки и списки", "Свои домены, IP, правила и настройки вернутся к исходным", true, |ui| {
                if ui.add_enabled(!self.busy, crate::theme::GlassButton::new("Сбросить")).clicked() {
                    self.confirm = Some(super::confirm::Confirm::ResetAll);
                }
            });
        });
        if hosts {
            self.run_hosts(ctx);
        }
        if discord {
            self.spawn(ctx, |_, tx| result_msg(tx, crate::core::diagnostics::clear_discord_cache(), "Кэш Discord"));
        }
    }

    pub(super) fn game_filter_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.game_edit.is_none() {
            if let Some(s) = &self.status {
                self.game_edit = Some((s.game.clone(), s.game.tcp.clone(), s.game.udp.clone()));
            }
        }
        let Some((gf, _, _)) = self.game_edit.clone() else { return };
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, Color32::from_white_alpha(10)));
        let modes = [Mode::Disabled, Mode::All, Mode::Tcp, Mode::Udp];
        let labels: Vec<String> = ["Выкл", "TCP и UDP", "TCP", "UDP"].iter().map(|s| s.to_string()).collect();
        let mut idx = modes.iter().position(|m| *m == gf.mode).unwrap_or(0);
        let mut changed = false;
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 50.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = theme::segmented(ui, &labels, &mut idx);
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(7.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("Игровой фильтр").color(Color32::from_gray(232)).size(14.5));
                ui.add(egui::Label::new(RichText::new(gf.summary()).color(Color32::from_gray(165)).size(12.5)).truncate());
            });
        })
        .response
        .on_hover_text("Порты для игр и других программ: на них движок подбирает приём сам. Порты меняются в «Дополнительно»");
        if changed {
            let new = GameFilter { mode: modes[idx], ..gf.clone() };
            self.save_game_filter(ctx, new);
        }
    }

    pub(super) fn game_ports(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((gf, tcp_in, udp_in)) = self.game_edit.clone() else { return };
        let mut tcp = tcp_in;
        let mut udp = udp_in;
        let mut save = false;
        theme::row(ui, 40.0, |ui| {
            theme::row_label(ui, "Порты TCP", 80.0);
            ui.add(egui::TextEdit::singleline(&mut tcp).hint_text(gamefilter::DEFAULT_RANGE).desired_width(150.0));
            ui.add_space(12.0);
            theme::row_label(ui, "UDP", 36.0);
            ui.add(egui::TextEdit::singleline(&mut udp).hint_text(gamefilter::DEFAULT_RANGE).desired_width(150.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = tcp != gf.tcp || udp != gf.udp;
                if ui.add_enabled(dirty, crate::theme::GlassButton::new("Сохранить")).clicked() {
                    save = true;
                }
            });
        });
        if let Some(e) = &self.game_err {
            ui.label(RichText::new(e).color(RED).size(12.5));
        } else {
            ui.label(RichText::new("Порты и диапазоны через запятую, например 1024-1934,1936-65535").color(Color32::from_gray(165)).size(12.5));
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
    }
}

fn action_row(ui: &mut egui::Ui, first: bool, title: &str, sub: &str, danger: bool, right: impl FnOnce(&mut egui::Ui)) {
    if !first {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 50.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        right(ui);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.add_space(7.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            let col = if danger { Color32::from_rgb(246, 170, 176) } else { Color32::from_gray(232) };
            ui.label(RichText::new(title).color(col).size(14.5));
            ui.add(egui::Label::new(RichText::new(sub).color(Color32::from_gray(165)).size(12.5)).truncate());
        });
    });
}
