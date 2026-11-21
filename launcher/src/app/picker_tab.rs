use super::{App, Msg, Tab};
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
    mode: usize,
    priority: usize,
    running: bool,
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
}

impl PickerState {
    pub fn invalidate(&mut self) {
        self.optimal = None;
        self.targets.clear();
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
        }
    }

    fn priority_cat(&self) -> Option<String> {
        if self.priority == 0 { None } else { picker::categories(&self.targets).get(self.priority - 1).cloned() }
    }

    fn key(&self, o: &Outcome) -> (bool, std::cmp::Reverse<usize>, std::cmp::Reverse<usize>, usize) {
        let pri = self.priority_cat().map(|c| o.category(&self.targets, &c).0).unwrap_or(0);
        (o.error.is_some(), std::cmp::Reverse(pri), std::cmp::Reverse(o.score().0), o.frozen())
    }

    fn best(&self) -> Option<usize> {
        let top = self.ranked().first().copied().filter(|&i| self.results[i].error.is_none());
        let base = self.results.iter().position(|o| o.path.is_none());
        match (top, base) {
            (Some(t), Some(b)) => {
                if self.key(&self.results[b]) <= self.key(&self.results[t]) { Some(b) } else { Some(t) }
            }
            (t, b) => t.or(b),
        }
    }

    fn ranked(&self) -> Vec<usize> {
        let cat = self.priority_cat();
        let mut idx: Vec<usize> = (0..self.results.len()).filter(|&i| self.results[i].path.is_some()).collect();
        idx.sort_by_key(|&i| {
            let o = &self.results[i];
            let pri = cat.as_ref().map(|c| o.category(&self.targets, c).0).unwrap_or(0);
            (o.error.is_some(), std::cmp::Reverse(pri), std::cmp::Reverse(o.score().0), o.frozen(), o.avg_ms())
        });
        idx
    }
}

