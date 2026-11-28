use super::{App, Msg};
use crate::core::checks::{self, SiteResult};
use crate::log::Lvl;
use crate::theme::{self, CYAN, GREEN, PINK, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Sense, Vec2};
use std::sync::mpsc::channel;

const TILE_MIN: f32 = 190.0;
const TILE_H: f32 = 58.0;
const GAP: f32 = 8.0;

pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    rest.split(['/', '?', '#', ':']).next().unwrap_or(rest).to_string()
}

pub fn short_error(detail: &str) -> String {
    let d = detail.to_lowercase();
    if d.starts_with("http ") {
        return detail.to_string();
    }
    let map = [
        (&["timed out", "timeout"][..], "таймаут"),
        (&["dns", "resolve", "lookup", "no such host"][..], "DNS не найден"),
        (&["refused"][..], "соединение отклонено"),
        (&["reset", "aborted"][..], "соединение сброшено"),
        (&["certificate", "tls", "handshake"][..], "ошибка TLS"),
    ];
    for (keys, text) in map {
        if keys.iter().any(|k| d.contains(k)) {
            return text.to_string();
        }
    }
    let mut s: String = detail.chars().take(28).collect();
    if detail.chars().count() > 28 {
        s.push('…');
    }
    s
}

fn ring(ui: &mut egui::Ui, frac: f32, label: &str) {
    let d = 72.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
    let c = rect.center();
    let r = d / 2.0 - 5.0;
    let p = ui.painter();
    p.circle_stroke(c, r, egui::Stroke::new(5.0, Color32::from_white_alpha(18)));
    let frac = frac.clamp(0.0, 1.0);
    if frac > 0.005 {
        let tau = std::f32::consts::TAU;
        let steps = ((96.0 * frac).ceil() as usize).max(2);
        let points: Vec<egui::Pos2> = (0..=steps).map(|i| c + Vec2::angled(-tau / 4.0 + tau * frac * i as f32 / steps as f32) * r).collect();
        let stroke = egui::epaint::PathStroke::new_uv(5.0, move |_, pos| {
            let v = pos - c;
            let f = ((v.y.atan2(v.x) + tau / 4.0).rem_euclid(tau)) / tau;
            theme::grad(f)
        });
        p.add(egui::Shape::Path(egui::epaint::PathShape::line(points, stroke)));
    }
    p.text(c, egui::Align2::CENTER_CENTER, label, theme::semibold(17.0), Color32::WHITE);
}

fn stat(ui: &mut egui::Ui, value: &str, caption: &str, color: Color32, divider: bool) {
    let r = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(value).font(theme::semibold(22.0)).strong().color(color));
            ui.add(egui::Label::new(RichText::new(caption).color(Color32::from_gray(175)).size(13.0)).extend());
        })
        .response
        .rect;
    if divider {
        ui.add_space(16.0);
        let x = ui.cursor().left();
        ui.painter().vline(x, r.top() + 4.0..=r.bottom() - 4.0, egui::Stroke::new(1.0, Color32::from_white_alpha(14)));
        ui.add_space(16.0);
    }
}

fn sites_word(n: usize) -> &'static str {
    let (d, h) = (n % 10, n % 100);
    if d == 1 && h != 11 {
        "сайт"
    } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
        "сайта"
    } else {
        "сайтов"
    }
}

enum TileAct {
    Remove,
    Pick,
    Copy,
}

