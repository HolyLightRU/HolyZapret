use super::{App, Msg, Tab, notify};
use crate::core::picker::{self, Event, Outcome, Target};
use crate::core::service::ServiceState;
use crate::log::Lvl;
use crate::theme::{self, CYAN, GREEN, PINK, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const SECONDS_PER_STRATEGY: usize = 11;


#[derive(Default)]
pub struct PickerState {
    auto: Option<Option<std::path::PathBuf>>,
    mode: usize,
    priority: usize,
    pub(super) running: bool,
    cancel: Option<Arc<AtomicBool>>,
    strategies: usize,
    checks: usize,
    phase: usize,
    phase_name: String,
    done: usize,
    targets: Vec<Target>,
    results: Vec<Outcome>,
    chosen: Option<usize>,
    optimal: Option<usize>,
    pub(super) scope: usize,
    report_form: Option<(String, String, String)>,
    pub(super) sites: String,
    skip_lists: bool,
    default_targets: Vec<Target>,
    pub(super) full_report: bool,
}

impl PickerState {
    pub fn invalidate(&mut self) {
        self.optimal = None;
        self.default_targets.clear();
    }

    fn preview_targets(&self) -> Vec<Target> {
        let mut t = if self.scope == 1 { picker::custom_targets(&picker::parse_sites(&self.sites).0) } else { self.default_targets.clone() };
        if self.mode == 1 && !t.is_empty() {
            t.extend(picker::quic_targets());
        }
        t
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn snapshot(&self) -> crate::core::pickstore::LastPick {
        let mut p = crate::core::pickstore::LastPick { when: crate::log::now_stamp(), ..Default::default() };
        for o in &self.results {
            if let Some(path) = &o.path {
                let (ok, total) = o.score();
                p.scores.insert(
                    path.clone(),
                    crate::core::pickstore::Score { ok, total, frozen: o.frozen(), ms: o.avg_ms(), failed: o.error.is_some() },
                );
            }
        }
        p
    }

    pub fn set_quick(&mut self) {
        self.mode = 0;
    }

    pub fn optimal_count(&mut self, paths: Option<&crate::core::paths::Paths>, strategies: &[crate::core::strategy::Strategy]) -> usize {
        if self.optimal.is_none() {
            if let Some(p) = paths {
                self.optimal = Some(picker::optimal(p, strategies).len());
            }
        }
        self.optimal.unwrap_or(0)
    }

    pub fn progress(&self) -> (f32, String) {
        let steps = self.strategies + 1;
        let checks = self.checks.max(1);
        let frac = ((self.phase * checks + self.done) as f32 / (steps * checks) as f32).clamp(0.0, 1.0);
        (frac, format!("Шаг {} из {steps}: {}", self.phase + 1, self.phase_name))
    }

    pub fn best_pick(&self) -> Option<(Option<std::path::PathBuf>, String)> {
        if self.running {
            return None;
        }
        self.best().map(|i| (self.results[i].path.clone(), self.results[i].name.clone()))
    }

    pub fn on_event(&mut self, e: Event) -> Option<(Lvl, String)> {
        match e {
            Event::Started { strategies, checks } => {
                self.strategies = strategies;
                self.checks = checks;
                None
            }
            Event::Phase { index, name } => {
                self.phase = index;
                self.phase_name = name;
                self.done = 0;
                None
            }
            Event::Progress { done } => {
                self.done = done;
                None
            }
            Event::Done(o) => {
                self.results.push(o);
                None
            }
            Event::Finished { cancelled, restore } => {
                self.running = false;
                self.cancel = None;
                self.chosen = None;
                let mut msg = if cancelled { "Подбор отменён".to_string() } else { "Подбор завершён".to_string() };
                let mut lvl = if cancelled { Lvl::Info } else { Lvl::Ok };
                match restore {
                    Some(Ok(())) => msg.push_str(", служба снова запущена"),
                    Some(Err(e)) => {
                        msg.push_str(&format!(", но службу не удалось запустить: {e}"));
                        lvl = Lvl::Err;
                    }
                    None => {}
                }
                Some((lvl, msg))
            }
            Event::Failed(e) => {
                self.running = false;
                self.cancel = None;
                Some((Lvl::Err, e))
            }
            Event::Lists { added, open, error } => {
                let mut parts = Vec::new();
                if !added.is_empty() {
                    parts.push(format!("Добавлено в «Мои домены»: {}", added.join(", ")));
                }
                if !open.is_empty() {
                    parts.push(format!("открываются без обхода, в «Мои домены» не добавлены: {}", open.join(", ")));
                }
                match error {
                    Some(e) => Some((Lvl::Err, format!("Не удалось дополнить «Мои домены»: {e}"))),
                    None if parts.is_empty() => None,
                    None => Some((if added.is_empty() { Lvl::Info } else { Lvl::Ok }, parts.join(". "))),
                }
            }
        }
    }

    fn priority_cat(&self) -> Option<String> {
        if self.priority == 0 { None } else { picker::categories(&self.targets).get(self.priority - 1).cloned() }
    }

    fn noisy(&self) -> Vec<bool> {
        picker::noisy_checks(&self.targets, &self.results)
    }

    fn noisy_names(&self) -> Vec<String> {
        self.targets.iter().zip(self.noisy()).filter(|(_, n)| *n).map(|(t, _)| format!("{} / {}", t.category, t.name)).collect()
    }

    fn unreachable(&self) -> Vec<bool> {
        picker::unreachable_checks(&self.targets, &self.results)
    }

    fn unreachable_names(&self) -> (Vec<String>, Vec<String>) {
        let dead = self.unreachable();
        let dns = picker::dns_failures(&self.results, &dead);
        let pick = |want_dns: bool| -> Vec<String> {
            self.targets.iter().enumerate().filter(|(i, _)| dead[*i] && dns[*i] == want_dns).map(|(_, t)| picker::check_label(t)).collect()
        };
        (pick(true), pick(false))
    }

    fn mask(&self) -> Vec<bool> {
        picker::either(&self.noisy(), &self.unreachable())
    }

    fn key(&self, o: &Outcome, noisy: &[bool]) -> (bool, std::cmp::Reverse<usize>, std::cmp::Reverse<usize>, usize) {
        let pri = self.priority_cat().map(|c| o.category(&self.targets, &c).0).unwrap_or(0);
        (o.error.is_some(), std::cmp::Reverse(pri), std::cmp::Reverse(o.clean_score(noisy)), o.frozen())
    }

    fn broken(&self) -> Option<(String, Vec<String>)> {
        let best = self.best().map(|i| &self.results[i]).filter(|o| o.path.is_some() && o.error.is_none())?;
        let base = self.results.iter().find(|o| o.path.is_none())?;
        let mut domains: Vec<String> = Vec::new();
        for (i, t) in self.targets.iter().enumerate() {
            let (Some(b), Some(c)) = (base.checks.get(i), best.checks.get(i)) else { continue };
            if b.ok && !c.ok && !c.skipped {
                let d = crate::core::discover::base_domain(&picker::host_of(&t.url));
                if !domains.contains(&d) {
                    domains.push(d);
                }
            }
        }
        (!domains.is_empty()).then(|| (best.name.clone(), domains))
    }

    fn best(&self) -> Option<usize> {
        let noisy = self.mask();
        let top = self.ranked_with(&noisy).first().copied().filter(|&i| self.results[i].error.is_none());
        let base = self.results.iter().position(|o| o.path.is_none());
        match (top, base) {
            (Some(t), Some(b)) => {
                if self.key(&self.results[b], &noisy) <= self.key(&self.results[t], &noisy) { Some(b) } else { Some(t) }
            }
            (t, b) => t.or(b),
        }
    }

    fn ranked(&self) -> Vec<usize> {
        self.ranked_with(&self.mask())
    }

    fn auto_choice(&self, current: Option<&std::path::Path>) -> picker::Choice {
        picker::choose(&self.targets, &self.results, current)
    }

    fn constructor(&self, base: &std::path::Path, donor_ok: impl Fn(&std::path::Path) -> bool) -> Option<Vec<(usize, Vec<String>)>> {
        let bi = self.results.iter().position(|o| o.path.as_deref() == Some(base) && o.error.is_none())?;
        let mask = self.mask();
        let mut left: Vec<usize> = (0..self.targets.len())
            .filter(|&i| !mask.get(i).copied().unwrap_or(false))
            .filter(|&i| self.results[bi].checks.get(i).is_some_and(|c| c.counted() && !c.ok))
            .collect();
        let order = self.ranked();
        let mut used: Vec<String> = Vec::new();
        let mut out = Vec::new();
        while !left.is_empty() && out.len() < 3 {
            let best = order
                .iter()
                .enumerate()
                .filter(|(_, ri)| **ri != bi && self.results[**ri].error.is_none() && self.results[**ri].path.as_deref().is_some_and(&donor_ok))
                .map(|(rank, &ri)| (ri, rank, left.iter().filter(|&&i| self.results[ri].checks.get(i).is_some_and(|c| c.ok)).count()))
                .filter(|(_, _, n)| *n > 0)
                .max_by_key(|(_, rank, n)| (*n, std::cmp::Reverse(*rank)));
            let Some((ri, _, _)) = best else { break };
            let covered: Vec<usize> = left.iter().copied().filter(|&i| self.results[ri].checks.get(i).is_some_and(|c| c.ok)).collect();
            left.retain(|i| !covered.contains(i));
            let mut domains = Vec::new();
            for i in covered {
                let d = crate::core::discover::base_domain(&picker::host_of(&self.targets[i].url));
                if !d.is_empty() && !used.contains(&d) {
                    used.push(d.clone());
                    domains.push(d);
                }
            }
            if !domains.is_empty() {
                out.push((ri, domains));
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn ranked_with(&self, noisy: &[bool]) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.results.len()).filter(|&i| self.results[i].path.is_some()).collect();
        idx.sort_by_key(|&i| {
            let o = &self.results[i];
            (self.key(o, noisy), o.avg_ms())
        });
        idx
    }
}

impl App {
    pub(super) fn open_report(&mut self) {
        let (isp, city) = crate::core::settings::report_place();
        self.picker.report_form = Some((isp, city, String::new()));
    }

    pub(super) fn picker_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(p) = &self.paths {
            if self.picker.default_targets.is_empty() {
                self.picker.default_targets = picker::load_targets(p);
            }
            if self.picker.optimal.is_none() {
                self.picker.optimal = Some(picker::optimal(p, &self.strategies).len());
            }
        }
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Подбор приёма").size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.picker.running {
                        if ui.button("Отменить").clicked() {
                            if let Some(c) = &self.picker.cancel {
                                c.store(true, Ordering::Relaxed);
                            }
                        }
                    } else {
                        ui.add_enabled_ui(!self.busy && self.paths.is_some(), |ui| {
                            if ui.add(theme::Primary("Начать подбор".into())).clicked() {
                                self.start_pick(ctx);
                            }
                            if crate::core::TEST_BUILD
                                && ui
                                    .button("Полный подбор и отчёт")
                                    .on_hover_text("Все приёмы на всех сайтах, а в конце отчёт для разработчика сам сохранится на рабочий стол. Можно нажать и отойти от компьютера")
                                    .clicked()
                            {
                                self.start_full_report(ctx);
                            }
                        });
                    }
                });
            });
            if self.picker.running {
                self.picker_progress(ui);
            } else {
                self.picker_setup(ui);
            }
        });
        if !self.picker.results.is_empty() {
            self.picker_results(ui, ctx);
        }
    }

    fn picker_setup(&mut self, ui: &mut egui::Ui) {
        let optimal = self.picker.optimal.unwrap_or(0);
        let all = self.strategies.len();
        theme::row(ui, 36.0, |ui| {
            theme::row_label(ui, "Набор", 110.0);
            let labels = vec![format!("Быстрый · {optimal}"), format!("Все · {all}")];
            if self.picker.mode > 1 {
                self.picker.mode = 1;
            }
            theme::segmented(ui, &labels, &mut self.picker.mode);
        });
        theme::row(ui, 36.0, |ui| {
            theme::row_label(ui, "Проверять", 110.0);
            let labels = vec!["Все сайты".to_string(), "Только мои".to_string()];
            if theme::segmented(ui, &labels, &mut self.picker.scope) {
                self.picker.priority = 0;
            }
        });
        let mut bad = Vec::new();
        if self.picker.scope == 1 {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let w = 110.0 - ui.spacing().item_spacing.x;
                        theme::row_label(ui, "Сайты", w)
                    });
                });
                ui.add(
                    egui::TextEdit::multiline(&mut self.picker.sites)
                        .hint_text("Ссылки или домены через пробел или с новой строки: rutracker.org, https://site.ru/page")
                        .desired_rows(3)
                        .desired_width(ui.available_width()),
                );
            });
            bad = picker::parse_sites(&self.picker.sites).1;
            theme::row(ui, 30.0, |ui| {
                theme::row_label(ui, "", 110.0);
                let mut add = !self.picker.skip_lists;
                ui.checkbox(&mut add, "Добавить заблокированные в «Мои домены»")
                    .on_hover_text("RayOfLight обходит блокировку для доменов из списков. Добавятся только сайты, которые не открылись без обхода: обход рабочих сайтов может их сломать");
                self.picker.skip_lists = !add;
            });
        }
        let preview = self.picker.preview_targets();
        let cats = picker::categories(&preview);
        if cats.len() > 1 {
            theme::row(ui, 36.0, |ui| {
                theme::row_label(ui, "Важнее всего", 110.0);
                let mut labels = vec!["Всё сразу".to_string()];
                labels.extend(cats);
                theme::segmented(ui, &labels, &mut self.picker.priority);
            });
        } else {
            self.picker.priority = 0;
        }
        let n = if self.picker.mode == 0 { optimal } else { all };
        let minutes = ((n + 1) * SECONDS_PER_STRATEGY).div_ceil(60);
        let mut notes = Vec::new();
        if self.picker.scope == 1 && preview.is_empty() {
            notes.push("Впишите хотя бы один сайт, который не открывается.".to_string());
        } else {
            notes.push(format!(
                "RayOfLight по очереди запускается с каждым приёмом и проходит {} проверок. Займёт около {minutes} мин.",
                preview.len()
            ));
        }
        if self.picker.mode == 0 {
            notes.push("Быстрый набор сравнивает приёмы самоподстройки. Набор «Все» добавляет приёмы из старых батников ALT и R, приёмы для QUIC и 4 проверки QUIC.".to_string());
        } else {
            notes.push("Все приёмы для TCP и QUIC, плюс 4 проверки QUIC. Лучший можно сразу поставить.".to_string());
        }
        if self.picker.scope == 1 {
            notes.push("Если сайт после подбора открывается не полностью, найдите его дополнительные домены во вкладке «Списки».".into());
        }
        if self.status.as_ref().is_some_and(|s| s.service == ServiceState::Running) {
            notes.push("Служба будет остановлена на время подбора и запущена снова.".into());
        } else if self.status.as_ref().is_some_and(|s| s.engine_running) {
            notes.push("Запущенный обход будет остановлен на время подбора.".into());
        }
        notes.push("Подбор сначала проверяет сайты без обхода, чтобы понять, что заблокировано.".into());
        if !picker::tls13_supported() {
            notes.push("В curl этой версии Windows нет TLS 1.3, поэтому TLS 1.3 проверяется по ответу сайта на первое сообщение, без загрузки страницы.".into());
        }
        for e in bad.iter().take(3) {
            ui.label(RichText::new(e).color(RED).size(12.5));
        }
        for n in notes {
            ui.label(RichText::new(n).weak().size(12.5));
        }
    }

    fn picker_progress(&mut self, ui: &mut egui::Ui) {
        let st = &self.picker;
        let steps = st.strategies + 1;
        let checks = st.checks.max(1);
        let frac = ((st.phase * checks + st.done) as f32 / (steps * checks) as f32).clamp(0.0, 1.0);
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(5), Color32::from_white_alpha(18));
        if frac > 0.01 {
            let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height()));
            theme::gradient_rect(ui.painter(), fill, 5.0, PINK, CYAN);
        }
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Шаг {} из {steps}: {}", st.phase + 1, st.phase_name));
            ui.label(RichText::new(format!("проверок {}/{}", st.done, st.checks)).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0}%", frac * 100.0)).color(Color32::from_gray(170)));
            });
        });
    }

    fn picker_results(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let targets = self.picker.targets.clone();
        let cats = picker::categories(&targets);
        let ranked = self.picker.ranked();
        let base_idx = self.picker.results.iter().position(|o| o.path.is_none());
        let baseline = base_idx.map(|i| self.picker.results[i].clone());
        let best = self.picker.best();
        let mask = self.picker.mask();
        let mut install = None;
        let mut remove = false;
        let mut report = false;
        let mut exclude = None;
        let mut build = None;
        let base = self
            .status
            .as_ref()
            .and_then(|s| s.active_strategy())
            .and_then(|id| self.strategies.iter().find(|s| s.matches(id)))
            .cloned();
        let engine = self.paths.as_ref().and_then(crate::core::ray::exe);
        let plan = base.as_ref().filter(|_| !self.picker.running).and_then(|b| {
            let ok = |path: &std::path::Path| self.strategies.iter().any(|s| s.path == path && Some(&s.engine) == engine.as_ref());
            self.picker.constructor(&b.path, ok)
        });
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Результаты").size(17.0).strong());
                ui.label(RichText::new(format!("{}", ranked.len())).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!self.picker.running && !self.busy, egui::Button::new("Сохранить отчёт"))
                        .on_hover_text("Подробный отчёт для разбора приёмов: сохранится на рабочий стол")
                        .clicked()
                    {
                        report = true;
                    }
                });
            });
            if let Some(b) = &baseline {
                let (ok, total) = b.score();
                if total > 0 && ok == total {
                    egui::Frame::new()
                        .fill(YELLOW.gamma_multiply(0.12))
                        .stroke(egui::Stroke::new(1.0, YELLOW.gamma_multiply(0.45)))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(
                                    "Без обхода всё доступно: эти сайты не заблокированы в вашей сети, и подбор не покажет разницы между приёмами.",
                                )
                                .color(YELLOW),
                            );
                        });
                }
            }
            let noisy = self.picker.noisy_names();
            if !noisy.is_empty() && !self.picker.running {
                ui.label(
                    RichText::new(format!(
                        "Не учитываются при выборе лучшего, проходят у отдельных приёмов случайно: {}",
                        noisy.join(", ")
                    ))
                    .weak()
                    .size(12.5),
                );
            }
            let (no_dns, dead) = self.picker.unreachable_names();
            if !no_dns.is_empty() && !self.picker.running {
                ui.label(
                    RichText::new(format!("Адрес не находится: такого сайта нет в DNS или в адресе опечатка. Обход тут не поможет, в счёт не идут: {}", no_dns.join(", ")))
                        .weak()
                        .size(12.5),
                );
            }
            if !dead.is_empty() && !self.picker.running {
                ui.label(
                    RichText::new(format!(
                        "Недоступны и без обхода, и со всеми приёмами, похоже на блокировку по IP. Обход тут не поможет, в счёт не идут: {}",
                        dead.join(", ")
                    ))
                    .weak()
                    .size(12.5),
                );
            }
            if let (Some(b), Some(plan)) = (&base, &plan) {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(CYAN.gamma_multiply(0.10))
                    .stroke(egui::Stroke::new(1.0, CYAN.gamma_multiply(0.40)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(format!("Собрать: «{}» + правила для сайтов", b.name)).strong());
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "Установленный «{}» не открывает часть сайтов, а другие приёмы открывают. Можно оставить «{}» для всего остального, а для этих сайтов взять другой приём:",
                                    b.name, b.name
                                ))
                                .color(Color32::from_gray(190))
                                .size(12.5),
                            )
                            .wrap(),
                        );
                        for (ri, domains) in plan {
                            ui.label(RichText::new(format!("• {} — как в «{}»", domains.join(", "), self.picker.results[*ri].name)).size(12.5));
                        }
                        ui.add_space(2.0);
                        if ui.add_enabled(!self.busy, egui::Button::new("Собрать")).on_hover_text("Правила появятся на вкладке «Сайты», служба перезапустится").clicked() {
                            build = Some(plan.clone());
                        }
                    });
            }
            if let Some((name, domains)) = self.picker.broken().filter(|_| !self.picker.running) {
                theme::row(ui, 30.0, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Не обходить их"))
                            .on_hover_text("Добавит правило на вкладке «Сайты»: обход не будет трогать эти сайты")
                            .clicked()
                        {
                            exclude = Some((name.clone(), domains.clone()));
                        }
                        let w = ui.available_width();
                        ui.add_sized(
                            [w, 18.0],
                            egui::Label::new(
                                RichText::new(format!("С «{name}» перестают открываться сайты, которые работают без обхода: {}", domains.join(", ")))
                                    .color(YELLOW)
                                    .size(12.5),
                            )
                            .truncate(),
                        );
                    });
                });
            }
            let h = (ui.available_height() - 52.0).max(120.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if let (Some(b), Some(bi)) = (&baseline, base_idx) {
                    let sel = self.picker.chosen == Some(bi) || (self.picker.chosen.is_none() && best == Some(bi));
                    if result_row(ui, None, b, &targets, &cats, &mask, sel, best == Some(bi)).clicked() {
                        self.picker.chosen = Some(bi);
                    }
                }
                for (rank, &i) in ranked.iter().enumerate() {
                    let o = &self.picker.results[i];
                    let sel = self.picker.chosen == Some(i) || (self.picker.chosen.is_none() && Some(i) == best);
                    if result_row(ui, Some(rank + 1), o, &targets, &cats, &mask, sel, Some(i) == best).clicked() {
                        self.picker.chosen = Some(i);
                    }
                }
            });
            let pick = self.picker.chosen.or(best).filter(|&i| self.picker.results[i].error.is_none());
            let pick_is_base = pick.is_some() && pick == base_idx;
            let installed = self.status.as_ref().is_some_and(|s| s.service != ServiceState::NotInstalled);
            theme::row(ui, 44.0, |ui| {
                let hint = if pick_is_base {
                    "Обход не нужен: без него результат не хуже"
                } else {
                    "По умолчанию выбрана лучшая, кликните по строке, чтобы выбрать другую"
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let free = !self.busy && !self.picker.running;
                    if pick_is_base {
                        if installed && ui.add_enabled(free, egui::Button::new("Удалить службу")).clicked() {
                            remove = true;
                        }
                    } else {
                        ui.add_enabled_ui(pick.is_some() && free, |ui| {
                            let name = pick.map(|i| self.picker.results[i].name.clone()).unwrap_or_default();
                            let btn = ui.add(theme::Primary(format!("Установить «{}»", theme::ellipsize(&name, 34))));
                            if btn.on_hover_text(&name).clicked() {
                                install = pick;
                            }
                        });
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(hint).color(Color32::from_gray(170)).size(12.5)).truncate());
                    });
                });
            });
        });
        if report {
            let (isp, city) = crate::core::settings::report_place();
            self.picker.report_form = Some((isp, city, String::new()));
        }
        if let (Some(base), Some(plan)) = (base, build) {
            let donors: Vec<(crate::core::strategy::Strategy, Vec<String>)> = plan
                .into_iter()
                .filter_map(|(ri, d)| {
                    let path = self.picker.results[ri].path.clone()?;
                    self.strategies.iter().find(|s| s.path == path).cloned().map(|s| (s, d))
                })
                .collect();
            self.spawn(ctx, move |p, tx| {
                let mut list = crate::core::rules::load(p);
                let mut made = Vec::new();
                for (donor, domains) in donors {
                    match donor.ray.as_ref().map(|r| r.0.clone()).filter(|t| !t.starts_with("quic-") && t != "adaptive") {
                        Some(t) => {
                            made.push(format!("{} — {}", domains.join(", "), donor.name));
                            crate::core::rules::upsert(&mut list, crate::core::rules::Rule { technique: t, domains, origin: base.name.clone(), ..Default::default() });
                        }
                        None => notify(tx, Lvl::Err, format!("{}: у варианта «{}» нет отдельного приёма для сайта", domains.join(", "), donor.name)),
                    }
                }
                if made.is_empty() {
                    return;
                }
                if let Err(e) = crate::core::rules::save(p, &list) {
                    notify(tx, Lvl::Err, format!("Не удалось сохранить правила: {e}"));
                    return;
                }
                match crate::core::service::install(p, &base) {
                    Ok(_) => notify(tx, Lvl::Ok, format!("Собрано: «{}» + {}. Служба перезапущена", base.name, made.join("; "))),
                    Err(e) => notify(tx, Lvl::Err, format!("Правила сохранены, но службу не удалось перезапустить: {e}")),
                }
                let _ = tx.send(Msg::Shared);
            });
        }
        if let Some((origin, domains)) = exclude {
            self.add_rule(ctx, crate::core::rules::Rule { technique: crate::core::rules::NONE.into(), domains, origin, ..Default::default() });
        }
        if remove {
            self.confirm = Some(super::confirm::Confirm::RemoveService);
        }
        if let Some(i) = install {
            let path = self.picker.results[i].path.clone().and_then(|p| self.install_target(p));
            if let Some(idx) = path.and_then(|p| self.strategies.iter().position(|s| s.path == p)) {
                self.selected = Some(idx);
                self.tab = Tab::Main;
                self.install_selected(ctx);
            }
        }
    }

    pub(super) fn report_dialog(&mut self, ctx: &egui::Context) {
        let Some((isp, city, problem)) = self.picker.report_form.as_mut() else { return };
        let mut answer = None;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        let modal = egui::Modal::new(egui::Id::NULL.with("report_form")).frame(frame).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(RichText::new("Отчёт для разработчика").size(17.0).strong());
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    RichText::new("В отчёт попадут версия, настройки RayOfLight, правила, свои домены, проверка сайтов, подбор, диагностика и журнал. Провайдер и город сильно влияют на то, что работает, их можно не заполнять")
                        .color(Color32::from_gray(170))
                        .size(12.5),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            for (label, value, hint) in [("Провайдер", &mut *isp, "Ростелеком, МТС, Билайн…"), ("Город", &mut *city, "Омск")] {
                theme::row(ui, 34.0, |ui| {
                    theme::row_label(ui, label, 92.0);
                    ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(ui.available_width()));
                });
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Что не работает").color(Color32::from_gray(170)));
            ui.add(egui::TextEdit::multiline(problem).hint_text("голос Discord, игры, видео, какой-то сайт…").desired_rows(2).desired_width(f32::INFINITY));
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::Primary("Сохранить".into())).clicked() {
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
                if let Some((isp, city, problem)) = self.picker.report_form.take() {
                    crate::core::settings::set_report_place(isp.trim(), city.trim());
                    self.save_report(ctx, isp, city, problem);
                }
            }
            Some(false) => self.picker.report_form = None,
            None => {}
        }
    }

    fn start_full_report(&mut self, ctx: &egui::Context) {
        self.picker.mode = 1;
        self.picker.scope = 0;
        self.picker.priority = 0;
        self.start_pick(ctx);
        if self.picker.running {
            self.picker.full_report = true;
            self.toast(ctx, Lvl::Info, "Полный подбор запущен. Когда он закончится, отчёт сам сохранится на рабочий стол");
        }
    }

    pub(super) fn finish_full_report(&mut self, ctx: &egui::Context, completed: bool) {
        if !std::mem::take(&mut self.picker.full_report) || !completed {
            return;
        }
        let (isp, city) = crate::core::settings::report_place();
        self.save_report(ctx, isp, city, String::new());
    }

    fn save_report(&mut self, ctx: &egui::Context, isp: String, city: String, problem: String) {
        let mut order: Vec<usize> = self.picker.results.iter().position(|o| o.path.is_none()).into_iter().collect();
        order.extend(self.picker.ranked());
        let results: Vec<picker::Outcome> = order.iter().map(|&i| self.picker.results[i].clone()).collect();
        let targets = self.picker.targets.clone();
        let noisy = self.picker.noisy_names();
        let unreachable = self.picker.unreachable();
        let version = self.local_version.clone().unwrap_or_else(|| "?".into());
        let game = self.status.as_ref().map(|s| s.game.summary()).unwrap_or_default();
        let mut sections = self.report_sections();
        let log: Vec<String> = self.log.iter().rev().take(60).rev().map(|e| format!("{} [{}] {}", e.time, e.lvl.tag(), e.text)).collect();
        self.spawn(ctx, move |p, tx| {
            super::send(tx, Lvl::Info, "Собираю отчёт: запускаю диагностику…");
            let lists = crate::core::share::Share::collect(p, None, &crate::core::gamefilter::GameFilter::default());
            let names = ["Мои домены", "Мои исключения доменов", "Мои IP", "Мои исключения IP"];
            let mut mine = Vec::new();
            for (name, items) in names.iter().zip(&lists.lists) {
                if !items.is_empty() {
                    let shown: Vec<String> = items.iter().take(40).cloned().collect();
                    let more = if items.len() > 40 { format!(" и ещё {}", items.len() - 40) } else { String::new() };
                    mine.push(format!("{name} ({}): {}{more}", items.len(), shown.join(", ")));
                }
            }
            sections.insert(1, ("Свои списки".into(), mine));
            let diag = crate::core::diagnostics::run(p);
            let when = crate::log::now_stamp();
            let input = crate::core::report::Input {
                when: when.clone(),
                version,
                launcher: env!("CARGO_PKG_VERSION").to_string(),
                game,
                root: &p.root,
                targets: &targets,
                ordered: results.iter().collect(),
                noisy,
                isp,
                city,
                problem,
                unreachable,
                diag: &diag,
                sections,
                log,
            };
            let text = crate::core::report::build(&input);
            match crate::core::report::save(&text, &when) {
                Ok(path) => {
                    super::notify(tx, Lvl::Ok, format!("Отчёт сохранён на рабочий стол: {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                    crate::core::report::reveal(&path);
                }
                Err(e) => super::notify(tx, Lvl::Err, format!("Не удалось сохранить отчёт: {e}")),
            }
        });
    }

    fn report_sections(&self) -> Vec<(String, Vec<String>)> {
        let mut state = Vec::new();
        match &self.status {
            Some(s) => {
                let service = match s.service {
                    ServiceState::Running => "работает",
                    ServiceState::Stopped => "остановлена",
                    ServiceState::NotInstalled => "не установлена",
                };
                state.push(format!("Служба: {service}, вариант: {}", s.installed_strategy.as_deref().unwrap_or("нет")));
                state.push(format!("Движок запущен: {}", if s.engine_running { "да" } else { "нет" }));
                if let Some(d) = s.foreign_dir() {
                    state.push(format!("Служба из другой папки: {}", d.display()));
                }
                if !s.foreign_engines.is_empty() {
                    state.push(format!("Запущен другой обход: {}", s.foreign_engines.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")));
                }
            }
            None => state.push("Состояние службы ещё не прочитано".into()),
        }
        match self.warp.as_ref().filter(|w| w.installed) {
            Some(w) => {
                let conn = match w.conn {
                    crate::core::warp::Conn::Connected => "подключён",
                    crate::core::warp::Conn::Connecting => "подключается",
                    crate::core::warp::Conn::Disconnected => "отключён",
                    crate::core::warp::Conn::Unknown => "состояние неизвестно",
                };
                state.push(format!("WARP: {conn}, режим «{}»", w.mode.label()))
            }
            None => state.push("WARP: не установлен".into()),
        }
        state.push(format!("Telegram через WARP: {}", if crate::core::tgwarp::enabled() { "включён" } else { "выключен" }));
        state.push(format!("Telegram-прокси: {}", if crate::core::tgproxy::control::snapshot().is_some() { "работает" } else { "выключен" }));
        state.push(match crate::core::lanproxy::control::snapshot() {
            Some(s) => format!("Раздача на телефон: работает, устройств {}, соединений {}", s.clients, s.total),
            None => "Раздача на телефон: выключена".into(),
        });
        if !self.health_alert.is_empty() {
            state.push(format!("Слежение: не открываются {}", self.health_alert.join(", ")));
        }
        let rules: Vec<String> = crate::core::rules::load_or_empty(self.paths.as_ref())
            .iter()
            .map(|r| format!("{} — {}", r.domains.join(", "), crate::core::rules::label(&r.technique)))
            .collect();
        let sites: Vec<String> = self
            .sites
            .iter()
            .map(|s| if s.ok { format!("{}: открывается, {} мс, {}", s.url, s.ms, s.detail) } else { format!("{}: НЕ открывается, {}", s.url, s.detail) })
            .collect();
        let mut out = vec![("Состояние".into(), state), ("Правила для сайтов".into(), rules), ("Проверка сайтов".into(), sites)];
        if let Some(c) = &self.crash {
            out.push(("Падение лаунчера в прошлый раз".into(), c.lines().map(String::from).collect()));
        }
        if let Some(p) = &self.paths {
            out.push(("RayOfLight".into(), crate::core::ray::report_lines(p)));
        }
        let gw = self.phone.gateway_iface();
        if crate::core::settings::ray_gateway() || gw.as_ref().is_some_and(|i| i.forwarding) {
            out.push(("Компьютер как шлюз".into(), vec![crate::core::gateway::report_line(gw.as_ref())]));
        }
        let devices = crate::core::lanproxy::control::report_lines();
        if !devices.is_empty() {
            out.push(("Раздача: что запрашивают устройства".into(), devices));
        }
        out
    }

    pub(super) fn repick(&mut self, ctx: &egui::Context) {
        self.picker.mode = 0;
        self.picker.scope = 0;
        self.tab = Tab::Picker;
        if !self.picker.running && !self.busy {
            self.start_pick(ctx);
        }
    }

    pub(super) fn auto_repick(&mut self, ctx: &egui::Context) {
        if self.picker.running || self.busy {
            return;
        }
        let current = self
            .status
            .as_ref()
            .and_then(|s| s.active_strategy())
            .and_then(|id| self.strategies.iter().find(|s| s.matches(id)))
            .map(|s| s.path.clone());
        self.picker.auto = Some(current);
        self.repick(ctx);
        if !self.picker.running {
            self.picker.auto = None;
        } else {
            self.toast(ctx, Lvl::Info, "Подбираю приём: в конце лаунчер сам поставит лучший, если он лучше текущего");
        }
    }

    pub(super) fn finish_auto(&mut self, ctx: &egui::Context, completed: bool) {
        let Some(current) = self.picker.auto.take() else { return };
        if !completed {
            return;
        }
        match self.picker.auto_choice(current.as_deref()) {
            picker::Choice::NoBlocks => self.toast(ctx, Lvl::Info, "Без обхода всё открывается: похоже, провайдер сейчас ничего не блокирует. Приём оставил как есть"),
            picker::Choice::Keep(Some(name)) => self.toast(
                ctx,
                Lvl::Info,
                format!("Лучше «{name}» в быстром наборе ничего нет, оставил её. Можно запустить полный подбор: набор «Все» на вкладке «Подбор»"),
            ),
            picker::Choice::Keep(None) => self.toast(ctx, Lvl::Err, "Подбор не нашёл рабочего приёма. Попробуйте полный подбор: набор «Все» на вкладке «Подбор»"),
            picker::Choice::Switch { path, name, from, score } => {
                let target = self.install_target(path);
                if let Some(idx) = target.and_then(|p| self.strategies.iter().position(|s| s.path == p)) {
                    self.selected = Some(idx);
                    self.health_alert.clear();
                    let why = match from {
                        Some((old, was)) => format!("Ставлю «{name}»: открывает {score} сайтов против {was} у «{old}»"),
                        None => format!("Ставлю «{name}»: она лучшая в подборе"),
                    };
                    self.toast(ctx, Lvl::Ok, why);
                    self.tab = Tab::Main;
                    self.install_selected(ctx);
                }
            }
        }
    }

    pub(super) fn health_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.health_alert.is_empty() {
            return;
        }
        let mut act = 0;
        egui::Frame::new()
            .fill(YELLOW.gamma_multiply(0.12))
            .stroke(egui::Stroke::new(1.0, YELLOW.gamma_multiply(0.45)))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::row(ui, 34.0, |ui| {
                    ui.label(RichText::new(theme::icon::WARNING).font(theme::icon_font(16.0)).color(YELLOW));
                    ui.label(RichText::new(format!("Обход перестал работать: не открываются {}", self.health_alert.join(", "))).color(YELLOW).size(14.5));
                });
                ui.label(
                    RichText::new("Похоже, провайдер изменил блокировку. Подбор найдёт приём, который работает сейчас")
                        .color(Color32::from_gray(190))
                        .size(12.5),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy && !self.picker.running, theme::Primary("Подобрать и поставить".into()))
                        .on_hover_text("Около минуты: лаунчер проверит быстрый набор приёмов и сам поставит лучший, если он лучше текущего")
                        .clicked()
                    {
                        act = 4;
                    }
                    if ui.add_enabled(!self.busy && !self.picker.running, egui::Button::new("Открыть подбор")).clicked() {
                        act = 1;
                    }
                    if ui.button("Проверить ещё раз").clicked() {
                        act = 2;
                    }
                    if ui.button("Скрыть").clicked() {
                        act = 3;
                    }
                });
            });
        ui.add_space(10.0);
        match act {
            1 => self.repick(ctx),
            2 => {
                self.health_force.store(true, std::sync::atomic::Ordering::Relaxed);
                self.toast(ctx, Lvl::Info, "Проверяю YouTube и Discord…");
            }
            3 => self.health_alert.clear(),
            4 => self.auto_repick(ctx),
            _ => {}
        }
    }

    pub(super) fn warp_blocks_checks(&mut self) -> bool {
        use crate::core::warp::{Conn, Mode};
        let on = self.warp.as_ref().is_some_and(|w| w.mode == Mode::Warp && matches!(w.conn, Conn::Connected | Conn::Connecting));
        if on {
            self.confirm = Some(super::confirm::Confirm::WarpBlocksCheck);
        }
        on
    }

    fn install_target(&mut self, path: std::path::PathBuf) -> Option<std::path::PathBuf> {
        if crate::core::ray::virtual_experiment(&path).is_some() {
            self.ray = Default::default();
        }
        Some(path)
    }

    pub(super) fn start_pick(&mut self, ctx: &egui::Context) {
        if self.warp_blocks_checks() {
            return;
        }
        let Some(p) = self.paths.clone() else { return };
        let idx: Vec<usize> = if self.picker.mode == 0 { picker::optimal(&p, &self.strategies) } else { (0..self.strategies.len()).collect() };
        if idx.is_empty() {
            self.toast(ctx, Lvl::Err, "RayOfLight недоступен: нет bin\\RayOfLight.exe");
            return;
        }
        let mut idx = idx;
        if let Some(Some(cur)) = &self.picker.auto {
            if let Some(i) = self.strategies.iter().position(|s| &s.path == cur) {
                if !idx.contains(&i) {
                    idx.push(i);
                }
            }
        }
        let list: Vec<crate::core::strategy::Strategy> = idx.iter().map(|&i| self.strategies[i].clone()).collect();
        self.run_pick_list(ctx, p, list);
    }

    fn run_pick_list(&mut self, ctx: &egui::Context, p: crate::core::paths::Paths, list: Vec<crate::core::strategy::Strategy>) {
        let mut add_to: Vec<std::path::PathBuf> = Vec::new();
        let targets = if self.picker.scope == 1 {
            let urls = picker::parse_sites(&self.picker.sites).0;
            if urls.is_empty() {
                self.toast(ctx, Lvl::Info, "Впишите сайт, для которого нужно подобрать приём");
                return;
            }
            let hosts: Vec<String> = urls.iter().map(|u| picker::host_of(u)).collect();
            let missing = picker::unresolved(&hosts);
            let urls: Vec<String> = urls.into_iter().filter(|u| !missing.contains(&picker::host_of(u))).collect();
            if urls.is_empty() {
                self.toast(ctx, Lvl::Err, format!("Адрес не находится: {}. Проверьте, нет ли опечатки", missing.join(", ")));
                return;
            }
            if !missing.is_empty() {
                self.toast(ctx, Lvl::Info, format!("Адрес не находится: {}. Эти сайты не проверяю", missing.join(", ")));
            }
            if !self.picker.skip_lists {
                for s in &list {
                    let d = crate::core::strategy::lists_dir(s, &p);
                    if !add_to.contains(&d) {
                        add_to.push(d);
                    }
                }
            }
            picker::custom_targets(&urls)
        } else {
            picker::load_targets(&p)
        };
        let mut targets = targets;
        if self.picker.mode == 1 {
            targets.extend(picker::quic_targets());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let st = &mut self.picker;
        st.targets = targets.clone();
        st.results.clear();
        st.chosen = None;
        st.running = true;
        st.phase = 0;
        st.done = 0;
        st.strategies = list.len();
        st.checks = targets.len();
        st.phase_name = "Подготовка".into();
        st.cancel = Some(cancel.clone());
        self.busy = true;
        self.last_action = ctx.input(|i| i.time);
        self.note(Lvl::Info, format!("Подбор: приёмов {}, проверок на каждый {}", list.len(), targets.len()));
        let gf = crate::core::gamefilter::load(&p);
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            picker::run(p, list, targets, gf, add_to, cancel, move |e| {
                let _ = tx.send(Msg::Pick(e));
                ctx2.request_repaint();
            });
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn result_row(
    ui: &mut egui::Ui,
    rank: Option<usize>,
    o: &Outcome,
    targets: &[Target],
    cats: &[String],
    noisy: &[bool],
    selected: bool,
    best: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
    let p = ui.painter();
    if selected {
        theme::gradient_rect(p, rect, 10.0, PINK, CYAN);
        p.rect_filled(rect.shrink(1.5), CornerRadius::same(9), Color32::from_rgba_unmultiplied(24, 20, 32, 245));
        p.rect_filled(rect.shrink(1.5), CornerRadius::same(9), Color32::from_white_alpha(8));
    } else {
        let a = if rank.is_none() { 3 } else if resp.hovered() { 14 } else { 6 };
        p.rect_filled(rect, CornerRadius::same(10), Color32::from_white_alpha(a));
    }
    let y1 = rect.top() + 17.0;
    let y2 = rect.bottom() - 15.0;
    let rank_text = rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into());
    p.text(egui::pos2(rect.left() + 22.0, rect.center().y), egui::Align2::CENTER_CENTER, rank_text, egui::FontId::proportional(14.0), Color32::from_gray(130));
    let name_col = if rank.is_none() { Color32::from_gray(160) } else { Color32::WHITE };
    p.text(egui::pos2(rect.left() + 44.0, y1), egui::Align2::LEFT_CENTER, &o.name, egui::FontId::proportional(15.0), name_col);
    let (ok, total) = o.score_without(noisy);
    let meta = if o.error.is_some() { String::new() } else { format!("{ok}/{total} · {} мс", o.avg_ms()) };
    let meta = p.layout_no_wrap(meta, egui::FontId::proportional(12.5), Color32::from_gray(170));
    let limit = rect.right() - 16.0 - meta.size().x - 14.0;
    let mut x = rect.left() + 44.0;
    if let Some(e) = &o.error {
        let mut short: String = e.chars().take(70).collect();
        if e.chars().count() > 70 {
            short.push('…');
        }
        p.text(egui::pos2(x, y2), egui::Align2::LEFT_CENTER, format!("Не запустилась: {short}"), egui::FontId::proportional(12.0), RED);
    } else {
        let shown: Vec<(String, Color32)> = cats
            .iter()
            .filter_map(|c| {
                let (a, b) = o.category_without(targets, c, noisy);
                if b > 0 {
                    return Some((format!("{c} {a}/{b}"), if a == b { GREEN } else if a == 0 { RED } else { YELLOW }));
                }
                masked(targets, c, noisy).then(|| (format!("{c} —"), Color32::from_gray(120)))
            })
            .collect();
        for (i, (text, col)) in shown.iter().enumerate() {
            let g = p.layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), *col);
            let more = p.layout_no_wrap(format!("ещё {}", shown.len() - i), egui::FontId::proportional(12.0), Color32::from_gray(150));
            let last = i + 1 == shown.len();
            if x + g.size().x > limit || (!last && x + g.size().x + 14.0 + more.size().x > limit) {
                if x + more.size().x <= limit {
                    p.galley(egui::pos2(x, y2 - more.size().y / 2.0), more, Color32::from_gray(150));
                }
                break;
            }
            let w = g.size().x;
            p.galley(egui::pos2(x, y2 - g.size().y / 2.0), g, *col);
            x += w + 14.0;
        }
    }
    let bar_w = 120.0;
    let bar = egui::Rect::from_min_size(egui::pos2(rect.right() - 16.0 - bar_w, y1 - 3.0), egui::vec2(bar_w, 6.0));
    p.rect_filled(bar, CornerRadius::same(3), Color32::from_white_alpha(18));
    if total > 0 && ok > 0 {
        let fill = egui::Rect::from_min_size(bar.min, egui::vec2(bar_w * ok as f32 / total as f32, 6.0));
        theme::gradient_rect(p, fill, 3.0, PINK, CYAN);
    }
    let mx = rect.right() - 16.0 - meta.size().x;
    p.galley(egui::pos2(mx, y2 - meta.size().y / 2.0), meta, Color32::from_gray(170));
    let mut pill_right = egui::pos2(bar.left() - 12.0, y1);
    if best {
        let r = theme::paint_pill(p, pill_right, "Лучшая", GREEN);
        pill_right.x = r.left() - 8.0;
    }
    if o.frozen() > 0 {
        theme::paint_pill(p, pill_right, &format!("заморозка {}", o.frozen()), YELLOW);
    }
    resp.on_hover_ui(|ui| {
        ui.label(tip_text(o, targets, cats, noisy));
    })
}

