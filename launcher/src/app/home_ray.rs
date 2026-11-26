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
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

pub(super) fn tile(ui: &mut egui::Ui, label: &str, value: &str, sub: &str, color: Color32) {
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

type Row = (u64, u64, u64, u64);

fn sum(rows: &[Row]) -> Row {
    rows.iter().fold((0, 0, 0, 0), |a, r| (a.0 + r.0, a.1 + r.1, a.2 + r.2, a.3 + r.3))
}

fn site_row(ui: &mut egui::Ui, height: f32, name: &str, right: impl FnOnce(&mut egui::Ui)) {
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 86.0), Sense::hover());
    let p = ui.painter();
    let n = crate::core::ray::HISTORY.max(rows.len());
    let max = rows.iter().map(|r| r.0.max(r.1 + r.2 + r.3)).max().unwrap_or(0).max(1) as f32;
    let slot = rect.width() / n as f32;
    let gap = (slot * 0.25).clamp(1.0, 4.0);
    let start = n - rows.len();
    let mut hover = None;
    for (k, r) in rows.iter().enumerate() {
        let x = rect.left() + (start + k) as f32 * slot;
        let col = egui::Rect::from_min_max(egui::pos2(x + gap / 2.0, rect.top()), egui::pos2(x + slot - gap / 2.0, rect.bottom()));
        let h = |v: u64| v as f32 / max * rect.height();
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(col.left(), rect.bottom() - h(r.0)), col.max), CornerRadius::same(2), Color32::from_white_alpha(14));
        let mut top = rect.bottom();
        for (v, c) in [(r.1, PINK), (r.3, GREEN), (r.2, CYAN)] {
            if v > 0 {
                let y = top - h(v);
                p.rect_filled(egui::Rect::from_min_max(egui::pos2(col.left(), y), egui::pos2(col.right(), top)), CornerRadius::same(2), c);
                top = y;
            }
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
        ui.label(RichText::new("Подобрано само").size(15.0).strong());
        ui.add_space(4.0);
        if !adaptive {
            ui.add(egui::Label::new(muted("Самоподстройка выключена: приём выбран вручную в карточке RayOfLight")).wrap());
            return;
        }
        if list.is_empty() {
            ui.add(egui::Label::new(muted("Пока нечего: всё открылось с первого приёма")).wrap());
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
        ui.label(RichText::new("Не открываются").size(15.0).strong());
        ui.add_space(4.0);
        if bad.is_empty() {
            let text = if adaptive { "Таких сайтов нет: с остальными самоподстройка справилась" } else { "Всё, что открывали через обход, ответило" };
            ui.add(egui::Label::new(muted(text)).wrap());
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
                } else if ui.add_enabled(!busy, egui::Button::new("Подобрать")).on_hover_text("Подобрать правило для этого сайта").clicked() {
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
        ui.columns(5, |c| {
            tile(&mut c[0], "Через обход за 10 мин", &h.to_string(), "соединений", Color32::WHITE);
            let (v, col) = match rate {
                Some(r) => (format!("{r:.0}%"), if r >= 90.0 { GREEN } else if r >= 70.0 { YELLOW } else { RED }),
                None => ("—".to_string(), Color32::WHITE),
            };
            tile(&mut c[1], "Сайты ответили", &v, &format!("{} из {}", a + t, a + f + t), col);
            tile(&mut c[2], "Подобрано само", &t.to_string(), "с другим приёмом", if t > 0 { GREEN } else { Color32::WHITE });
            tile(&mut c[3], "Не прошли", &f.to_string(), "сброс или тишина", if f > 0 { YELLOW } else { Color32::WHITE });
            let (cpu, mem) = live.as_ref().map(|l| (l.cpu_x100 as f64 / 100.0, l.mem)).unwrap_or((0.0, 0));
            let load = if live.is_some() { format!("{cpu:.1}%").replace('.', ",") } else { "—".into() };
            tile(&mut c[4], "Нагрузка движка", &load, &format!("процессор · {} МБ", mem / (1024 * 1024)), Color32::WHITE);
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
                for (c, t) in [(PINK, "ответили"), (GREEN, "подобрано само"), (CYAN, "не прошли"), (Color32::from_white_alpha(40), "без итога")] {
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
        ui.columns(if wide { 3 } else { 2 }, |c| {
            theme::card(&mut c[0], |ui| {
                ui.set_min_height(170.0);
                ui.label(RichText::new("Что идёт через обход").size(15.0).strong());
                ui.add_space(4.0);
                if apps.is_empty() {
                    ui.label(muted("Пока ничего: программы появятся, когда откроют сайт из списков"));
                }
                for a in apps.iter().take(5) {
                    site_row(ui, 24.0, &app_name(&a.app), |ui| {
                        let share = if a.answered + a.failed > 0 { format!(" · ответили {}%", a.answered * 100 / (a.answered + a.failed)) } else { String::new() };
                        ui.label(muted(format!("{}{share}", a.handled)));
                    });
                }
                if apps.len() > 5 && ui.link(muted(format!("ещё {} — подробно в «Инструментах»", apps.len() - 5))).clicked() {
                    more = true;
                }
            });
            if wide {
                tuned_card(&mut c[1], &tuned, adaptive, total, next);
                pick = failing_card(&mut c[2], &bad, adaptive, busy, warp);
            } else {
                tuned_card(&mut c[1], &tuned, adaptive, total, next);
                c[1].add_space(8.0);
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
