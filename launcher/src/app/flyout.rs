use super::App;
use crate::core::gamefilter;
use crate::core::service::ServiceState;
use crate::core::settings;
use crate::core::tgproxy::control;
use crate::log::Lvl;
use crate::theme::{self, GREEN, PINK, RED, YELLOW};
use crate::tray;
use eframe::egui::{self, Color32, RichText, Vec2};

const WIDTH: f32 = 360.0;
const MENU_WIDTH: f32 = 290.0;
const MENU_ROW: f32 = 36.0;
const FULL_MIN: Vec2 = Vec2::new(780.0, 520.0);

#[derive(Default)]
pub struct FlyState {
    pub compact: bool,
    open: bool,
    seen_focus: bool,
    reveal_in: u8,
    saved: Option<(egui::Pos2, Vec2, bool)>,
    confirm_off: bool,
    rows: usize,
    menu: bool,
}

enum Act {
    Power,
    PowerOff,
    Game(bool),
    Warp(bool),
    Telegram(bool),
    Open,
    Devices,
    Quit,
}

const POP_TITLE: &str = "HolyZapret · меню";

pub struct Pop {
    pos: egui::Pos2,
    ready: bool,
    seen_focus: bool,
    frames: u32,
}

fn menu_height() -> f32 {
    16.0 + 40.0 + 18.0 + MENU_ROW * 5.0
}

fn place(ctx: &egui::Context, at: (i32, i32), size: Vec2, menu: bool) -> Option<egui::Pos2> {
    let (mon, work) = tray::work_area(at.0, at.1)?;
    let ppp = ctx.pixels_per_point();
    let (w, h) = ((size.x * ppp) as i32, (size.y * ppp) as i32);
    let gap = (12.0 * ppp) as i32;
    let fit = |v: i32, lo: i32, hi: i32| v.min(hi).max(lo);
    let across = fit(at.0 - w / 2, work[0] + gap, work[2] - w - gap);
    let down = fit(at.1 - h / 2, work[1] + gap, work[3] - h - gap);
    if menu {
        let x = fit(at.0, work[0] + gap, work[2] - w - gap);
        let y = fit(at.1.min(work[3]) - h - gap / 2, work[1] + gap, work[3] - h - gap / 2);
        return Some(egui::pos2(x as f32 / ppp, y as f32 / ppp));
    }
    let (x, y) = if work[1] > mon[1] {
        (across, work[1] + gap)
    } else if work[0] > mon[0] {
        (work[0] + gap, down)
    } else if work[2] < mon[2] {
        (work[2] - w - gap, down)
    } else {
        (across, work[3] - h - gap)
    };
    Some(egui::pos2(x as f32 / ppp, y as f32 / ppp))
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(Color32::from_gray(165))
}

impl App {
    fn fly_size(&self) -> Vec2 {
        if self.fly.menu {
            return Vec2::new(MENU_WIDTH, menu_height());
        }
        let rows = 1 + self.fly_warp_row() as usize + self.tg.client.is_some() as usize;
        Vec2::new(WIDTH, 346.0 + 50.0 * rows as f32)
    }

    fn fly_warp_row(&self) -> bool {
        settings::ray_experiment() == "adaptive" && self.status.as_ref().is_none_or(|s| s.foreign_dir().is_none())
    }