fn masked(targets: &[Target], cat: &str, noisy: &[bool]) -> bool {
    targets.iter().enumerate().any(|(i, t)| t.category == cat && noisy.get(i).copied().unwrap_or(false))
}

fn tip_text(o: &Outcome, targets: &[Target], cats: &[String], noisy: &[bool]) -> String {
    let mut lines: Vec<String> = cats
        .iter()
        .map(|c| {
            let (a, b) = o.category_without(targets, c, noisy);
            if b > 0 {
                Some(format!("{c}: {a} из {b}"))
            } else {
                masked(targets, c, noisy).then(|| format!("{c}: не в счёт, недоступен и без обхода, и со всеми приёмами"))
            }
        })
        .flatten()
        .collect();
    let failed: Vec<String> = targets
        .iter()
        .zip(&o.checks)
        .enumerate()
        .filter(|(i, (_, c))| !c.ok && !c.skipped && !noisy.get(*i).copied().unwrap_or(false))
        .map(|(_, (t, c))| {
            let kind = match t.kind {
                picker::CheckKind::Tls12 => "TLS 1.2",
                picker::CheckKind::Tls13 => "TLS 1.3",
                picker::CheckKind::Freeze => "загрузка",
                picker::CheckKind::Quic => "QUIC",
            };
            format!("  {} ({kind}): {}", t.name, c.detail())
        })
        .collect();
    if let Some(e) = &o.error {
        lines = vec![format!("Не запустилась: {e}"), format!("Лог: {}", picker::log_path(&o.name).display())];
    } else {
        if !failed.is_empty() {
            lines.push("Не прошли:".into());
            lines.extend(failed.into_iter().take(12));
        }
        let untouched: Vec<&str> = targets
            .iter()
            .zip(&o.checks)
            .filter(|(_, c)| !c.counted())
            .map(|(t, _)| t.name.as_str())
            .collect();
        if !untouched.is_empty() {
            lines.push(format!("Не обходит (не открываются и без обхода): {}", untouched.join(", ")));
        }
        if let Some(r) = &o.ray {
            lines.push(r.clone());
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::picker::{Check, CheckKind};
    use std::path::{Path, PathBuf};

    fn outcome(name: &str, path: Option<&str>, ok: &[bool]) -> Outcome {
        let checks = ok.iter().map(|&o| Check { ok: o, ms: 100, code: if o { 200 } else { 0 }, exit: if o { 0 } else { 28 }, ..Check::failed() }).collect();
        Outcome { name: name.into(), path: path.map(PathBuf::from), error: None, checks, ray: None }
    }

    #[test]
    fn auto_choice_only_switches_when_better() {
        let mut st = PickerState::default();
        st.targets = ["a.com", "b.com", "c.com"]
            .iter()
            .map(|h| Target { name: h.to_string(), category: "Сайты".into(), url: format!("https://{h}/"), kind: CheckKind::Tls13 })
            .collect();
        st.results = vec![
            outcome("Без обхода", None, &[false, false, false]),
            outcome("general", Some("g.bat"), &[true, false, false]),
            outcome("ALT 2", Some("a2.bat"), &[true, true, true]),
        ];
        match st.auto_choice(Some(Path::new("g.bat"))) {
            picker::Choice::Switch { path, name, from, score } => {
                assert_eq!((path, name.as_str(), score), (PathBuf::from("a2.bat"), "ALT 2", 3));
                assert_eq!(from, Some(("general".to_string(), 1)));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(st.auto_choice(Some(Path::new("a2.bat"))), picker::Choice::Keep(Some("ALT 2".into())));
        st.results[2] = outcome("ALT 2", Some("a2.bat"), &[true, false, false]);
        assert!(matches!(st.auto_choice(Some(Path::new("g.bat"))), picker::Choice::Keep(Some(_))));
        st.results[0] = outcome("Без обхода", None, &[true, true, true]);
        assert_eq!(st.auto_choice(Some(Path::new("g.bat"))), picker::Choice::NoBlocks);
    }

    #[test]
    fn constructor_picks_donors() {
        let mut st = PickerState::default();
        st.targets = ["a.com", "b.com", "web.whatsapp.com", "soundcloud.com", "x.org"]
            .iter()
            .map(|h| Target { name: h.to_string(), category: "Сайты".into(), url: format!("https://{h}/"), kind: CheckKind::Tls13 })
            .collect();
        st.results = vec![
            outcome("Без обхода", None, &[false, false, false, false, false]),
            outcome("general", Some("g.bat"), &[true, true, false, false, false]),
            outcome("ALT 2", Some("a2.bat"), &[false, false, true, true, false]),
            outcome("ALT 3", Some("a3.bat"), &[true, false, true, false, false]),
            outcome("Чужая", Some("f.bat"), &[true, true, true, true, true]),
        ];
        let ours = |p: &Path| p != Path::new("f.bat");
        let plan = st.constructor(Path::new("g.bat"), ours).unwrap();
        assert_eq!(plan, vec![(2, vec!["whatsapp.com".to_string(), "soundcloud.com".to_string()])]);
        assert!(st.constructor(Path::new("f.bat"), ours).is_none());
        assert!(st.constructor(Path::new("missing.bat"), ours).is_none());
    }
}
