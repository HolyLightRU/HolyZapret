use super::{App, Msg, result_msg};
use crate::core::discover::{self, Found};
use crate::core::lists::{self, Doc, Kind, ListFile};
use crate::core::service::{self, ServiceState};
use crate::core::strategy;
use crate::log::Lvl;
use crate::theme::{self, CYAN, PINK, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::path::PathBuf;

#[derive(Default)]
pub struct ListsState {
    mode: usize,
    folder: usize,
    file: usize,
    files: Vec<ListFile>,
    files_for: Option<PathBuf>,
    doc: Doc,
    doc_for: Option<PathBuf>,
    search: String,
    filtered: Vec<usize>,
    filtered_key: Option<(String, u64)>,
    version: u64,
    input: String,
    restart_hint: bool,
    url: String,
    searching: bool,
    found: Option<Found>,
    checks: Vec<(bool, bool)>,
}

impl ListsState {
    pub fn found(&mut self, f: Found) {
        self.searching = false;
        self.checks = f.groups.iter().map(|g| (g.main || g.related, false)).collect();
        self.found = Some(f);
    }

    pub fn search_failed(&mut self) {
        self.searching = false;
    }
}

impl App {
    fn list_folders(&self) -> Vec<PathBuf> {
        let Some(p) = &self.paths else { return Vec::new() };
        let mut v: Vec<PathBuf> = Vec::new();
        for s in &self.strategies {
            let d = strategy::lists_dir(s, p);
            if !v.contains(&d) {
                v.push(d);
            }
        }
        if v.is_empty() {
            v.push(p.exp_lists());
        }
        v
    }

    pub(super) fn lists_reload(&mut self) {
        self.lists.doc_for = None;
    }

    fn lists_sync(&mut self) {
        let folders = self.list_folders();
        if folders.is_empty() {
            return;
        }
        let st = &mut self.lists;
        if st.folder >= folders.len() {
            st.folder = 0;
        }
        let dir = folders[st.folder].clone();
        if st.files_for.as_ref() != Some(&dir) {
            st.files = lists::files(&dir);
            st.files_for = Some(dir);
            st.file = 0;
            st.doc_for = None;
        }
        if st.file >= st.files.len() {
            st.file = 0;
        }
        let Some(f) = st.files.get(st.file) else { return };
        if st.doc_for.as_ref() != Some(&f.path) {
            st.doc = Doc::load(&f.path);
            st.doc_for = Some(f.path.clone());
            st.version += 1;
        }
        let key = (st.search.trim().to_lowercase(), st.version);
        if st.filtered_key.as_ref() != Some(&key) {
            st.filtered = (0..st.doc.len())
                .filter(|&n| key.0.is_empty() || st.doc.entry(n).to_lowercase().contains(&key.0))
                .collect();
            st.filtered_key = Some(key);
        }
    }

    pub(super) fn service_installed(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.service != ServiceState::NotInstalled)
    }

    fn after_change(&mut self) {
        self.lists.version += 1;
        if self.service_installed() {
            self.lists.restart_hint = true;
        }
    }

    pub(super) fn lists_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.lists_sync();
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Списки").font(crate::theme::semibold(17.0)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.lists.restart_hint {
                        let b = ui
                            .add_enabled(!self.busy, crate::theme::GlassButton::new("Перезапустить службу"))
                            .on_hover_text("Движок перечитает списки при запуске");
                        if b.clicked() {
                            self.lists.restart_hint = false;
                            self.spawn(ctx, |_, tx| {
                                result_msg(
                                    tx,
                                    service::restart().map(|_| "Служба перезапущена".to_string()),
                                    "Не удалось перезапустить службу",
                                )
                            });
                        }
                        theme::chip(ui, "Изменения применятся после перезапуска", YELLOW);
                    }
                });
            });
            let modes = vec!["Редактор".to_string(), "Поиск по ссылке".to_string()];
            theme::segmented(ui, &modes, &mut self.lists.mode);
            ui.add_space(2.0);
            if self.lists.mode == 0 {
                self.lists_editor(ui, ctx);
            } else {
                self.lists_discover(ui, ctx);
            }
        });
    }

    fn lists_editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let folders = self.list_folders();
        if self.lists.files.is_empty() {
            ui.label(RichText::new("Папка со списками не найдена").weak());
            return;
        }
        theme::row(ui, 38.0, |ui| {
            if folders.len() > 1 {
                let root = self.paths.as_ref().map(|p| p.root.clone()).unwrap_or_default();
                let labels: Vec<String> = folders
                    .iter()
                    .map(|f| f.strip_prefix(&root).map(|r| r.display().to_string()).unwrap_or_else(|_| f.display().to_string()))
                    .collect();
                theme::segmented(ui, &labels, &mut self.lists.folder);
            }
            let selected = self.lists.files[self.lists.file].label.clone();
            egui::ComboBox::from_id_salt("list_file").selected_text(selected).width(230.0).show_ui(ui, |ui| {
                let mut in_user = true;
                for (i, f) in self.lists.files.iter().enumerate() {
                    if in_user && !f.user {
                        ui.separator();
                        in_user = false;
                    }
                    ui.selectable_value(&mut self.lists.file, i, &f.label);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.lists.search).hint_text("Поиск по списку").desired_width(170.0));
            });
        });

        let f = self.lists.files[self.lists.file].clone();
        let hint = match f.kind {
            Kind::Domains => "Домены или ссылки через пробел: example.com https://site.ru/page",
            Kind::Ips => "IP или подсети через пробел: 1.2.3.4 10.0.0.0/24",
        };
        let mut add = false;
        theme::row(ui, 42.0, |ui| {
            let w = (ui.available_width() - 130.0).max(120.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.lists.input).hint_text(hint).desired_width(w));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                add = true;
            }
            if ui.add(theme::Primary("Добавить".into())).clicked() {
                add = true;
            }
        });
        let note = if f.user {
            "Свой список: при установке службы RayOfLight берёт его вместе с основным"
        } else {
            "Основной список: может быть перезаписан при обновлении HolyZapret"
        };
        ui.label(RichText::new(note).weak().size(12.5));
        if add {
            self.lists_add(ctx, &f);
            self.lists_sync();
        }

        let total = self.lists.doc.len();
        let n = self.lists.filtered.len();
        let count = if self.lists.search.trim().is_empty() {
            format!("Записей: {total}")
        } else {
            format!("Найдено {n} из {total}")
        };
        ui.label(RichText::new(count).color(Color32::from_gray(150)).size(12.5));

        let mut remove = None;
        if n == 0 {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                let t = if total == 0 { "Список пуст. Добавьте записи в поле выше" } else { "Ничего не найдено" };
                ui.label(RichText::new(t).weak());
            });
        } else {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 30.0, n, |ui, range| {
                for k in range {
                    let idx = self.lists.filtered[k];
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
                    let hovered = ui.rect_contains_pointer(rect);
                    if hovered {
                        ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(10));
                    }
                    ui.painter().text(
                        egui::pos2(rect.left() + 12.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        self.lists.doc.entry(idx),
                        egui::FontId::monospace(13.5),
                        Color32::from_gray(220),
                    );
                    if hovered {
                        let b = egui::Rect::from_center_size(egui::pos2(rect.right() - 20.0, rect.center().y), egui::vec2(26.0, 26.0));
                        let br = ui.interact(b, ui.make_persistent_id(("list_del", idx)), egui::Sense::click());
                        if br.hovered() {
                            ui.painter().rect_filled(b, CornerRadius::same(6), Color32::from_rgba_unmultiplied(244, 100, 110, 50));
                        }
                        ui.painter().text(
                            b.center(),
                            egui::Align2::CENTER_CENTER,
                            theme::icon::DELETE,
                            theme::icon_font(13.0),
                            Color32::from_gray(210),
                        );
                        if br.on_hover_text("Удалить").clicked() {
                            remove = Some(idx);
                        }
                    }
                }
            });
        }
        if let Some(idx) = remove {
            if let Some(v) = self.lists.doc.remove(idx) {
                match self.lists.doc.save(&f.path, f.kind) {
                    Ok(()) => {
                        self.after_change();
                        self.toast(ctx, Lvl::Info, format!("Удалено: {v}"));
                    }
                    Err(e) => {
                        self.lists.doc_for = None;
                        self.toast(ctx, Lvl::Err, format!("Не удалось сохранить {}: {e}", f.label));
                    }
                }
            }
        }
    }

    fn lists_add(&mut self, ctx: &egui::Context, f: &ListFile) {
        let (ok, bad) = lists::parse_input(f.kind, &self.lists.input);
        for b in bad.iter().take(3) {
            self.toast(ctx, Lvl::Err, b.clone());
        }
        if ok.is_empty() {
            if bad.is_empty() {
                self.toast(ctx, Lvl::Info, "Введите записи для добавления");
            }
            return;
        }
        let n = self.lists.doc.add(&ok);
        if n == 0 {
            self.toast(ctx, Lvl::Info, "Все записи уже есть в списке");
        } else {
            match self.lists.doc.save(&f.path, f.kind) {
                Ok(()) => {
                    self.after_change();
                    self.toast(ctx, Lvl::Ok, format!("Добавлено в «{}»: {n}", f.label));
                    let known = crate::core::known::names(&ok);
                    if f.kind == Kind::Domains && !f.path.ends_with("list-exclude-user.txt") && !known.is_empty() {
                        self.toast(
                            ctx,
                            Lvl::Info,
                            format!(
                                "{} обычно работает без обхода, и обход может его сломать. Если он перестанет открываться, уберите его из списка",
                                known.join(", ")
                            ),
                        );
                    }
                }
                Err(e) => {
                    self.lists.doc_for = None;
                    self.toast(ctx, Lvl::Err, format!("Не удалось сохранить {}: {e}", f.label));
                    return;
                }
            }
        }
        if bad.is_empty() {
            self.lists.input.clear();
        }
    }

    fn lists_discover(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut go = false;
        theme::row(ui, 42.0, |ui| {
            let w = (ui.available_width() - 110.0).max(120.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.lists.url)
                    .hint_text("Ссылка на сайт, например open.spotify.com")
                    .desired_width(w),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            ui.add_enabled_ui(!self.lists.searching, |ui| {
                if ui.add(theme::Primary("Найти".into())).clicked() {
                    go = true;
                }
            });
        });
        if go && !self.lists.searching && !self.lists.url.trim().is_empty() {
            self.lists.searching = true;
            self.lists.found = None;
            let url = self.lists.url.trim().to_string();
            let (tx, ctx2) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let r = discover::run(&url).map_err(|e| e.to_string());
                let _ = tx.send(Msg::Found(r));
                ctx2.request_repaint();
            });
        }
        if self.lists.searching {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Загружаю страницу и скрипты, определяю IP…");
            });
            return;
        }
        let Some(found) = &self.lists.found else {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Лаунчер загрузит страницу, найдёт домены в HTML, CSS и JS и определит их IP. \
                     Домены, к которым сайт обращается уже из работающего скрипта, так не найти.",
                )
                .weak(),
            );
            return;
        };
        ui.label(
            RichText::new(format!(
                "{} · доменов: {} · файлов просмотрено: {}",
                found.final_url,
                found.groups.len(),
                found.assets_scanned
            ))
            .color(Color32::from_gray(150))
            .size(12.5),
        );
        let st = &mut self.lists;
        let found = st.found.as_ref().unwrap();
        let checks = &mut st.checks;
        let h = (ui.available_height() - 56.0).max(80.0);
        egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("Домен").weak().size(12.0));
                ui.add_space(4.0);
                ui.label(RichText::new("IP").weak().size(12.0));
            });
            for (g, (d, i)) in found.groups.iter().zip(checks.iter_mut()) {
                ui.horizontal(|ui| {
                    ui.checkbox(d, "").on_hover_text("Добавить домен в «Мои домены»");
                    ui.add_enabled_ui(!g.ips.is_empty(), |ui| {
                        ui.checkbox(i, "").on_hover_text("Добавить IP в «Мои IP»");
                    });
                    let name = RichText::new(&g.base).color(if g.third_party { Color32::from_gray(150) } else { Color32::WHITE });
                    ui.label(name);
                    if g.hosts.len() > 1 || g.hosts.first() != Some(&g.base) {
                        ui.label(RichText::new(format!("{} адр.", g.hosts.len())).weak().size(12.0))
                            .on_hover_text(g.hosts.join("\n"));
                    }
                    if g.main {
                        theme::chip(ui, "Основной", PINK);
                    } else if g.related {
                        theme::chip(ui, "Связанный", CYAN);
                    } else if g.third_party {
                        theme::chip(ui, "Сторонний", Color32::from_gray(150));
                    }
                    if g.cloudflare {
                        theme::chip(ui, "Cloudflare", YELLOW)
                            .on_hover_text("IP Cloudflare общие для тысяч сайтов: обход по ним затронет их все");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} IP", g.ips.len())).weak().size(12.5))
                            .on_hover_text(g.ips.iter().map(|ip| ip.to_string()).collect::<Vec<_>>().join("\n"));
                    });
                });
            }
        });
        let nd = checks.iter().filter(|c| c.0).count();
        let ni: usize = found.groups.iter().zip(checks.iter()).filter(|(_, c)| c.1).map(|(g, _)| g.ips.len()).sum();
        let cf = found.groups.iter().zip(checks.iter()).any(|(g, c)| c.1 && g.cloudflare);
        let mut add = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Отмечено доменов: {nd}, IP: {ni}")).color(Color32::from_gray(170)));
            if cf {
                ui.label(RichText::new("· среди IP есть Cloudflare").color(YELLOW).size(12.5));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(nd + ni > 0, |ui| {
                    if ui.add(theme::Primary("Добавить отмеченное".into())).clicked() {
                        add = true;
                    }
                });
            });
        });
        if add {
            self.lists_add_found(ctx);
        }
    }

    fn lists_add_found(&mut self, ctx: &egui::Context) {
        let folders = self.list_folders();
        let Some(dir) = folders.get(self.lists.folder).cloned() else { return };
        let Some(found) = &self.lists.found else { return };
        let mut domains = Vec::new();
        let mut ips = Vec::new();
        for (g, (d, i)) in found.groups.iter().zip(&self.lists.checks) {
            if *d {
                domains.push(g.base.clone());
            }
            if *i {
                for ip in &g.ips {
                    ips.push(format!("{ip}/{}", if ip.is_ipv4() { 32 } else { 128 }));
                }
            }
        }
        let mut added = [0usize; 2];
        for (slot, file, items, kind) in
            [(0, "list-general-user.txt", domains, Kind::Domains), (1, "ipset-all-user.txt", ips, Kind::Ips)]
        {
            if items.is_empty() {
                continue;
            }
            let p = dir.join(file);
            let mut d = Doc::load(&p);
            let n = d.add(&items);
            if n > 0 {
                if let Err(e) = d.save(&p, kind) {
                    self.toast(ctx, Lvl::Err, format!("Не удалось сохранить {file}: {e}"));
                    return;
                }
            }
            added[slot] = n;
        }
        self.lists.doc_for = None;
        if added[0] + added[1] == 0 {
            self.toast(ctx, Lvl::Info, "Всё отмеченное уже есть в списках");
        } else {
            self.after_change();
            self.toast(ctx, Lvl::Ok, format!("Добавлено доменов: {}, IP: {}", added[0], added[1]));
        }
    }
}