fn tile(ui: &mut egui::Ui, w: f32, s: &SiteResult, can_pick: bool) -> Option<TileAct> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, TILE_H), Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    let p = ui.painter();
    let bg = if hovered { 12 } else { 6 };
    p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(bg));
    if !s.ok && !s.warn() {
        p.rect_stroke(rect, CornerRadius::same(12), egui::Stroke::new(1.0, RED.gamma_multiply(0.35)), egui::StrokeKind::Inside);
    }
    let host = host_of(&s.url);
    let letter = host.trim_start_matches("www.").chars().next().unwrap_or('?').to_uppercase().to_string();
    let av = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(32.0));
    if s.warn() {
        p.circle_filled(av.center(), 16.0, YELLOW.gamma_multiply(0.45));
    } else if s.ok {
        theme::gradient_rect(p, av, 16.0, PINK, CYAN);
    } else {
        p.circle_filled(av.center(), 16.0, RED.gamma_multiply(0.35));
    }
    let letter_col = if !s.ok && !s.warn() { Color32::from_rgb(246, 170, 176) } else { Color32::from_rgb(30, 18, 36) };
    p.text(av.center(), egui::Align2::CENTER_CENTER, letter, theme::semibold(15.0), letter_col);
    let x = rect.left() + 52.0;
    let pick_btn = !s.ok && hovered && can_pick;
    let reserve = if pick_btn { 96.0 } else if s.custom { 34.0 } else { 10.0 };
    let max_w = rect.right() - x - reserve;
    let clip = egui::Rect::from_min_max(rect.min + Vec2::splat(2.0), egui::pos2(rect.right() - reserve, rect.bottom() - 2.0));
    let mut job = egui::text::LayoutJob::simple_singleline(host.clone(), FontId::proportional(14.0), Color32::WHITE);
    job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let name = p.layout_job(job);
    p.with_clip_rect(clip).galley(egui::pos2(x, rect.top() + 10.0), name, Color32::WHITE);
    let (sub, col) = if s.warn() {
        (format!("{} · отказ сервера", s.detail), YELLOW)
    } else if s.ok {
        (format!("{} мс · {}", s.ms, s.detail), Color32::from_gray(150))
    } else {
        (short_error(&s.detail), RED)
    };
    p.with_clip_rect(clip).text(
        egui::pos2(x, rect.bottom() - 15.0),
        egui::Align2::LEFT_CENTER,
        sub,
        FontId::proportional(12.0),
        col,
    );
    let mut act = None;
    if pick_btn {
        let right = if s.custom { rect.right() - 36.0 } else { rect.right() - 8.0 };
        let b = egui::Rect::from_min_max(egui::pos2(right - 84.0, rect.center().y - 13.0), egui::pos2(right, rect.center().y + 13.0));
        let br = ui.interact(b, ui.make_persistent_id(("site_pick", &s.url)), Sense::click());
        let fill = if br.hovered() { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(20) };
        ui.painter().rect_filled(b, CornerRadius::same(13), fill);
        ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, "Подобрать", FontId::proportional(12.5), Color32::WHITE);
        if br.on_hover_text("Подобрать правило для этого сайта: лаунчер по очереди проверит приёмы RayOfLight").clicked() {
            act = Some(TileAct::Pick);
        }
    }
    if s.custom && hovered {
        let b = egui::Rect::from_center_size(egui::pos2(rect.right() - 18.0, rect.top() + 18.0), Vec2::splat(24.0));
        let br = ui.interact(b, ui.make_persistent_id(("site_del", &s.url)), Sense::click());
        if br.hovered() {
            ui.painter().rect_filled(b, CornerRadius::same(6), RED.gamma_multiply(0.25));
        }
        ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, theme::icon::DELETE, theme::icon_font(12.0), Color32::from_gray(210));
        if br.on_hover_text("Убрать из моих сайтов").clicked() {
            act = Some(TileAct::Remove);
        }
    }
    let tip = if s.warn() {
        format!("{}\n{}: соединение прошло, но сервер отказал. Это не блокировка", s.url, s.detail)
    } else {
        format!("{}\n{}", s.url, s.detail)
    };
    resp.on_hover_text(tip).context_menu(|ui| {
        if crate::theme::button(ui, "Скопировать домен").clicked() {
            act = Some(TileAct::Copy);
            ui.close();
        }
        if !s.ok && can_pick && crate::theme::button(ui, "Подобрать правило").clicked() {
            act = Some(TileAct::Pick);
            ui.close();
        }
    });
    act
}

impl App {
    pub(super) fn ray_adapts(&self) -> bool {
        use crate::core::{ray, settings};
        let Some(p) = &self.paths else { return false };
        let adapts = settings::ray_experiment() == "adaptive" || settings::ray_autolist();
        adapts && ray::read_live(p).is_some_and(|(l, age)| l.running && age.as_secs() < 10)
    }

    pub(super) fn start_site_recheck(&mut self, ctx: &egui::Context, urls: Vec<String>) -> bool {
        let Some(p) = &self.paths else { return false };
        let sites: Vec<_> = checks::load_sites(p).into_iter().filter(|s| urls.contains(&s.url)).collect();
        if sites.is_empty() || self.warp_blocks_checks(super::AfterWarp::SiteRecheck(urls.clone())) {
            return false;
        }
        self.sites_running = true;
        self.spawn_site_check(ctx, sites);
        true
    }

