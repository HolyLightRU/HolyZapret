use super::App;
use crate::core::paths::Paths;
use crate::core::{netspeed, ray, settings};
use crate::theme::{self, CYAN, GREEN, ORANGE, PINK, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2};
use std::time::{Duration, Instant};

const WINDOW: usize = 10;

#[derive(Default)]
pub struct RayHome {
    at: Option<Instant>,
    live: Option<ray::Live>,
    fresh: bool,
    net: Option<(Instant, u64, u64)>,
    rate: Option<(f64, f64)>,
    checked: Option<(String, bool)>,
    next: std::collections::HashMap<String, String>,
}

impl RayHome {
    pub fn warp(&self) -> Option<&ray::WarpLive> {
        self.live.as_ref().filter(|_| self.fresh).and_then(|l| l.warp.as_ref())
    }

    pub fn totals(&self) -> Option<(u64, u64, u64, u64)> {
        let live = self.live.as_ref().filter(|_| self.fresh)?;
        let rows = &live.history;
        Some(sum(&rows[rows.len().saturating_sub(WINDOW)..]))
    }

    pub fn refresh(&mut self, paths: Option<&Paths>) {
        if self.at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        let now = Instant::now();
        self.at = Some(now);
        self.next = paths.map(ray::next_techniques).unwrap_or_default();
        match paths.and_then(ray::read_live) {
            Some((l, age)) => {
                self.fresh = l.running && age.as_secs() < 10;
                self.live = Some(l);
            }
            None => {
                self.fresh = false;
                self.live = None;
            }
        }
        if let Some((i, o)) = netspeed::octets() {
            if let Some((t, pi, po)) = self.net {
                let s = now.duration_since(t).as_secs_f64();
                if s > 0.5 {
                    self.rate = Some((i.saturating_sub(pi) as f64 / s, o.saturating_sub(po) as f64 / s));
                }
            }
            self.net = Some((now, i, o));
        }
    }
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(175)).size(13.0)
}

fn tinted(c: Color32) -> bool {
    c.r() != c.g() || c.g() != c.b()
}

fn dot(ui: &mut egui::Ui, color: Color32, size: f32) {
    dot_at(ui, color, size, 0.0);
}

fn dot_at(ui: &mut egui::Ui, color: Color32, size: f32, dy: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().circle_filled(r.center() + Vec2::new(0.0, dy), size / 2.0, color);
}

pub(super) fn tile_sized(ui: &mut egui::Ui, label: &str, value: &str, sub: &str, color: Color32, size: f32) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(9))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(14, if size > 24.0 { 11 } else { 9 }))
        .show(ui, |ui| stat(ui, label, value, sub, color, size));
}

fn stat(ui: &mut egui::Ui, label: &str, value: &str, sub: &str, color: Color32, size: f32) {
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(muted(label));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        if tinted(color) {
            dot_at(ui, color, 8.0, size * 0.17);
        }
        ui.label(RichText::new(value).font(crate::theme::semibold(size)).strong().color(color));
    });
    if !sub.is_empty() {
        ui.label(muted(sub));
    }
}

fn empty(ui: &mut egui::Ui, glyph: &str, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(glyph).font(theme::icon_font(15.0)).color(Color32::from_gray(150)));
        ui.add(egui::Label::new(muted(text)).wrap());
    });
}

fn hover_bg(ui: &mut egui::Ui, height: f32) {
    let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height)).expand2(Vec2::new(6.0, 0.0));
    let on = ui.ctx().input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
    let k = ui.ctx().animate_bool_responsive(ui.scope_id().with(("row_hover", rect.min.y as i32)), on);
    if k > 0.0 {
        ui.painter().rect_filled(rect, CornerRadius::same(10), Color32::from_white_alpha((9.0 * k) as u8));
    }
}

fn pill_button(text: &str) -> crate::theme::GlassButton {
    crate::theme::GlassButton::new(RichText::new(text).size(12.5)).corner_radius(CornerRadius::same(14)).min_size(Vec2::new(0.0, 26.0))
}

type Row = (u64, u64, u64, u64);