    pub(super) fn fly_update(&mut self, ctx: &egui::Context) {
        if let Some((ax, ay, menu)) = tray::take_fly() {
            let at = (ax, ay);
            self.fly.menu = menu;
            self.fly.rows = 0;
            if !self.fly.compact {
                let (outer, inner, max) = ctx.input(|i| {
                    let v = i.viewport();
                    (v.outer_rect, v.inner_rect, v.maximized.unwrap_or(false))
                });
                if let (Some(o), Some(i)) = (outer, inner) {
                    self.fly.saved = Some((o.min, i.size(), max));
                }
                if max {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                }
            }
            let size = self.fly_size();
            self.fly.compact = true;
            self.fly.open = true;
            self.fly.seen_focus = false;
            self.fly.confirm_off = false;
            tray::set_compact(true);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if let Some(p) = place(ctx, at, size, self.fly.menu) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
            }
            self.fly.reveal_in = 3;
        }
        let shown_elsewhere = self.fly.compact && !self.fly.open && tray::window_visible();
        if tray::take_full() || shown_elsewhere {
            if self.fly.compact {
                self.fly_restore(ctx);
            }
        }
        if self.fly.compact && self.fly.open && !self.fly.menu {
            let rows = 1 + self.fly_warp_row() as usize + self.tg.client.is_some() as usize;
            if rows != self.fly.rows {
                self.fly.rows = rows;
                let size = self.fly_size();
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
        }
        if self.fly.reveal_in > 0 {
            self.fly.reveal_in -= 1;
            if self.fly.reveal_in == 0 {
                tray::cloak(false);
            }
            ctx.request_repaint();
            return;
        }
        if self.fly.compact && self.fly.open {
            let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
            let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if focused {
                self.fly.seen_focus = true;
            } else if self.fly.seen_focus || esc {
                self.fly_close();
            }
            if esc && focused {
                self.fly_close();
            }
        }
    }

    fn fly_close(&mut self) {
        self.fly.open = false;
        self.fly.confirm_off = false;
        tray::hide_window();
        tray::fly_closed();
    }

