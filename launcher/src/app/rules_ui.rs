use super::{App, Msg};
use crate::core::rules::{self, Advice, Rule, RuleState, Trial, Verdict};
use crate::core::strategy::Strategy;
use crate::core::{checks, gamefilter};
use crate::log::Lvl;
use crate::theme::{self, CYAN, GREEN, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct RulesState {
    pub list: Vec<Rule>,
    loaded: bool,
    input: String,
    stage: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
    verdict: Option<Verdict>,
    expanded: bool,
    states: Vec<(Rule, RuleState)>,
    batch: Vec<(Verdict, bool)>,
}

fn batch_rule(v: &Verdict) -> Option<Rule> {
    match &v.advice {
        Advice::Use(r) => Some(r.clone()),
        Advice::Exclude => Some(Rule { technique: rules::NONE.into(), domains: v.domains.clone(), origin: v.origin.clone(), ..Default::default() }),
        _ => None,
    }
}

fn batch_line(v: &Verdict) -> (String, Color32) {
    let host = super::sites_tab::host_of(&v.url);
    match &v.advice {
        Advice::Fine => (format!("{host}: открывается и без обхода, правило не нужно"), Color32::from_gray(170)),
        Advice::Works(s) => (format!("{host}: открывается с «{s}», правило не нужно"), Color32::from_gray(170)),
        Advice::Exclude => (format!("{host}: обход ломает сайт, не обходить"), YELLOW),
        Advice::Use(r) => match r.technique.strip_prefix(rules::BORROW) {
            Some(name) => (format!("{host}: поможет приём из «{name}»"), GREEN),
            None => (format!("{host}: поможет приём «{}»", rules::label(&r.technique)), GREEN),
        },
        Advice::Nothing if host.contains("telegram") => (
            format!("{host}: ничего не помогло, похоже на блокировку по IP. Включите «Telegram через WARP» на вкладке Telegram"),
            RED,
        ),
        Advice::Nothing => (format!("{host}: ничего не помогло, похоже на блокировку по IP"), RED),
    }
}

impl RulesState {
    pub fn running(&self) -> bool {
        self.stage.is_some()
    }
}

fn plural(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (1, x) if x != 11 => "правило",
        (2..=4, x) if !(12..=14).contains(&x) => "правила",
        _ => "правил",
    }
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

fn trial_line(t: &Trial) -> (String, Color32) {
    let state = if t.passed() {
        format!("открывается, {} мс", t.ms)
    } else if t.ok > 0 {
        format!("частично, {} из {}", t.ok, t.total)
    } else {
        "не открывается".to_string()
    };
    let col = if t.passed() { GREEN } else if t.ok > 0 { YELLOW } else { RED };
    (format!("{}: {state}", t.label), col)
}

enum Act {
    Check,
    Cancel,
    Apply(Rule),
    Picker,
    Remove(usize),
    Dismiss,
    Recheck,
    CheckSite(String),
    ApplyBatch,
    DismissBatch,
}

impl App {
    pub(super) fn rules_strategy(&self) -> Option<Strategy> {
        let installed = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from));
        match installed {
            Some(id) => self.strategies.iter().find(|s| s.matches(&id)).cloned(),
            None => self.selected.and_then(|i| self.strategies.get(i)).or_else(|| self.strategies.iter().find(|s| s.id == crate::core::ray::DEFAULT_ID)).cloned(),
        }
    }

    pub(super) fn rules_summary(&mut self) -> Option<(String, String)> {
        if !self.rules.loaded {
            self.reload_rules();
        }
        let n = self.rules.list.len();
        if n == 0 {
            return None;
        }
        let hover = self.rules.list.iter().map(|r| format!("{} — {}", r.domains.join(", "), rules::label(&r.technique))).collect::<Vec<_>>().join("
");
        Some((format!("+ {n} {} для сайтов", plural(n)), hover))
    }

    pub(super) fn reload_rules(&mut self) {
        self.rules.list = self.paths.as_ref().map(rules::load).unwrap_or_default();
        self.rules.loaded = true;
    }

    pub(super) fn add_rule(&mut self, ctx: &egui::Context, rule: Rule) {
        let Some(p) = self.paths.clone() else { return };
        let mut list = rules::load(&p);
        let text = format!("{}: {}", rules::label(&rule.technique), rule.domains.join(", "));
        rules::upsert(&mut list, rule);
        self.store_rules(ctx, &p, list, format!("Правило добавлено. {text}"));
    }

    fn store_rules(&mut self, ctx: &egui::Context, p: &crate::core::paths::Paths, list: Vec<Rule>, msg: String) {
        if let Err(e) = rules::save(p, &list) {
            self.toast(ctx, Lvl::Err, format!("Не удалось сохранить правила: {e}"));
            return;
        }
        self.rules.list = list;
        self.apply_rules(ctx, msg);
    }

    fn apply_rules(&mut self, ctx: &egui::Context, msg: String) {
        let installed = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from));
        let strategy = installed.and_then(|id| self.strategies.iter().find(|s| s.matches(&id)).cloned());
        match strategy {
            Some(s) => self.spawn(ctx, move |p, tx| {
                match crate::core::service::install(p, &s) {
                    Ok(_) => super::notify(tx, Lvl::Ok, format!("{msg}. Служба перезапущена с правилами")),
                    Err(e) => super::notify(tx, Lvl::Err, format!("{msg}, но службу не удалось перезапустить: {e}")),
                }
            }),
            None => self.toast(ctx, Lvl::Ok, format!("{msg}. Применится при установке службы")),
        }
    }

    pub(super) fn start_recheck(&mut self, ctx: &egui::Context) {
        if self.warp_blocks_checks(super::AfterWarp::RulesRecheck) {
            return;
        }
        if self.picker.running {
            self.toast(ctx, Lvl::Info, "Сначала остановите подбор");
            return;
        }
        let Some(current) = self.rules_strategy() else {
            self.toast(ctx, Lvl::Info, "RayOfLight недоступен: нет bin\\RayOfLight.exe");
            return;
        };
        let Some(p) = self.paths.clone() else { return };
        let list = rules::load(&p);
        if list.is_empty() {
            return;
        }
        let gf = gamefilter::load(&p);
        let cancel = Arc::new(AtomicBool::new(false));
        self.rules.cancel = Some(cancel.clone());
        self.rules.stage = Some("Подготовка".into());
        self.rules.states.clear();
        self.busy = true;
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            rules::recheck(p, list, current, gf, &cancel, |e| {
                let _ = tx.send(Msg::Rules(e));
                ctx2.request_repaint();
            });
        });
    }

    pub(super) fn check_site_rule(&mut self, ctx: &egui::Context, url: &str) {
        self.rules.input = url.to_string();
        self.start_rule_check(ctx);
    }

    pub(super) fn start_batch(&mut self, ctx: &egui::Context, urls: Vec<String>) {
        if self.busy || self.rules.running() || urls.is_empty() || self.warp_blocks_checks(super::AfterWarp::Batch(urls.clone())) {
            return;
        }
        if self.picker.running {
            self.toast(ctx, Lvl::Info, "Сначала остановите подбор");
            return;
        }
        let Some(p) = self.paths.clone() else { return };
        let current = self.rules_strategy();
        let gf = gamefilter::load(&p);
        let cancel = Arc::new(AtomicBool::new(false));
        self.rules.cancel = Some(cancel.clone());
        self.rules.stage = Some("Подготовка".into());
        self.rules.verdict = None;
        self.rules.batch.clear();
        self.busy = true;
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            rules::find_many(p, urls, current, gf, &cancel, |e| {
                let _ = tx.send(Msg::Rules(e));
                ctx2.request_repaint();
            });
        });
    }

    pub(super) fn start_rule_check(&mut self, ctx: &egui::Context) {
        if self.busy || self.rules.running() || self.warp_blocks_checks(super::AfterWarp::Rules) {
            return;
        }
        if self.picker.running {
            self.toast(ctx, Lvl::Info, "Сначала остановите подбор");
            return;
        }
        let url = match checks::normalize_url(&self.rules.input) {
            Ok(u) => u,
            Err(e) => {
                self.toast(ctx, Lvl::Err, e);
                return;
            }
        };
        let Some(p) = self.paths.clone() else { return };
        let current = self.rules_strategy();
        let gf = gamefilter::load(&p);
        let cancel = Arc::new(AtomicBool::new(false));
        self.rules.cancel = Some(cancel.clone());
        self.rules.stage = Some("Подготовка".into());
        self.rules.verdict = None;
        self.busy = true;
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            rules::find(p, url, current, gf, &cancel, |e| {
                let _ = tx.send(Msg::Rules(e));
                ctx2.request_repaint();
            });
        });
    }

    pub(super) fn on_rules_event(&mut self, ctx: &egui::Context, e: rules::Event) {
        match e {
            rules::Event::Stage(s) => self.rules.stage = Some(s),
            rules::Event::Done(v) => {
                self.rules.stage = None;
                self.rules.cancel = None;
                self.busy = false;
                self.rules.verdict = Some(v);
                self.refresh(ctx);
            }
            rules::Event::Batch(list) => {
                self.rules.stage = None;
                self.rules.cancel = None;
                self.busy = false;
                let found = list.iter().filter(|v| batch_rule(v).is_some()).count();
                self.rules.batch = list.into_iter().map(|v| (v.clone(), batch_rule(&v).is_some())).collect();
                self.toast(ctx, if found > 0 { Lvl::Ok } else { Lvl::Info }, format!("Подбор для сайтов закончен, правил найдено: {found}"));
                self.refresh(ctx);
            }
            rules::Event::Rechecked(list, states) => {
                self.rules.stage = None;
                self.rules.cancel = None;
                self.busy = false;
                self.rules.expanded = true;
                let broken = states.iter().filter(|s| **s == RuleState::Broken).count();
                let spare = states.iter().filter(|s| **s == RuleState::NotNeeded).count();
                self.rules.states = list.into_iter().zip(states).collect();
                let msg = match (broken, spare) {
                    (0, 0) => "Все правила работают".to_string(),
                    _ => format!("Правила перепроверены: не помогают {broken}, больше не нужны {spare}"),
                };
                self.toast(ctx, if broken > 0 { Lvl::Err } else { Lvl::Ok }, msg);
                self.refresh(ctx);
            }
            rules::Event::Failed(e) => {
                self.rules.stage = None;
                self.rules.cancel = None;
                self.busy = false;
                self.toast(ctx, Lvl::Err, e);
                self.refresh(ctx);
            }
        }
    }

    fn verdict_ui(&self, ui: &mut egui::Ui, v: &Verdict) -> Option<Act> {
        let mut act = None;
        egui::Frame::new()
            .fill(Color32::from_white_alpha(6))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::row(ui, 30.0, |ui| {
                    ui.label(RichText::new(crate::core::picker::host_of(&v.url)).size(14.5).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(20));
                        }
                        let col = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(185) };
                        ui.painter().text(rect.center() + egui::vec2(0.0, 3.0), egui::Align2::CENTER_CENTER, theme::icon::EJECT, theme::icon_font(12.0), col);
                        if resp.on_hover_text("Скрыть результат").clicked() {
                            act = Some(Act::Dismiss);
                        }
                    });
                });
                for t in std::iter::once(&v.baseline).chain(v.strategy.iter()) {
                    let (text, col) = trial_line(t);
                    ui.label(RichText::new(text).color(col).size(12.5));
                }
                let (advice, rule) = match &v.advice {
                    Advice::Fine => ("Сайт открывается, правило не нужно. Если не работает приложение, а не сайт, проверка этого не видит".to_string(), None),
                    Advice::Works(s) => (format!("Сайт открывается с «{s}», правило не нужно"), None),
                    Advice::Exclude => (
                        format!("Обход ломает сайт, который работает и без него. Не обходить: {}", v.domains.join(", ")),
                        Some(Rule { technique: rules::NONE.into(), domains: v.domains.clone(), origin: v.origin.clone(), ..Default::default() }),
                    ),
                    Advice::Use(rule) => {
                        let best = v.techniques.iter().find(|t| t.rule.as_ref() == Some(rule));
                        let ms = best.map(|t| t.ms).unwrap_or(0);
                        let what = if rule.borrowed() {
                            format!("Помогает приём: {}", rules::label(&rule.technique).to_lowercase())
                        } else {
                            format!("Помогает приём «{}»", rules::label(&rule.technique))
                        };
                        (format!("{what}, {ms} мс. Правило для: {}", v.domains.join(", ")), Some(rule.clone()))
                    }
                    Advice::Nothing => (
                        "Ни один приём не помог. Похоже, сайт заблокирован по IP: такой обход тут бессилен".to_string(),
                        None,
                    ),
                };
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(advice).size(13.0)).wrap());
                if !v.techniques.is_empty() {
                    let passed = v.techniques.iter().filter(|t| t.passed()).count();
                    ui.label(muted(format!("Проверено приёмов: {}, сработало: {passed}", v.techniques.len())));
                }
                let enabled = !self.busy;
                if let Some(r) = rule {
                    ui.add_space(6.0);
                    let label = if r.technique == rules::NONE { "Не обходить" } else { "Применить правило" };
                    if ui.add_enabled(enabled, theme::Primary(label.into())).clicked() {
                        act = Some(Act::Apply(r));
                    }
                }
                if matches!(v.advice, Advice::Nothing) {
                    ui.add_space(6.0);
                    if ui.add_enabled(enabled, crate::theme::GlassButton::new("Открыть подбор для сайта")).clicked() {
                        act = Some(Act::Picker);
                    }
                }
            });
        act
    }

    fn rules_input(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if self.rules.running() {
                if crate::theme::button(ui, "Отменить").clicked() {
                    *act = Some(Act::Cancel);
                }
            } else if ui.add_enabled(!self.busy, theme::Primary("Проверить".into())).clicked() {
                *act = Some(Act::Check);
            }
            let w = ui.available_width();
            let r = ui.add_enabled(
                !self.rules.running(),
                egui::TextEdit::singleline(&mut self.rules.input).hint_text("Сайт не работает? Вставьте адрес").desired_width(w),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.busy {
                *act = Some(Act::Check);
            }
        });
    }

    pub(super) fn rules_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.rules.loaded {
            self.reload_rules();
        }
        let mut act = None;
        theme::card(ui, |ui| {
            let wide = ui.available_width() >= 640.0;
            theme::row(ui, 38.0, |ui| {
                ui.label(RichText::new("Правила для сайтов").font(crate::theme::semibold(17.0)).strong()).on_hover_text(
                    "Если сайт не работает, лаунчер проверит его без обхода и с приёмами RayOfLight по очереди и подскажет: не обходить его или какой приём ему подходит. Самоподстройка начнёт с этого приёма и сменит его, если сайт перестанет отвечать",
                );
                if !self.rules.list.is_empty() {
                    let n = self.rules.list.len();
                    let arrow = if self.rules.expanded { theme::icon::CHEVRON_UP } else { theme::icon::CHEVRON_DOWN };
                    let button = egui::Button::new(RichText::new(format!("{n} {}", plural(n))).color(CYAN).size(13.0))
                        .right_text(RichText::new(arrow).font(theme::icon_font(10.0)).color(CYAN));
                    let tip = if self.rules.expanded { "Свернуть список" } else { "Показать правила" };
                    if ui.add(button).on_hover_text(tip).clicked() {
                        self.rules.expanded = !self.rules.expanded;
                    }
                }
                if wide {
                    ui.add_space(8.0);
                    self.rules_input(ui, &mut act);
                }
            });
            if !wide {
                theme::row(ui, 38.0, |ui| self.rules_input(ui, &mut act));
            }
            if let Some(stage) = &self.rules.stage {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(muted(format!("{stage}…")));
                });
            }
            if let Some(v) = self.rules.verdict.clone() {
                ui.add_space(6.0);
                if let Some(a) = self.verdict_ui(ui, &v) {
                    act = Some(a);
                }
            }
            if !self.rules.batch.is_empty() {
                ui.add_space(6.0);
                let busy = self.busy;
                egui::Frame::new()
                    .fill(Color32::from_white_alpha(6))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Подбор для недоступных сайтов").strong());
                        for (v, on) in self.rules.batch.iter_mut() {
                            let (text, col) = batch_line(v);
                            if batch_rule(v).is_some() {
                                ui.checkbox(on, RichText::new(text).color(col).size(12.5));
                            } else {
                                ui.add(egui::Label::new(RichText::new(text).color(col).size(12.5)).wrap());
                            }
                        }
                        let picked = self.rules.batch.iter().filter(|(v, on)| *on && batch_rule(v).is_some()).count();
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.add_enabled(picked > 0 && !busy, crate::theme::GlassButton::new(format!("Применить отмеченные · {picked}"))).clicked() {
                                act = Some(Act::ApplyBatch);
                            }
                            if crate::theme::button(ui, "Скрыть").clicked() {
                                act = Some(Act::DismissBatch);
                            }
                        });
                    });
            }
            if self.rules.expanded && !self.rules.list.is_empty() {
                ui.add_space(6.0);
                theme::row(ui, 30.0, |ui| {
                    ui.label(muted("Проверить с RayOfLight, какие правила помогают, а какие больше не нужны"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can = !self.busy && !self.rules.running();
                        if ui.add_enabled(can, crate::theme::GlassButton::new("Перепроверить все")).clicked() {
                            act = Some(Act::Recheck);
                        }
                    });
                });
                egui::ScrollArea::vertical().id_salt("rules_list").max_height(132.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, r) in self.rules.list.iter().enumerate() {
                        theme::row(ui, 32.0, |ui| {
                            let col = if r.technique == rules::NONE { YELLOW } else { CYAN };
                            theme::chip(ui, &rules::label(&r.technique), col);
                            let state = self.rules.states.iter().find(|(x, _)| x == r).map(|(_, s)| *s);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if theme::icon_button(ui, theme::icon::DELETE, "Удалить правило").clicked() {
                                    act = Some(Act::Remove(i));
                                }
                                match state {
                                    Some(RuleState::Works) => {
                                        ui.label(RichText::new("работает").color(GREEN).size(12.5));
                                    }
                                    Some(RuleState::NotNeeded) => {
                                        if ui.small_button("Убрать").on_hover_text("Сайт открывается и без этого правила").clicked() {
                                            act = Some(Act::Remove(i));
                                        }
                                        ui.label(RichText::new("больше не нужно").color(YELLOW).size(12.5));
                                    }
                                    Some(RuleState::Broken) => {
                                        if ui.small_button("Подобрать").on_hover_text("Проверить сайт заново и подобрать другой приём").clicked() {
                                            act = Some(Act::CheckSite(r.domains.first().cloned().unwrap_or_default()));
                                        }
                                        ui.label(RichText::new("не помогает").color(RED).size(12.5));
                                    }
                                    None => {}
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add_space(4.0);
                                    ui.add(egui::Label::new(RichText::new(r.domains.join(", ")).size(13.5)).truncate().show_tooltip_when_elided(false))
                                        .on_hover_text(r.domains.join("
"));
                                });
                            });
                        });
                    }
                });
            }
        });
        ui.add_space(10.0);
        match act {
            Some(Act::Check) => self.start_rule_check(ctx),
            Some(Act::Cancel) => {
                if let Some(c) = &self.rules.cancel {
                    c.store(true, Ordering::Relaxed);
                }
            }
            Some(Act::Apply(r)) => {
                self.rules.verdict = None;
                self.add_rule(ctx, r);
            }
            Some(Act::Picker) => {
                if let Some(v) = self.rules.verdict.take() {
                    self.picker.scope = 1;
                    self.picker.sites = v.url;
                    self.tab = super::Tab::Sites;
                }
            }
            Some(Act::Remove(i)) => {
                if let Some(p) = self.paths.clone() {
                    let mut list = self.rules.list.clone();
                    if i < list.len() {
                        let r = list.remove(i);
                        self.store_rules(ctx, &p, list, format!("Правило удалено: {}", r.domains.join(", ")));
                    }
                }
            }
            Some(Act::Dismiss) => self.rules.verdict = None,
            Some(Act::Recheck) => self.start_recheck(ctx),
            Some(Act::CheckSite(site)) => {
                self.rules.input = format!("https://{site}/");
                self.start_rule_check(ctx);
            }
            Some(Act::ApplyBatch) => {
                let picked: Vec<Rule> = self.rules.batch.iter().filter(|(_, on)| *on).filter_map(|(v, _)| batch_rule(v)).collect();
                self.rules.batch.clear();
                if let Some(p) = self.paths.clone() {
                    let mut list = rules::load(&p);
                    let hosts: Vec<String> = picked.iter().map(|r| r.domains.first().cloned().unwrap_or_default()).collect();
                    for r in picked {
                        rules::upsert(&mut list, r);
                    }
                    self.store_rules(ctx, &p, list, format!("Правила добавлены: {}", hosts.join(", ")));
                }
            }
            Some(Act::DismissBatch) => self.rules.batch.clear(),
            None => {}
        }
    }
}
