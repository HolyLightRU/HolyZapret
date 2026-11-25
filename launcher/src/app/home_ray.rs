use super::App;
use crate::core::paths::Paths;
use crate::core::{netspeed, ray, settings};
use crate::theme::{self, CYAN, GREEN, PINK, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2};
use std::time::{Duration, Instant};

const WINDOW: usize = 10;

#[derive(Default)]
pub struct RayHome {
    at: Option<Instant>,
    pub on: bool,
    live: Option<ray::Live>,
    fresh: bool,
    net: Option<(Instant, u64, u64)>,
    rate: Option<(f64, f64)>,
    checked: Option<(String, bool)>,
}

impl RayHome {
    pub fn refresh(&mut self, paths: Option<&Paths>) {
        if self.at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        let now = Instant::now();
        self.at = Some(now);
        self.on = settings::ray_enabled();
        if !self.on {
            return;
        }
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
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

fn tile(ui: &mut egui::Ui, label: &str, value: &str, sub: &str, color: Color32) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(6))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(muted(label));
            ui.label(RichText::new(value).size(22.0).strong().color(color));
            ui.label(muted(sub));
        });
}

fn sum(rows: &[(u64, u64, u64)]) -> (u64, u64, u64) {
    rows.iter().fold((0, 0, 0), |a, r| (a.0 + r.0, a.1 + r.1, a.2 + r.2))
}

pub fn failing(live: &ray::Live) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for i in live.recent.iter().rev() {
        let (Some(h), Some(o)) = (&i.host, &i.outcome) else { continue };
        if !i.handled || !seen.insert(h.clone()) {
            continue;
        }
        if o != "answered" {
            out.push(h.clone());
        }
    }
    out
}

fn chart(ui: &mut egui::Ui, rows: &[(u64, u64, u64)]) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 86.0), Sense::hover());
    let p = ui.painter();
    let n = crate::core::ray::HISTORY.max(rows.len());
    let max = rows.iter().map(|r| r.0.max(r.1 + r.2)).max().unwrap_or(0).max(1) as f32;
    let slot = rect.width() / n as f32;
    let gap = (slot * 0.25).clamp(1.0, 4.0);
    let start = n - rows.len();
    let mut hover = None;
    for (k, r) in rows.iter().enumerate() {
        let x = rect.left() + (start + k) as f32 * slot;
        let col = egui::Rect::from_min_max(egui::pos2(x + gap / 2.0, rect.top()), egui::pos2(x + slot - gap / 2.0, rect.bottom()));
        let h = |v: u64| v as f32 / max * rect.height();
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(col.left(), rect.bottom() - h(r.0)), col.max), CornerRadius::same(2), Color32::from_white_alpha(14));
        let g = h(r.1);
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(col.left(), rect.bottom() - g), col.max), CornerRadius::same(2), PINK);
        let f = h(r.2);
        if r.2 > 0 {
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(col.left(), rect.bottom() - g - f), egui::pos2(col.right(), rect.bottom() - g)), CornerRadius::same(2), CYAN);
        }
        if resp.hover_pos().is_some_and(|pos| pos.x >= x && pos.x < x + slot) {
            hover = Some((rows.len() - 1 - k, *r));
        }
    }
    if rows.iter().all(|r| r.0 == 0) {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "Пока тихо: соединений через обход не было", egui::FontId::proportional(12.5), Color32::from_gray(140));
    }
    if let Some((ago, r)) = hover {
        let when = if ago == 0 { "Эта минута".to_string() } else { format!("{ago} мин назад") };
        resp.on_hover_text(format!("{when}: через обход {}, ответили {}, не прошли {}", r.0, r.1, r.2));
    }
}