fn sum(rows: &[Row]) -> Row {
    rows.iter().fold((0, 0, 0, 0), |a, r| (a.0 + r.0, a.1 + r.1, a.2 + r.2, a.3 + r.3))
}

fn app_row(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, name: &str, right: impl FnOnce(&mut egui::Ui)) {
    hover_bg(ui, 32.0);
    theme::row(ui, 32.0, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                super::ray_ui::icon_or_dot(ui, tex, Color32::from_gray(140));
                ui.add(egui::Label::new(RichText::new(name).size(13.5)).truncate());
            });
        });
    });
}

fn site_row(ui: &mut egui::Ui, height: f32, name: &str, right: impl FnOnce(&mut egui::Ui)) {
    hover_bg(ui, height);
    theme::row(ui, height, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(name).size(13.5)).truncate());
            });
        });
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Site {
    Tuned(String),
    Tuning(usize),
    Failing,
    Frozen,
}

pub fn rotation_for(experiment: &str) -> usize {
    match rayoflight::flows::Mode::parse(experiment) {
        Some(rayoflight::flows::Mode::Adaptive(list)) => list.len(),
        _ => 1,
    }
}

fn tcp(i: &ray::LiveItem) -> bool {
    i.handled && i.technique.as_deref().is_some_and(|t| !t.starts_with("quic-"))
}

pub fn sites(live: &ray::Live, adaptive: bool, total: usize) -> Vec<(String, Site)> {
    let mut order: Vec<&str> = Vec::new();
    for i in live.recent.iter().rev() {
        if let Some(h) = i.host.as_deref().filter(|_| tcp(i)) {
            if !order.contains(&h) {
                order.push(h);
            }
        }
    }
    let mut out = Vec::new();
    for h in order {
        let tries: Vec<(&str, bool)> = live
            .recent
            .iter()
            .filter(|i| tcp(i) && i.host.as_deref() == Some(h))
            .filter_map(|i| Some((i.technique.as_deref()?, i.outcome.as_deref()? == "answered")))
            .collect();
        let Some(&(tech, ok)) = tries.last() else { continue };
        let last = live.recent.iter().rev().find(|i| tcp(i) && i.host.as_deref() == Some(h) && i.outcome.is_some());
        if last.and_then(|i| i.outcome.as_deref()) == Some("frozen") {
            out.push((h.to_string(), Site::Frozen));
            continue;
        }
        if ok {
            if tries.iter().any(|(t, a)| !a && *t != tech) {
                out.push((h.to_string(), Site::Tuned(tech.to_string())));
            }
            continue;
        }
        let from = tries.iter().rposition(|(_, a)| *a).map_or(0, |p| p + 1);
        let mut tried: Vec<&str> = tries[from..].iter().map(|(t, _)| *t).collect();
        tried.sort_unstable();
        tried.dedup();
        let state = if adaptive && tried.len() < limit(h, total) { Site::Tuning(tried.len()) } else { Site::Failing };
        out.push((h.to_string(), state));
    }
    out
}