    pub(super) fn start_site_check(&mut self, ctx: &egui::Context) {
        if self.warp_blocks_checks(super::AfterWarp::Sites) {
            return;
        }
        let Some(p) = &self.paths else { return };
        let sites = checks::load_sites(p);
        self.sites.clear();
        self.sites_total = sites.len();
        self.sites_running = true;
        self.spawn_site_check(ctx, sites);
    }

    fn spawn_site_check(&mut self, ctx: &egui::Context, sites: Vec<checks::Site>) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        let (itx, irx) = channel();
        let total = sites.len();
        checks::check_all(sites, itx);
        std::thread::spawn(move || {
            for r in irx.iter().take(total) {
                let _ = tx.send(Msg::Site(r));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::SitesDone);
            ctx.request_repaint();
        });
    }

    pub(super) fn sites_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.sites_card(ui, ctx);
            ui.add_space(2.0);
            self.picker_card(ui, ctx);
            if !self.picker.results.is_empty() {
                ui.add_space(2.0);
                self.picker_results(ui, ctx);
            }
            ui.add_space(2.0);
            self.rules_card(ui, ctx);
        });
    }

    fn sites_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let done = self.sites.len();
        let total = self.sites_total.max(done);
        let ok = self.sites.iter().filter(|s| s.ok).count();
        let warn = self.sites.iter().filter(|s| s.warn()).count();
        let fail = done - ok;
        let oks: Vec<u128> = self.sites.iter().filter(|s| s.ok && !s.warn()).map(|s| s.ms).collect();
        let avg = if oks.is_empty() { 0 } else { oks.iter().sum::<u128>() / oks.len() as u128 };
        let mut start = false;
        let mut start_pick = false;
        let mut add = false;
        let mut remove: Option<String> = None;
        let mut pick: Option<String> = None;
        let mut pick_all = false;
        let can_pick = !self.busy && !self.rules.running() && !self.sites_running && self.paths.is_some();
        let failed: Vec<String> = self.sites.iter().filter(|s| !s.ok).map(|s| s.url.clone()).collect();
        let listed = self.paths.as_ref().map(|p| checks::load_sites(p).len()).unwrap_or(0);
        let pick_label = self.picker_start_label();
        let pick_free = !self.busy && !self.picker.running && self.paths.is_some();
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Сайты").font(theme::semibold(17.0)).strong());
                ui.label(RichText::new(format!("быстрая проверка · {listed} {}", sites_word(listed))).color(Color32::from_gray(175)).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(pick_free, |ui| {
                        if ui.add(theme::Primary(pick_label.into())).on_hover_text("Подбор сравнит приёмы RayOfLight на сайтах и предложит лучший").clicked() {
                            start_pick = true;
                        }
                    });
                    ui.add_enabled_ui(!self.sites_running && self.paths.is_some(), |ui| {
                        let label = if done == 0 { "Проверить" } else { "Проверить снова" };
                        if crate::theme::button(ui, label).clicked() {
                            start = true;
                        }
                    });
                });
            });
            ui.add_space(4.0);
            theme::row(ui, 76.0, |ui| {
                let frac = if total == 0 { 0.0 } else { ok as f32 / total as f32 };
                let label = if done == 0 { "—".to_string() } else { format!("{:.0}%", frac * 100.0) };
                ring(ui, frac, &label);
                ui.add_space(18.0);
                if done == 0 && !self.sites_running {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Проверка ещё не запускалась").font(theme::semibold(15.0)).strong());
                        ui.label(RichText::new("Нажмите «Проверить», чтобы узнать, какие сайты открываются").color(Color32::from_gray(175)).size(13.0));
                    });
                } else {
                    let caption = if warn > 0 { format!("открываются, {warn} с ошибкой") } else { "открываются".to_string() };
                    stat(ui, &format!("{ok} из {total}"), &caption, Color32::WHITE, true);
                    stat(ui, &if oks.is_empty() { "—".into() } else { format!("{avg} мс") }, "в среднем", Color32::WHITE, true);
                    stat(ui, &fail.to_string(), "не открываются", if fail > 0 { RED } else { GREEN }, false);
                }
            });
            if self.sites_running {
                let frac = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(3), Color32::from_white_alpha(16));
                if frac > 0.01 {
                    let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height()));
                    theme::gradient_rect(ui.painter(), fill, 3.0, PINK, CYAN);
                }
            }
            theme::row(ui, 40.0, |ui| {
                if done > 0 {
                    let labels = vec![format!("Не открываются · {fail}"), format!("Все · {done}")];
                    theme::segmented(ui, &labels, &mut self.sites_filter);
                    if !failed.is_empty() && !self.sites_running {
                        ui.add_space(4.0);
                        if ui
                            .add_enabled(can_pick, crate::theme::GlassButton::new("Подобрать для них"))
                            .on_hover_text("По очереди подберёт правило для каждого сайта, который не открывается. Итог появится в карточке «Правила для сайтов» внизу")
                            .clicked()
                        {
                            pick_all = true;
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::theme::button(ui, "Добавить").clicked() {
                        add = true;
                    }
                    let r = ui.add(egui::TextEdit::singleline(&mut self.site_input).hint_text("Добавить свой сайт").desired_width(200.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        add = true;
                    }
                });
            });
            if done > 0 {
                let mut items: Vec<&SiteResult> = self.sites.iter().filter(|s| self.sites_filter == 1 || !s.ok).collect();
                items.sort_by_key(|s| (s.ok, !s.warn(), s.ms));
                let w = ui.available_width();
                let cols = (((w + GAP) / (TILE_MIN + GAP)).floor() as usize).max(1);
                let tw = (w - GAP * (cols as f32 - 1.0)) / cols as f32;
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                    for chunk in items.chunks(cols) {
                        ui.horizontal(|ui| {
                            for s in chunk {
                                match tile(ui, tw, s, can_pick) {
                                    Some(TileAct::Remove) => remove = Some(s.url.clone()),
                                    Some(TileAct::Pick) => pick = Some(s.url.clone()),
                                    Some(TileAct::Copy) => ui.ctx().copy_text(host_of(&s.url)),
                                    None => {}
                                }
                            }
                        });
                    }
                });
                if items.is_empty() && !self.sites_running {
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(format!("Все {done} {} открываются. Список — в «Все · {done}»", sites_word(done))).color(Color32::from_gray(175))));
                }
            }
        });
        if start {
            self.start_site_check(ctx);
        }
        if start_pick {
            self.start_pick(ctx);
        }
        if let Some(url) = pick {
            self.check_site_rule(ctx, &url);
        }
        if pick_all {
            self.start_batch(ctx, failed);
        }
        if add {
            if let Some(p) = &self.paths {
                match checks::add_my_site(p, &self.site_input) {
                    Ok(url) => {
                        self.site_input.clear();
                        self.picker.invalidate();
                        self.toast(ctx, Lvl::Ok, format!("Добавлен {url}: он будет в проверке сайтов и в подборе"));
                        let site = checks::Site { url, custom: true };
                        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
                        self.sites_total += 1;
                        std::thread::spawn(move || {
                            let _ = tx.send(Msg::Site(checks::check_one(&site)));
                            ctx2.request_repaint();
                        });
                    }
                    Err(e) => self.toast(ctx, Lvl::Err, e.to_string()),
                }
            }
        }
        if let Some(url) = remove {
            if let Some(p) = &self.paths {
                match checks::remove_my_site(p, &url) {
                    Ok(()) => {
                        self.sites.retain(|s| s.url != url);
                        self.sites_total = self.sites_total.saturating_sub(1);
                        self.picker.invalidate();
                        self.toast(ctx, Lvl::Info, format!("Убран {url}"));
                    }
                    Err(e) => self.toast(ctx, Lvl::Err, e.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_errors() {
        assert_eq!(host_of("https://www.youtube.com/watch?v=1"), "www.youtube.com");
        assert_eq!(host_of("http://x.org:8080"), "x.org");
        assert_eq!(short_error("http://x/: Timeout: global"), "таймаут");
        assert_eq!(short_error("io: failed to lookup address"), "DNS не найден");
        assert_eq!(short_error("HTTP 403"), "HTTP 403");
        assert_eq!(short_error("rustls: handshake failure"), "ошибка TLS");
    }
}