fn app_name(app: &str) -> String {
    match app.strip_prefix(ray::DEVICE) {
        Some(ip) => format!("Устройство {ip}"),
        None => app.to_string(),
    }
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
        let ok = ray::takeover_for(p, s).is_some();
        self.home.checked = Some((id, ok));
        Some(ok)
    }

    pub(super) fn install_general(&mut self, ctx: &egui::Context) {
        match self.strategies.iter().position(|s| s.matches("general")) {
            Some(i) => {
                self.selected = Some(i);
                self.install_selected(ctx);
            }
            None => self.toast(ctx, crate::log::Lvl::Err, "Стратегия general не найдена в папке scripts"),
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
                ui.add(egui::Label::new(RichText::new(format!("Сейчас стоит стратегия «{name}», RayOfLight с ней не работает. Он работает со стратегией general и по проверкам обходит лучше остальных.")).color(YELLOW).size(13.0)).wrap());
                theme::row(ui, 36.0, |ui| {
                    if ui.add_enabled(!self.busy, theme::Primary("Поставить general".into())).clicked() {
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
        let rows: Vec<(u64, u64, u64)> = live.as_ref().map(|l| l.history.clone()).unwrap_or_default();
        let last = &rows[rows.len().saturating_sub(WINDOW)..];
        let (h, a, f) = sum(last);
        let rate = if a + f > 0 { Some(a as f64 * 100.0 / (a + f) as f64) } else { None };
        ui.columns(4, |c| {
            tile(&mut c[0], "Через обход за 10 мин", &h.to_string(), "соединений", Color32::WHITE);
            let (v, col) = match rate {
                Some(r) => (format!("{r:.0}%"), if r >= 90.0 { GREEN } else if r >= 70.0 { YELLOW } else { RED }),
                None => ("—".to_string(), Color32::WHITE),
            };
            tile(&mut c[1], "Сайты ответили", &v, &format!("{a} из {}", a + f), col);
            tile(&mut c[2], "Не прошли", &f.to_string(), "сброс или тишина", if f > 0 { YELLOW } else { Color32::WHITE });
            let (cpu, mem) = live.as_ref().map(|l| (l.cpu_x100 as f64 / 100.0, l.mem)).unwrap_or((0.0, 0));
            let load = if live.is_some() { format!("{cpu:.1}%").replace('.', ",") } else { "—".into() };
            tile(&mut c[3], "Нагрузка движка", &load, &format!("процессор · {} МБ памяти", mem / (1024 * 1024)), Color32::WHITE);
        });
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            theme::row(ui, 28.0, |ui| {
                ui.label(RichText::new("Активность за 30 минут").size(15.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((down, up)) = self.home.rate {
                        ui.label(muted(format!("сеть сейчас ↓ {} ↑ {}", netspeed::mbit(down), netspeed::mbit(up))));
                    }
                });
            });
            chart(ui, &rows);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (c, t) in [(PINK, "ответили"), (CYAN, "не прошли"), (Color32::from_white_alpha(40), "без итога")] {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(2), c);
                    ui.label(muted(t));
                    ui.add_space(8.0);
                }
            });
        });
        ui.add_space(8.0);
        let apps: Vec<ray::AppActivity> = live.as_ref().map(|l| ray::by_app(l).into_iter().filter(|a| a.handled > 0).collect()).unwrap_or_default();
        let mut apps = apps;
        apps.sort_by(|x, y| y.handled.cmp(&x.handled));
        let bad: Vec<String> = live.as_ref().map(failing).unwrap_or_default();
        let mut pick = None;
        let mut more = false;
        ui.columns(2, |c| {
            theme::card(&mut c[0], |ui| {
                ui.set_min_height(150.0);
                ui.label(RichText::new("Что идёт через обход").size(15.0).strong());
                ui.add_space(4.0);
                if apps.is_empty() {
                    ui.label(muted("Пока ничего: программы появятся, когда откроют сайт из списков"));
                }
                for a in apps.iter().take(5) {
                    theme::row(ui, 24.0, |ui| {
                        ui.add(egui::Label::new(RichText::new(app_name(&a.app)).size(13.5)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let share = if a.answered + a.failed > 0 { format!(" · ответили {}%", a.answered * 100 / (a.answered + a.failed)) } else { String::new() };
                            ui.label(muted(format!("{}{share}", a.handled)));
                        });
                    });
                }
                if apps.len() > 5 && ui.link(muted(format!("ещё {} — подробно в «Инструментах»", apps.len() - 5))).clicked() {
                    more = true;
                }
            });
            theme::card(&mut c[1], |ui| {
                ui.set_min_height(150.0);
                ui.label(RichText::new("Не открываются").size(15.0).strong());
                ui.add_space(4.0);
                if bad.is_empty() {
                    ui.label(muted("Всё, что открывали через обход, ответило"));
                } else {
                    for host in bad.iter().take(4) {
                        theme::row(ui, 36.0, |ui| {
                            ui.add(egui::Label::new(RichText::new(host).size(13.5)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(!self.busy, egui::Button::new("Подобрать")).on_hover_text("Подобрать правило для этого сайта").clicked() {
                                    pick = Some(host.clone());
                                }
                            });
                        });
                    }
                    ui.label(muted("Самоподстройка сама пробует другие приёмы, «Подобрать» — если сайт так и не открылся"));
                }
            });
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

    fn item(host: &str, outcome: Option<&str>) -> ray::LiveItem {
        ray::LiveItem { host: Some(host.into()), handled: true, outcome: outcome.map(String::from), ..Default::default() }
    }

    #[test]
    fn failing_takes_latest_outcome_per_site() {
        let live = ray::Live { recent: vec![item("a.com", Some("silent")), item("a.com", Some("answered")), item("b.com", Some("answered")), item("b.com", Some("reset")), item("c.com", None)], ..Default::default() };
        assert_eq!(failing(&live), vec!["b.com".to_string()]);
        assert_eq!(sum(&[(1, 2, 3), (4, 5, 6)]), (5, 7, 9));
        assert_eq!(app_name("устройство 192.168.0.4"), "Устройство 192.168.0.4");
    }
}