    fn fly_restore(&mut self, ctx: &egui::Context) {
        tray::cloak(true);
        tray::topmost(false);
        self.fly.compact = false;
        self.fly.open = false;
        self.fly.rows = 0;
        tray::set_compact(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(FULL_MIN));
        let (pos, size, max) = self.fly.saved.take().unwrap_or((egui::pos2(120.0, 80.0), Vec2::new(940.0, 640.0), false));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.max(FULL_MIN)));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        if max {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.fly.reveal_in = 3;
    }

    pub(super) fn flyout_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.fly.menu {
            self.menu_ui(ui, ctx);
            return;
        }
        self.home.refresh(self.paths.as_ref());
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
        let mut act = None;
        let status = self.status.clone();
        let busy = self.busy || tray::busy();
        let foreign = status.as_ref().is_some_and(|s| s.foreign_dir().is_some());
        let state = status.as_ref().map(|s| s.service.clone());
        let running = state == Some(ServiceState::Running) && !foreign;
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin::same(16))).show(ui, |ui| {
            theme::row(ui, 28.0, |ui| {
                ui.label(RichText::new("HolyZapret").font(theme::semibold(16.0)).strong().color(Color32::WHITE));
                ui.label(muted(format!("v{}", env!("CARGO_PKG_VERSION"))).size(12.5));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let tip = match &state {
                    _ if foreign => "Поставить службу из этой папки",
                    Some(ServiceState::Running) => "Выключить защиту",
                    Some(ServiceState::Stopped) => "Запустить службу",
                    _ => "Включить защиту",
                };
                if theme::power_button(ui, running, busy).on_hover_text(tip).clicked() && !busy && self.paths.is_some() {
                    act = Some(if running { Act::PowerOff } else { Act::Power });
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(16.0);
                    let title = match &state {
                        _ if foreign => "Служба из другой папки",
                        Some(ServiceState::Running) => "Защита активна",
                        Some(ServiceState::Stopped) => "Служба остановлена",
                        Some(ServiceState::NotInstalled) => "Защита выключена",
                        None => "Читаю состояние…",
                    };
                    ui.label(RichText::new(title).font(theme::semibold(19.0)).strong().color(Color32::WHITE));
                    ui.add_space(2.0);
                    let engine_down = running && status.as_ref().is_some_and(|s| !s.engine_running);
                    if engine_down {
                        theme::chip(ui, "движок остановлен", RED);
                    } else if running {
                        let mode = if settings::ray_experiment() == "adaptive" { "самоподстройка".to_string() } else { crate::core::ray::experiment_label(&settings::ray_experiment()).to_string() };
                        theme::chip(ui, &mode, PINK);
                    } else {
                        ui.label(muted("Нажмите кнопку, чтобы включить").size(13.0));
                    }
                });
            });
            ui.add_space(10.0);
            let line = egui::Stroke::new(1.0, Color32::from_white_alpha(14));
            if self.fly.confirm_off {
                egui::Frame::new().fill(RED.gamma_multiply(0.12)).corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Выключить защиту?").font(theme::semibold(15.0)).strong().color(Color32::WHITE));
                    ui.add(egui::Label::new(muted("Служба будет остановлена и удалена. Включить обратно можно этой же кнопкой.").size(12.5)).wrap());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("Выключить").color(Color32::WHITE)).fill(RED.gamma_multiply(0.55))).clicked() {
                            act = Some(Act::Power);
                        }
                        if theme::button(ui, "Отмена").clicked() {
                            self.fly.confirm_off = false;
                        }
                    });
                });
            } else {
                let totals = self.home.totals();
                let (h, a, f, t) = totals.unwrap_or_default();
                let rate = if a + f + t > 0 { Some((a + t) as f64 * 100.0 / (a + f + t) as f64) } else { None };
                let (rv, rc) = match rate {
                    Some(r) => (format!("{r:.0}%"), if r >= 90.0 { GREEN } else if r >= 70.0 { YELLOW } else { RED }),
                    None => ("—".to_string(), Color32::WHITE),
                };
                let stats = [
                    ("За 10 мин", if totals.is_some() { h.to_string() } else { "—".into() }, Color32::WHITE),
                    ("Ответили", rv, rc),
                    ("Подобрано", if totals.is_some() { t.to_string() } else { "—".into() }, if t > 0 { GREEN } else { Color32::WHITE }),
                ];
                let gap = 20.0;
                ui.spacing_mut().item_spacing.x = gap;
                let rects = ui.columns(stats.len(), |c| {
                    stats
                        .iter()
                        .zip(c.iter_mut())
                        .map(|((l, v, col), cu)| {
                            cu.spacing_mut().item_spacing.y = 2.0;
                            cu.label(muted(*l).size(12.5));
                            cu.label(RichText::new(v).font(theme::semibold(21.0)).strong().color(*col));
                            cu.min_rect()
                        })
                        .collect::<Vec<_>>()
                });
                ui.spacing_mut().item_spacing.x = 8.0;
                for pair in rects.windows(2) {
                    let x = pair[0].right() + gap / 2.0;
                    ui.painter().vline(x, pair[0].top() + 2.0..=pair[0].bottom() - 2.0, line);
                }
            }
            ui.add_space(8.0);
            let y = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), y, line);
            if let Some(s) = &status {
                let mut on = s.game.enabled();
                if theme::setting_row(ui, true, "Игровой фильтр", &s.game.summary(), "", &mut on, !busy) {
                    act = Some(Act::Game(on));
                }
            }
            if self.fly_warp_row() {
                let mut on = settings::ray_warp();
                let (sub, tip) = self.warp_sub(on);
                if theme::setting_row(ui, false, "WARP, если не открылось", &sub, tip, &mut on, !busy) {
                    act = Some(Act::Warp(on));
                }
            }
            if self.tg.client.is_some() {
                let mut on = control::running();
                let sub = if on { "работает, Telegram Desktop идёт через него" } else { "ускоряет Telegram Desktop" };
                if theme::setting_row(ui, false, "Telegram-прокси", sub, "", &mut on, true) {
                    act = Some(Act::Telegram(on));
                }
            }
            ui.add_space(6.0);
            theme::row(ui, 40.0, |ui| {
                if ui.add(theme::Primary("Открыть HolyZapret".into())).clicked() {
                    act = Some(Act::Open);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::button(ui, "Выход").clicked() {
                        act = Some(Act::Quit);
                    }
                });
            });
        });
        let Some(act) = act else { return };
        match act {
            Act::PowerOff => self.fly.confirm_off = true,
            Act::Power => {
                self.fly.confirm_off = false;
                if foreign {
                    self.install_general(ctx);
                } else {
                    match state {
                        Some(ServiceState::Running) => {
                            self.spawn(ctx, |_, tx| match crate::core::service::remove() {
                                Ok(m) => super::notify(tx, Lvl::Ok, m),
                                Err(m) => super::notify(tx, Lvl::Err, m),
                            });
                        }
                        Some(s) => self.power(ctx, s),
                        None => {}
                    }
                }
            }
            Act::Game(on) => {
                if let Some(s) = &status {
                    let mut gf = s.game.clone();
                    gf.mode = if on { gamefilter::Mode::All } else { gamefilter::Mode::Disabled };
                    self.save_game_filter(ctx, gf);
                }
            }
            Act::Warp(on) => self.set_warp_fallback(ctx, on),
            Act::Telegram(on) => {
                match control::enable(on) {
                    Ok(m) => self.toast(ctx, Lvl::Ok, m),
                    Err(e) => self.toast(ctx, Lvl::Err, format!("Telegram-прокси не запустился: {e}")),
                }
                self.tg_reload();
            }
            Act::Open => tray::show_window(),
            Act::Devices => {
                self.tab = super::Tab::Phone;
                tray::show_window();
            }
            Act::Quit => {
                if crate::core::lanproxy::control::running() || self.working() {
                    tray::show_window();
                    if crate::core::lanproxy::control::running() {
                        self.confirm = Some(super::confirm::Confirm::QuitSharing);
                    } else {
                        self.request_quit(ctx);
                    }
                } else {
                    tray::exit();
                }
            }
        }
    }

    fn menu_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(act) = self.menu_body(ui) else { return };
        if matches!(act, Act::Power | Act::Telegram(_)) {
            self.fly_close();
        }
        self.menu_act(ctx, act);
    }

    pub(super) fn popup_update(&mut self, ctx: &egui::Context) {
        if let Some(at) = tray::take_pop() {
            let size = Vec2::new(MENU_WIDTH, menu_height());
            self.pop = place(ctx, at, size, true).map(|p| Pop { pos: p, ready: false, seen_focus: false, frames: 0 });
        }
        let Some(mut pop) = self.pop.take() else { return };
        let size = Vec2::new(MENU_WIDTH, menu_height());
        let builder = egui::ViewportBuilder::default()
            .with_title(POP_TITLE)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(true)
            .with_inner_size(size)
            .with_position(pop.pos);
        let mut act = None;
        let mut close = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("tray-menu"), builder, |ui, _| {
            let pctx = ui.ctx().clone();
            theme::backdrop(&pctx.layer_painter(egui::LayerId::background()), pctx.content_rect());
            act = self.menu_body(ui);
            pop.frames = pop.frames.saturating_add(1);
            if !pop.ready && pop.frames >= 2 {
                pop.ready = tray::popup_ready(POP_TITLE);
            }
            let focused = pctx.input(|i| i.viewport().focused.unwrap_or(false));
            if focused {
                pop.seen_focus = true;
            }
            let esc = pctx.input(|i| i.key_pressed(egui::Key::Escape));
            if esc || (pop.seen_focus && !focused) || pop.frames > 120 && !pop.seen_focus || pctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            pctx.request_repaint_after(std::time::Duration::from_millis(100));
        });
        if act.is_some() || close {
            tray::pop_closed();
            ctx.request_repaint();
        } else {
            self.pop = Some(pop);
        }
        if let Some(act) = act {
            self.menu_act(ctx, act);
        }
    }

    fn menu_body(&mut self, ui: &mut egui::Ui) -> Option<Act> {
        let mut act = None;
        let status = self.status.clone();
        let foreign = status.as_ref().is_some_and(|s| s.foreign_dir().is_some());
        let state = status.as_ref().map(|s| s.service.clone());
        let running = state == Some(ServiceState::Running) && !foreign;
        let view = status.as_ref().map(tray::View::from_status);
        let busy = self.busy || tray::busy();
        let tg = control::running();
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin::same(8))).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            theme::row(ui, 40.0, |ui| {
                ui.add_space(10.0);
                let on = view.as_ref().is_some_and(|v| v.on);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, if on { GREEN } else { Color32::from_gray(120) });
                ui.add_space(4.0);
                let label = view.as_ref().map_or("Читаю состояние…".to_string(), |v| v.label.clone());
                ui.add(egui::Label::new(muted(&label).size(13.0)).truncate());
            });
            separator(ui);
            if menu_item(ui, theme::icon::POWER, if running { "Выключить защиту" } else { "Включить защиту" }, !busy && self.paths.is_some(), None) {
                act = Some(Act::Power);
            }
            if menu_item(ui, theme::icon::TELEGRAM, if tg { "Выключить Telegram-прокси" } else { "Включить Telegram-прокси" }, true, None) {
                act = Some(Act::Telegram(!tg));
            }
            if menu_item(ui, theme::icon::DEVICES, "Устройства…", true, None) {
                act = Some(Act::Devices);
            }
            if menu_item(ui, theme::icon::HOME, "Открыть HolyZapret", true, None) {
                act = Some(Act::Open);
            }
            separator(ui);
            if menu_item(ui, theme::icon::EJECT, "Выход", true, Some(RED)) {
                act = Some(Act::Quit);
            }
        });
        act
    }

    fn menu_act(&mut self, ctx: &egui::Context, act: Act) {
        let status = self.status.clone();
        let foreign = status.as_ref().is_some_and(|s| s.foreign_dir().is_some());
        let state = status.as_ref().map(|s| s.service.clone());
        match act {
            Act::Power => {
                match state {
                    _ if foreign => self.install_general(ctx),
                    Some(ServiceState::Running) => self.spawn(ctx, |_, tx| match crate::core::service::remove() {
                        Ok(m) => super::notify(tx, Lvl::Ok, m),
                        Err(m) => super::notify(tx, Lvl::Err, m),
                    }),
                    Some(s) => self.power(ctx, s),
                    None => {}
                }
            }
            Act::Telegram(on) => {
                match control::enable(on) {
                    Ok(m) => self.toast(ctx, Lvl::Ok, m),
                    Err(e) => self.toast(ctx, Lvl::Err, format!("Telegram-прокси не запустился: {e}")),
                }
                self.tg_reload();
            }
            Act::Devices => {
                self.tab = super::Tab::Phone;
                tray::show_window();
            }
            Act::Open => tray::show_window(),
            Act::Quit => {
                if crate::core::lanproxy::control::running() || self.working() {
                    tray::show_window();
                    if crate::core::lanproxy::control::running() {
                        self.confirm = Some(super::confirm::Confirm::QuitSharing);
                    } else {
                        self.request_quit(ctx);
                    }
                } else {
                    tray::exit();
                }
            }
            _ => {}
        }
    }
}

fn separator(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), egui::Sense::hover());
    ui.painter().hline(r.x_range().shrink(8.0), r.center().y, egui::Stroke::new(1.0, Color32::from_white_alpha(14)));
}

fn menu_item(ui: &mut egui::Ui, glyph: &str, text: &str, enabled: bool, color: Option<Color32>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), MENU_ROW), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), enabled && resp.hovered());
    if hover > 0.01 {
        theme::glass(ui.painter(), rect.shrink2(Vec2::new(0.0, 2.0)), 10.0, hover);
    }
    let base = color.unwrap_or(Color32::from_gray(232));
    let col = if enabled { base } else { Color32::from_gray(110) };
    theme::paint_icon(ui.painter(), egui::pos2(rect.left() + 20.0, rect.center().y), glyph, 15.0, if enabled { color.unwrap_or(Color32::from_gray(190)) } else { col });
    ui.painter().text(egui::pos2(rect.left() + 40.0, rect.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(14.0), col);
    if enabled {
        resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}