fn chart(ui: &mut egui::Ui, rows: &[Row]) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 100.0), Sense::hover());
    let p = ui.painter();
    let n = crate::core::ray::HISTORY.max(rows.len());
    let max = rows.iter().map(|r| r.0.max(r.1 + r.2 + r.3)).max().unwrap_or(0).max(1) as f32;
    let slot = rect.width() / n as f32;
    let gap = (slot * 0.22).clamp(2.0, 5.0);
    let start = n - rows.len();
    let radius = ((slot - gap) / 2.0).min(7.0);
    let corner = radius as u8;
    let mut hover = None;
    let quiet = rows.iter().all(|r| r.0 == 0);
    for k in 0..n {
        let x = rect.left() + k as f32 * slot;
        let tube = egui::Rect::from_min_max(egui::pos2(x + gap / 2.0, rect.top()), egui::pos2(x + slot - gap / 2.0, rect.bottom()));
        let row = k.checked_sub(start).and_then(|i| rows.get(i).map(|r| (i, *r)));
        let here = !quiet && row.is_some() && resp.hover_pos().is_some_and(|pos| pos.x >= x && pos.x < x + slot);
        p.rect_filled(tube, CornerRadius::same(corner), Color32::from_white_alpha(if here { 12 } else { 5 }));
        let Some((i, r)) = row else { continue };
        if here {
            hover = Some((rows.len() - 1 - i, r));
        }
        if r.0 == 0 {
            continue;
        }
        let h = |v: u64| v as f32 / max * rect.height();
        let bar = egui::Rect::from_min_max(egui::pos2(tube.left(), rect.bottom() - h(r.0).max(radius * 2.0).min(rect.height())), tube.max);
        p.rect_filled(bar, CornerRadius::same(corner), Color32::from_white_alpha(18));
        let parts: Vec<(u64, Color32)> = [(r.1, PINK), (r.3, GREEN), (r.2, CYAN)].into_iter().filter(|(v, _)| *v > 0).collect();
        let last = parts.len().saturating_sub(1);
        let full = r.1 + r.2 + r.3 >= r.0;
        let mut edges = vec![rect.bottom()];
        for (i, (v, _)) in parts.iter().enumerate() {
            let below = edges[i];
            edges.push(if i == last && full { bar.top() } else { (below - h(*v)).max(bar.top()) });
        }
        let min_h = 3.0;
        for i in (1..=last).rev() {
            let limit = edges[i + 1] + min_h;
            if edges[i] < limit {
                edges[i] = limit.min(rect.bottom());
            }
        }
        let pill = egui::Rect::from_min_max(egui::pos2(tube.left(), bar.top()), tube.max);
        for (i, (_, c)) in parts.iter().enumerate() {
            let (bottom, top) = (edges[i], edges[i + 1]);
            if bottom - top < 0.5 {
                continue;
            }
            let band = egui::Rect::from_min_max(egui::pos2(tube.left() - 1.0, top), egui::pos2(tube.right() + 1.0, bottom));
            p.with_clip_rect(band.intersect(p.clip_rect())).rect_filled(pill, CornerRadius::same(corner), c.gamma_multiply(0.78));
        }
        if let Some(s) = theme::soft_glass_with(bar, radius, if here { 0.6 } else { 0.3 }, 0.0, Color32::WHITE) {
            p.add(s);
        }
    }
    if quiet {
        let c = rect.center();
        p.text(c - Vec2::new(0.0, 12.0), egui::Align2::CENTER_CENTER, theme::icon::DIAGNOSTIC, theme::icon_font(20.0), Color32::from_gray(130));
        p.text(c + Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, "Здесь появится активность, когда программы откроют сайты из списков", egui::FontId::proportional(13.0), Color32::from_gray(160));
    }
    if let Some((ago, r)) = hover {
        let when = if ago == 0 { "Эта минута".to_string() } else { format!("{ago} мин назад") };
        resp.on_hover_text(format!("{when}: через обход {}, ответили {}, подобрано само {}, не прошли {}", r.0, r.1, r.3, r.2));
    }
}

fn app_name(app: &str) -> String {
    match app.strip_prefix(ray::DEVICE) {
        Some(ip) => format!("Устройство {ip}"),
        None => app.to_string(),
    }
}

const ROWS: usize = 4;

fn limit(host: &str, total: usize) -> usize {
    if host.starts_with(rayoflight::flows::UDP_KEY) {
        rayoflight::flows::UDP_ROTATION.len()
    } else {
        total + settings::ray_warp() as usize
    }
}

pub fn shown(host: &str) -> String {
    match host.strip_prefix(rayoflight::flows::UDP_KEY) {
        Some(app) => format!("UDP · {app}"),
        None => host.to_string(),
    }
}