impl App {
    pub(super) fn picker_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(p) = &self.paths {
            if self.picker.targets.is_empty() {
                self.picker.targets = picker::load_targets(p);
            }
            if self.picker.optimal.is_none() {
                self.picker.optimal = Some(picker::optimal(p, &self.strategies).len());
            }
        }
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Подбор стратегии").size(17.0).strong());
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
            theme::segmented(ui, &labels, &mut self.picker.mode);
        });
        theme::row(ui, 36.0, |ui| {
            theme::row_label(ui, "Важнее всего", 110.0);
            let mut labels = vec!["Всё сразу".to_string()];
            labels.extend(picker::categories(&self.picker.targets));
            theme::segmented(ui, &labels, &mut self.picker.priority);
        });
        let n = if self.picker.mode == 0 { optimal } else { all };
        let minutes = ((n + 1) * SECONDS_PER_STRATEGY).div_ceil(60);
        let mut notes = vec![format!(
            "Каждая стратегия запускается по очереди и проходит {} проверок. Займёт около {minutes} мин.",
            self.picker.targets.len()
        )];
        if self.status.as_ref().is_some_and(|s| s.service == ServiceState::Running) {
            notes.push("Служба будет остановлена на время подбора и запущена снова.".into());
        } else if self.status.as_ref().is_some_and(|s| s.engine_running) {
            notes.push("Запущенный вручную HolyZapret будет остановлен.".into());
        }
        notes.push("Подбор сначала проверяет сайты без обхода, чтобы понять, что заблокировано.".into());
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
        let mut install = None;
        let mut remove = false;
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Результаты").size(17.0).strong());
                ui.label(RichText::new(format!("{}", ranked.len())).weak());
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
                                    "Без обхода всё доступно: блокировок в этой сети нет, и подбор не покажет разницы между стратегиями.",
                                )
                                .color(YELLOW),
                            );
                        });
                }
            }
            let h = (ui.available_height() - 52.0).max(120.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if let (Some(b), Some(bi)) = (&baseline, base_idx) {
                    let sel = self.picker.chosen == Some(bi) || (self.picker.chosen.is_none() && best == Some(bi));
                    if result_row(ui, None, b, &targets, &cats, sel, best == Some(bi)).clicked() {
                        self.picker.chosen = Some(bi);
                    }
                }
                for (rank, &i) in ranked.iter().enumerate() {
                    let o = &self.picker.results[i];
                    let sel = self.picker.chosen == Some(i) || (self.picker.chosen.is_none() && Some(i) == best);
                    if result_row(ui, Some(rank + 1), o, &targets, &cats, sel, Some(i) == best).clicked() {
                        self.picker.chosen = Some(i);
                    }
                }
            });
            let pick = self.picker.chosen.or(best).filter(|&i| self.picker.results[i].error.is_none());
            let pick_is_base = pick.is_some() && pick == base_idx;
            let installed = self.status.as_ref().is_some_and(|s| s.service != ServiceState::NotInstalled);
            theme::row(ui, 44.0, |ui| {
                let hint = if pick_is_base {
                    "Обход не нужен: без стратегии результат не хуже"
                } else {
                    "По умолчанию выбрана лучшая, кликните по строке, чтобы выбрать другую"
                };
                ui.label(RichText::new(hint).color(Color32::from_gray(170)).size(12.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let free = !self.busy && !self.picker.running;
                    if pick_is_base {
                        if installed && ui.add_enabled(free, egui::Button::new("Удалить службу")).clicked() {
                            remove = true;
                        }
                    } else {
                        ui.add_enabled_ui(pick.is_some() && free, |ui| {
                            let name = pick.map(|i| self.picker.results[i].name.clone()).unwrap_or_default();
                            if ui.add(theme::Primary(format!("Установить «{name}»"))).clicked() {
                                install = pick;
                            }
                        });
                    }
                });
            });
        });
        if remove {
            self.confirm = Some(super::confirm::Confirm::RemoveService);
        }
        if let Some(i) = install {
            let path = self.picker.results[i].path.clone();
            if let Some(idx) = path.and_then(|p| self.strategies.iter().position(|s| s.path == p)) {
                self.selected = Some(idx);
                self.tab = Tab::Main;
                self.install_selected(ctx);
            }
        }
    }

    pub(super) fn start_pick(&mut self, ctx: &egui::Context) {
        let Some(p) = self.paths.clone() else { return };
        let idx: Vec<usize> = if self.picker.mode == 0 {
            picker::optimal(&p, &self.strategies)
        } else {
            (0..self.strategies.len()).collect()
        };
        if idx.is_empty() {
            self.toast(ctx, Lvl::Info, "В utils/optimal_strategies.txt нет стратегий, выберите набор «Все»");
            return;
        }
        let list: Vec<_> = idx.iter().map(|&i| self.strategies[i].clone()).collect();
        let targets = picker::load_targets(&p);
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
        self.note(Lvl::Info, format!("Подбор: {} стратегий, {} проверок на каждую", list.len(), targets.len()));
        let gf = crate::core::gamefilter::load(&p);
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            picker::run(p, list, targets, gf, cancel, move |e| {
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
    let (ok, total) = o.score();
    let mut x = rect.left() + 44.0;
    if let Some(e) = &o.error {
        let mut short: String = e.chars().take(70).collect();
        if e.chars().count() > 70 {
            short.push('…');
        }
        p.text(egui::pos2(x, y2), egui::Align2::LEFT_CENTER, format!("Не запустилась: {short}"), egui::FontId::proportional(12.0), RED);
    } else {
        for c in cats {
            let (a, b) = o.category(targets, c);
            if b == 0 {
                continue;
            }
            let col = if a == b { GREEN } else if a == 0 { RED } else { YELLOW };
            let g = p.layout_no_wrap(format!("{c} {a}/{b}"), egui::FontId::proportional(12.0), col);
            let w = g.size().x;
            p.galley(egui::pos2(x, y2 - g.size().y / 2.0), g, col);
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
    let mut right = egui::pos2(rect.right() - 16.0, y2);
    let meta = if o.error.is_some() { String::new() } else { format!("{ok}/{total} · {} мс", o.avg_ms()) };
    let g = p.layout_no_wrap(meta, egui::FontId::proportional(12.5), Color32::from_gray(170));
    right.x -= g.size().x;
    p.galley(egui::pos2(right.x, y2 - g.size().y / 2.0), g, Color32::from_gray(170));
    let mut pill_right = egui::pos2(bar.left() - 12.0, y1);
    if best {
        let r = theme::paint_pill(p, pill_right, "Лучшая", GREEN);
        pill_right.x = r.left() - 8.0;
    }
    if o.frozen() > 0 {
        theme::paint_pill(p, pill_right, &format!("заморозка {}", o.frozen()), YELLOW);
    }
    resp.on_hover_ui(|ui| {
        ui.label(tip_text(o, targets, cats));
    })
}

fn tip_text(o: &Outcome, targets: &[Target], cats: &[String]) -> String {
    let mut lines: Vec<String> = cats
        .iter()
        .map(|c| {
            let (a, b) = o.category(targets, c);
            format!("{c}: {a} из {b}")
        })
        .collect();
    let failed: Vec<String> = targets
        .iter()
        .zip(&o.checks)
        .filter(|(_, c)| !c.ok)
        .map(|(t, c)| {
            let kind = match t.kind {
                picker::CheckKind::Tls12 => "TLS 1.2",
                picker::CheckKind::Tls13 => "TLS 1.3",
                picker::CheckKind::Freeze => {
                    if c.frozen {
                        "заморозка"
                    } else {
                        "загрузка"
                    }
                }
            };
            format!("  {} ({kind})", t.name)
        })
        .collect();
    if let Some(e) = &o.error {
        lines = vec![format!("Не запустилась: {e}"), format!("Лог: {}", picker::log_path(&o.name).display())];
    } else if !failed.is_empty() {
        lines.push("Не прошли:".into());
        lines.extend(failed.into_iter().take(12));
    }
    lines.join("\n")
}