fn tuned_card(ui: &mut egui::Ui, list: &[(String, Site)], adaptive: bool, total: usize, next: &std::collections::HashMap<String, String>) {
    theme::card(ui, |ui| {
        ui.set_min_height(170.0);
        ui.label(RichText::new("Подобрано само").font(crate::theme::semibold(15.0)).strong());
        ui.add_space(4.0);
        if !adaptive {
            empty(ui, theme::icon::SETTINGS, "Самоподстройка выключена: приём выбран вручную в карточке RayOfLight");
            return;
        }
        if list.is_empty() {
            empty(ui, theme::icon::CHECK, "Пока нечего: всё открылось с первого приёма");
            return;
        }
        for (host, state) in list.iter().take(ROWS) {
            let total = limit(host, total);
            site_row(ui, 36.0, &shown(host), |ui| match state {
                Site::Tuned(tech) => {
                    ui.label(RichText::new(ray::experiment_inline(tech)).size(12.5).color(GREEN))
                        .on_hover_text(format!("Сначала сайт не ответил, самоподстройка нашла рабочий приём: {}", ray::experiment_inline(tech)));
                }
                Site::Tuning(n) => {
                    let text = match next.get(host) {
                        Some(t) => format!("подбирается · дальше {}", ray::experiment_inline(t)),
                        None => format!("подбирается · {n} из {total}"),
                    };
                    ui.label(muted(text)).on_hover_text(format!(
                        "Сайт пока не ответил, перепробовано приёмов: {n} из {total}. Если программы молчат 30 секунд, движок сам проверяет сайт следующим приёмом"
                    ));
                }
                Site::Failing | Site::Frozen => {}
            });
        }
        if list.len() > ROWS {
            ui.label(muted(format!("ещё {}", list.len() - ROWS)));
        }
        ui.add(egui::Label::new(muted("Пока приём подбирается, сайт может пару раз не открыться, дальше откроется сам")).wrap());
    });
}

fn failing_card(ui: &mut egui::Ui, bad: &[(String, Site)], adaptive: bool, busy: bool, warp: bool) -> Option<String> {
    let mut pick = None;
    theme::card(ui, |ui| {
        ui.set_min_height(170.0);
        ui.label(RichText::new("Не открываются").font(crate::theme::semibold(15.0)).strong());
        ui.add_space(4.0);
        if bad.is_empty() {
            let text = if adaptive { "Таких сайтов нет: с остальными самоподстройка справилась" } else { "Всё, что открывали через обход, ответило" };
            empty(ui, theme::icon::CHECK, text);
            return;
        }
        for (host, state) in bad.iter().take(ROWS) {
            site_row(ui, 36.0, &shown(host), |ui| {
                if *state == Site::Frozen {
                    let hint = if warp {
                        "Сервер ответил, но после 12–32 КБ замолчал: так провайдер режет загрузку с некоторых хостингов. Движок берёт другой адрес сервера, а если и он замолчит — пускает сайт через WARP"
                    } else {
                        "Сервер ответил, но после 12–32 КБ замолчал: так провайдер режет загрузку с некоторых хостингов. Движок берёт другой адрес сервера. Если не поможет, включите «WARP, если не открылось»"
                    };
                    ui.label(RichText::new("обрывает загрузку").size(12.5).color(ORANGE)).on_hover_text(hint);
                } else if host.starts_with(rayoflight::flows::UDP_KEY) {
                    ui.label(muted("игра: сервер не отвечает"))
                        .on_hover_text("Самоподстройка перепробовала все приёмы для UDP этой программы. Часто это блокировка по адресу сервера: помогает только WARP в режиме «весь трафик» (прокси WARP не пропускает UDP)");
                } else if ui.add_enabled(!busy, pill_button("Подобрать")).on_hover_text("Подобрать правило для этого сайта").clicked() {
                    pick = Some(host.clone());
                }
            });
        }
        if bad.len() > ROWS {
            ui.label(muted(format!("ещё {}", bad.len() - ROWS)));
        }
        let failing = bad.iter().any(|(_, s)| *s != Site::Frozen);
        let text = if !failing {
            "Сервер отвечает, но обрывает загрузку: движок берёт другой адрес, потом WARP"
        } else if adaptive {
            "Самоподстройка перепробовала все приёмы, сайт так и не ответил"
        } else {
            "Сайт не ответил на выбранный приём"
        };
        ui.add(egui::Label::new(muted(text)).wrap());
    });
    pick
}

impl App {
    pub(super) fn ray_strategy_ok(&mut self) -> Option<bool> {
        let id = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from))?;
        if let Some((cached, ok)) = &self.home.checked {
            if *cached == id {
                return Some(*ok);
            }
        }
        let p = self.paths.as_ref()?;
        let s = self.strategies.iter().find(|s| s.matches(&id))?;
        let _ = (p, s);
        let ok = id.to_lowercase().starts_with("rayoflight:");
        self.home.checked = Some((id, ok));
        Some(ok)
    }

    pub(super) fn install_general(&mut self, ctx: &egui::Context) {
        match self.ray_current() {
            Some(s) => self.install_strategy(ctx, s),
            None => self.toast(ctx, crate::log::Lvl::Err, "Нет файла bin\\RayOfLight.exe: скачайте HolyZapret заново"),
        }
    }

    pub(super) fn ray_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_secs(2));
        let installed = self.status.as_ref().is_some_and(|s| s.service != crate::core::service::ServiceState::NotInstalled && s.foreign_dir().is_none());
        let strategy_ok = self.ray_strategy_ok();
        let mut fix = false;
        if installed && strategy_ok == Some(false) {
            let name = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from)).unwrap_or_default();
            theme::card(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(format!("Служба установлена старой версией HolyZapret («{name}»). Переустановите её, чтобы обход шёл через RayOfLight.")).color(YELLOW).size(13.0)).wrap());
                theme::row(ui, 36.0, |ui| {
                    if ui.add_enabled(!self.busy, theme::Primary("Переустановить службу".into())).clicked() {
                        fix = true;
                    }
                });
            });
        } else if installed && !self.home.fresh {
            let last = self.paths.as_ref().and_then(ray::last_line).unwrap_or_else(|| "журнал пуст".into());
            theme::card(ui, |ui| {
                ui.add(egui::Label::new(RichText::new("RayOfLight сейчас не работает. Последняя строка его журнала:").color(YELLOW).size(13.0)).wrap());
                ui.add(egui::Label::new(muted(last)).wrap());
            });
        }
        if fix {
            self.install_general(ctx);
        }
        let live = self.home.live.clone().filter(|_| self.home.fresh);
        let rows: Vec<Row> = live.as_ref().map(|l| l.history.clone()).unwrap_or_default();
        let last = &rows[rows.len().saturating_sub(WINDOW)..];
        let (h, a, f, t) = sum(last);
        let rate = if a + f + t > 0 { Some((a + t) as f64 * 100.0 / (a + f + t) as f64) } else { None };
        let (v, col) = match rate {
            Some(r) => (format!("{r:.0}%"), if r >= 90.0 { GREEN } else if r >= 70.0 { YELLOW } else { RED }),
            None => ("—".to_string(), Color32::WHITE),
        };
        let stats = [
            ("За 10 минут", h.to_string(), "соединений".to_string(), Color32::WHITE),
            ("Сайты ответили", v, format!("{} из {}", a + t, a + f + t), col),
            ("Подобрано само", t.to_string(), "с другим приёмом".to_string(), if t > 0 { GREEN } else { Color32::WHITE }),
            ("Не прошли", f.to_string(), "сброс или тишина".to_string(), if f > 0 { YELLOW } else { Color32::WHITE }),
        ];
        let load = live.as_ref().map(|l| {
            let cpu = format!("{:.1}", l.cpu_x100 as f64 / 100.0).replace('.', ",");
            format!("движок: процессор {cpu}% · {} МБ", l.mem / (1024 * 1024))
        });
        ui.add_space(2.0);
        theme::card(ui, |ui| {
            let gap = 32.0;
            ui.spacing_mut().item_spacing.x = gap;
            let rects = ui.columns(stats.len(), |c| {
                stats.iter().zip(c.iter_mut()).map(|((l, v, s, col), cu)| {
                    stat(cu, l, v, s, *col, 26.0);
                    cu.min_rect()
                }).collect::<Vec<_>>()
            });
            ui.spacing_mut().item_spacing.x = 10.0;
            let line = egui::Stroke::new(1.0, Color32::from_white_alpha(14));
            for pair in rects.windows(2) {
                let x = pair[0].right() + gap / 2.0;
                ui.painter().vline(x, pair[0].top() + 2.0..=pair[0].bottom() - 2.0, line);
            }
            ui.add_space(4.0);
            let y = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), y, line);
            ui.add_space(8.0);
            theme::row(ui, 28.0, |ui| {
                ui.label(RichText::new("Активность за 30 минут").font(crate::theme::semibold(15.0)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((down, up)) = self.home.rate {
                        ui.label(muted(format!("сеть сейчас ↓ {} ↑ {}", netspeed::mbit(down), netspeed::mbit(up))));
                    }
                });
            });
            chart(ui, &rows);
            ui.add_space(2.0);
            theme::row(ui, 26.0, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (c, t) in [(PINK, "ответили"), (GREEN, "подобрано само"), (CYAN, "не прошли"), (Color32::from_white_alpha(50), "без итога")] {
                    egui::Frame::new().fill(Color32::from_white_alpha(8)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(9, 3)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        dot(ui, c, 8.0);
                        ui.label(muted(t).size(12.5));
                    });
                }
                if let Some(load) = &load {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(muted(load).size(12.5)).on_hover_text("Сколько процессора и памяти занимает RayOfLight");
                    });
                }
            });
        });
        ui.add_space(2.0);
        let apps: Vec<ray::AppActivity> = live.as_ref().map(|l| ray::by_app(l).into_iter().filter(|a| a.handled > 0).collect()).unwrap_or_default();
        let mut apps = apps;
        apps.sort_by(|x, y| y.handled.cmp(&x.handled));
        let adaptive = settings::ray_experiment() == "adaptive";
        let total = rotation_for(&ray::tcp_experiment());
        let all: Vec<(String, Site)> = live.as_ref().map(|l| sites(l, adaptive, total)).unwrap_or_default();
        let mut bad: Vec<(String, Site)> = all.iter().filter(|(_, s)| *s == Site::Frozen).cloned().collect();
        bad.extend(all.iter().filter(|(_, s)| *s == Site::Failing).cloned());
        let warp = settings::ray_warp();
        let mut tuned: Vec<(String, Site)> = all.iter().filter(|(_, s)| matches!(s, Site::Tuning(_))).cloned().collect();
        tuned.extend(all.iter().filter(|(_, s)| matches!(s, Site::Tuned(_))).cloned());
        let busy = self.busy;
        let next = &self.home.next;
        let mut pick = None;
        let mut more = false;
        let wide = ui.available_width() >= 1000.0;
        let icons: Vec<Option<egui::TextureHandle>> = apps
            .iter()
            .take(5)
            .map(|a| if a.app.starts_with(ray::DEVICE) || a.app == "неизвестная программа" { None } else { super::ray_ui::app_icon(ctx, &mut self.ray.icons, &a.app) })
            .collect();
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.columns(if wide { 3 } else { 2 }, |c| {
            for col in c.iter_mut() {
                col.spacing_mut().item_spacing.x = 10.0;
            }
            theme::card(&mut c[0], |ui| {
                ui.set_min_height(170.0);
                ui.label(RichText::new("Что идёт через обход").font(crate::theme::semibold(15.0)).strong());
                ui.add_space(4.0);
                if apps.is_empty() {
                    empty(ui, theme::icon::GLOBE, "Пока ничего: программы появятся, когда откроют сайт из списков");
                }
                for (a, tex) in apps.iter().take(5).zip(&icons) {
                    app_row(ui, tex.as_ref(), &app_name(&a.app), |ui| {
                        let share = if a.answered + a.failed > 0 { format!(" · ответили {}%", a.answered * 100 / (a.answered + a.failed)) } else { String::new() };
                        ui.label(muted(format!("{}{share}", a.handled)));
                    });
                }
                if apps.len() > 5 && ui.link(muted(format!("ещё {} — подробно в «Настройках»", apps.len() - 5))).clicked() {
                    more = true;
                }
            });
            if wide {
                tuned_card(&mut c[1], &tuned, adaptive, total, next);
                pick = failing_card(&mut c[2], &bad, adaptive, busy, warp);
            } else {
                tuned_card(&mut c[1], &tuned, adaptive, total, next);
                c[1].add_space(2.0);
                pick = failing_card(&mut c[1], &bad, adaptive, busy, warp);
            }
        });
        if let Some(host) = pick {
            self.tab = super::Tab::Sites;
            self.check_site_rule(ctx, &format!("https://{host}"));
        }
        if more {
            self.tab = super::Tab::Tools;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(host: &str, tech: &str, outcome: Option<&str>) -> ray::LiveItem {
        ray::LiveItem { host: Some(host.into()), handled: true, technique: Some(tech.into()), outcome: outcome.map(String::from), ..Default::default() }
    }

    #[test]
    fn games_use_their_own_rotation() {
        let mut recent: Vec<ray::LiveItem> = ["udp-none", "udp-fake", "udp-random", "udp-quic"].iter().map(|t| item("udp:g.exe", t, Some("silent"))).collect();
        recent.extend([item("udp:h.exe", "udp-none", Some("silent")), item("udp:h.exe", "udp-fake", Some("silent"))]);
        let live = ray::Live { recent, ..Default::default() };
        assert_eq!(sites(&live, true, 2), vec![("udp:h.exe".into(), Site::Tuning(2)), ("udp:g.exe".into(), Site::Failing)]);
        assert_eq!(shown("udp:h.exe"), "UDP · h.exe");
        assert_eq!(shown("discord.com"), "discord.com");
        assert_eq!(ray::experiment_inline("udp-fake"), "фейки как в general");
    }

    #[test]
    fn frozen_site_is_shown_apart_and_warp_answer_counts_as_tuned() {
        let live = ray::Live {
            recent: vec![
                item("tr.rbxcdn.com", "general", Some("answered")),
                item("tr.rbxcdn.com", "general", Some("frozen")),
                item("sc0.rbxcdn.com", "general", Some("frozen")),
                item("sc0.rbxcdn.com", "warp", Some("answered")),
            ],
            ..Default::default()
        };
        assert_eq!(sites(&live, true, 11), vec![("sc0.rbxcdn.com".into(), Site::Tuned("warp".into())), ("tr.rbxcdn.com".into(), Site::Frozen)]);
        let parsed = ray::parse_live(r#"{"running":true,"answered":1,"reset":0,"silent":0,"frozen":2,"techniques":[{"name":"general","used":3,"answered":1,"reset":0,"silent":0,"frozen":2}],"recent":[]}"#).unwrap();
        assert_eq!((parsed.frozen, parsed.techniques[0].frozen), (2, 2));
        assert_eq!(ray::tech_result(&parsed.techniques[0]), "1 из 3");
        assert_eq!(ray::outcome_label("frozen"), "сервер оборвал загрузку");
    }

    #[test]
    fn sites_split_into_tuned_tuning_and_failing() {
        let live = ray::Live {
            recent: vec![
                item("a.com", "general", Some("silent")),
                item("a.com", "general", Some("answered")),
                item("b.com", "general", Some("answered")),
                item("b.com", "general", Some("reset")),
                item("c.com", "general", None),
                item("d.com", "general", Some("silent")),
                item("d.com", "sni-split", Some("silent")),
                item("d.com", "none", Some("answered")),
                item("e.com", "quic-general", Some("silent")),
                item("f.com", "general", Some("silent")),
                item("f.com", "sni-split", Some("silent")),
            ],
            ..Default::default()
        };
        let got = sites(&live, true, 2);
        assert_eq!(got, vec![("f.com".into(), Site::Failing), ("d.com".into(), Site::Tuned("none".into())), ("b.com".into(), Site::Tuning(1))]);
        let fixed = sites(&live, false, 2);
        assert_eq!(fixed.iter().filter(|(_, s)| *s == Site::Failing).count(), 2);
        assert_eq!((rotation_for("adaptive"), rotation_for("adaptive:fakedsplit"), rotation_for("ttl")), (10, 11, 1));
        assert_eq!(sum(&[(1, 2, 3, 4), (4, 5, 6, 7)]), (5, 7, 9, 11));
        let next = ray::parse_memory("a.com\tnone\t1\t4\nb.com\tquic-random\t0\t2\n\tgeneral\t0\t0\n");
        assert_eq!(next.len(), 1);
        assert_eq!(next.get("a.com").map(String::as_str), Some("none"));
        assert_eq!(app_name("устройство 192.168.0.4"), "Устройство 192.168.0.4");
    }
}
