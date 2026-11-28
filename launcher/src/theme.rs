use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response, RichText, Sense,
    Stroke, TextStyle, Ui, Vec2, Widget,
};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

pub const PINK: Color32 = Color32::from_rgb(242, 143, 201);
pub const CYAN: Color32 = Color32::from_rgb(105, 205, 245);
pub const GREEN: Color32 = Color32::from_rgb(98, 214, 142);
pub const RED: Color32 = Color32::from_rgb(242, 112, 122);
pub const YELLOW: Color32 = Color32::from_rgb(240, 196, 90);
pub const ORANGE: Color32 = Color32::from_rgb(245, 150, 80);

pub const CARD_RADIUS: u8 = 18;
pub const WIDGET_RADIUS: u8 = 12;

pub fn card_fill() -> Color32 {
    Color32::from_white_alpha(15)
}

pub fn card_stroke() -> Color32 {
    Color32::from_white_alpha(9)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn acrylic_window() -> bool {
    static CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| crate::core::picker::windows_build() >= 22621)
}

const RUBIK: &[u8] = include_bytes!("../assets/fonts/Rubik-Regular.ttf");
const RUBIK_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Rubik-SemiBold.ttf");

fn load_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    for (key, file, front) in [
        ("segoe", "segoeui.ttf", true),
        ("segoe_symbol", "seguisym.ttf", false),
        ("segoe_emoji", "seguiemj.ttf", false),
    ] {
        let path = std::path::Path::new(&windir).join("Fonts").join(file);
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(key.into(), Arc::new(FontData::from_owned(bytes)));
            for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                let list = fonts.families.entry(fam).or_default();
                if front {
                    list.insert(0, key.into());
                } else {
                    list.push(key.into());
                }
            }
        }
    }
    fonts.font_data.insert("rubik".into(), Arc::new(FontData::from_static(RUBIK)));
    fonts.font_data.insert("rubik_semibold".into(), Arc::new(FontData::from_static(RUBIK_SEMIBOLD)));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "rubik".into());
    let regular = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let semi = std::fs::read(std::path::Path::new(&windir).join("Fonts").join("seguisb.ttf"));
    let mut semi_list = regular;
    if let Ok(bytes) = semi {
        fonts.font_data.insert("segoe_semibold".into(), Arc::new(FontData::from_owned(bytes)));
        semi_list.insert(0, "segoe_semibold".into());
    }
    semi_list.insert(0, "rubik_semibold".into());
    fonts.families.insert(FontFamily::Name("semibold".into()), semi_list);
    let icons = FontFamily::Name("icons".into());
    let icon_file = ["SegoeIcons.ttf", "segmdl2.ttf"]
        .iter()
        .find_map(|f| std::fs::read(std::path::Path::new(&windir).join("Fonts").join(f)).ok());
    if let Some(bytes) = icon_file {
        fonts.font_data.insert("icons".into(), Arc::new(FontData::from_owned(bytes)));
        fonts.families.insert(icons, vec!["icons".into()]);
    } else {
        let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        fonts.families.insert(icons, fallback);
    }
    ctx.set_fonts(fonts);
}

pub mod icon {
    pub const HOME: &str = "\u{E80F}";
    pub const GLOBE: &str = "\u{E774}";
    pub const DIAGNOSTIC: &str = "\u{E9D9}";
    pub const PICK: &str = "\u{E945}";
    pub const EJECT: &str = "\u{E711}";
    pub const WARNING: &str = "\u{E7BA}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const CLOUD: &str = "\u{E753}";
    pub const POWER: &str = "\u{E7E8}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const COPY: &str = "\u{E8C8}";
    pub const DELETE: &str = "\u{E74D}";
    pub const CHECK: &str = "\u{E73E}";
    pub const ERROR: &str = "\u{E783}";
    pub const INFO: &str = "\u{E946}";
    pub const TELEGRAM: &str = "tg-plane";
    pub const PHONE: &str = "\u{E8EA}";
    pub const DEVICES: &str = "\u{E772}";
    pub const TV: &str = "\u{E7F4}";
    pub const GAME: &str = "\u{E7FC}";
}

pub fn paint_icon(painter: &egui::Painter, center: egui::Pos2, glyph: &str, size: f32, color: Color32) {
    if glyph != icon::TELEGRAM {
        painter.text(center, egui::Align2::CENTER_CENTER, glyph, icon_font(size), color);
        return;
    }
    let s = size * 0.5;
    let p = |x: f32, y: f32| center + Vec2::new(x * s, y * s);
    let stroke = Stroke::new((size / 11.0).max(1.2), color);
    let body = vec![p(-0.95, 0.02), p(0.92, -0.74), p(0.6, 0.78), p(0.02, 0.36)];
    painter.add(egui::Shape::closed_line(body, stroke));
    painter.line_segment([p(0.02, 0.36), p(0.62, -0.42)], stroke);
    painter.add(egui::Shape::line(vec![p(0.02, 0.36), p(-0.12, 0.8), p(0.2, 0.52)], stroke));
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = Color32::from_rgba_unmultiplied(30, 27, 38, 240);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.window_stroke = Stroke::new(1.0, Color32::from_white_alpha(16));
    v.window_shadow = egui::epaint::Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(90) };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(70) };
    v.menu_corner_radius = CornerRadius::same(WIDGET_RADIUS);
    v.extreme_bg_color = Color32::from_white_alpha(12);
    v.faint_bg_color = Color32::from_white_alpha(6);
    v.selection.bg_fill = PINK.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, PINK.gamma_multiply(0.7));
    v.hyperlink_color = CYAN;
    v.disabled_alpha = 0.45;
    let r = CornerRadius::same(WIDGET_RADIUS);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, card_stroke());
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_gray(150));
    v.widgets.inactive.bg_fill = Color32::from_white_alpha(20);
    v.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(20);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(16));
    v.widgets.hovered.bg_fill = Color32::from_white_alpha(34);
    v.widgets.hovered.weak_bg_fill = Color32::from_white_alpha(34);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(34));
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = PINK.gamma_multiply(0.35);
    v.widgets.active.weak_bg_fill = PINK.gamma_multiply(0.35);
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.bg_stroke = Stroke::NONE;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(10.0, 10.0);
        s.spacing.button_padding = Vec2::new(16.0, 8.0);
        s.spacing.scroll.bar_width = 6.0;
        s.text_styles.insert(TextStyle::Heading, semibold(24.0));
        s.text_styles.insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace));
    });
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let shown = egui::Frame::new()
        .fill(card_fill())
        .stroke(Stroke::new(1.0, card_stroke()))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), add).inner
        });
    sheen(ui.painter(), shown.response.rect, CARD_RADIUS as f32);
    shown.inner
}

pub fn sheen(painter: &egui::Painter, rect: egui::Rect, radius: f32) {
    let top = rect.top() + 0.5;
    let inset = radius * 0.9;
    let (l, r) = (rect.left() + inset, rect.right() - inset);
    if r <= l {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let edge = Color32::TRANSPARENT;
    let mid = Color32::from_white_alpha(26);
    let xs = [l, l + (r - l) * 0.3, l + (r - l) * 0.7, r];
    let cols = [edge, mid, mid, edge];
    for (x, c) in xs.iter().zip(cols) {
        mesh.colored_vertex(egui::pos2(*x, top - 0.5), c);
        mesh.colored_vertex(egui::pos2(*x, top + 0.5), c);
    }
    for i in 0..3u32 {
        let a = 2 * i;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub const FLOW_PERIOD: f64 = 40.0;
static FLOW: AtomicU32 = AtomicU32::new(0);

pub fn set_flow(phase: f32) {
    FLOW.store(phase.rem_euclid(1.0).to_bits(), Ordering::Relaxed);
}

static SCREEN_W: AtomicU32 = AtomicU32::new(0);

pub fn live(rect: egui::Rect) -> Color32 {
    let w = f32::from_bits(SCREEN_W.load(Ordering::Relaxed));
    let w = if w > 1.0 { w } else { 940.0 };
    grad((rect.left() / w).clamp(0.0, 1.0))
}

fn flow_phase() -> f32 {
    f32::from_bits(FLOW.load(Ordering::Relaxed))
}

pub fn flow_mix(t: f32) -> f32 {
    let u = (t.clamp(0.0, 1.0) * 0.5 + f32::from_bits(FLOW.load(Ordering::Relaxed))).rem_euclid(1.0);
    1.0 - (1.0 - 2.0 * u).abs()
}

pub fn animations_allowed() -> bool {
    static CACHE: Mutex<Option<(std::time::Instant, bool)>> = Mutex::new(None);
    let mut g = CACHE.lock().unwrap();
    if let Some((at, v)) = *g {
        if at.elapsed().as_secs() < 5 {
            return v;
        }
    }
    let mut on: i32 = 1;
    let ok = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows_sys::Win32::UI::WindowsAndMessaging::SPI_GETCLIENTAREAANIMATION,
            0,
            &mut on as *mut i32 as *mut _,
            0,
        )
    };
    let mut power: windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    let saver = unsafe { windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut power) } != 0 && power.SystemStatusFlag == 1;
    let v = (ok == 0 || on != 0) && !saver;
    *g = Some((std::time::Instant::now(), v));
    v
}

pub fn gradient_rect(painter: &egui::Painter, rect: egui::Rect, radius: f32, c0: Color32, c1: Color32) {
    let r = radius.min(rect.height() / 2.0).min(rect.width() / 2.0).max(0.5);
    let quarter = std::f32::consts::FRAC_PI_2;
    let steps = (r * 1.5).ceil().clamp(4.0, 32.0) as usize;
    let color_at = |x: f32| c0.lerp_to_gamma(c1, flow_mix((x - rect.left()) / rect.width()));
    let inset = |x: f32| {
        let dx = (rect.left() + r - x).max(x - (rect.right() - r)).max(0.0);
        r - (r * r - dx * dx).max(0.0).sqrt()
    };
    let mut xs = Vec::new();
    for i in 0..=steps {
        xs.push(rect.left() + r - r * (quarter * i as f32 / steps as f32).cos());
    }
    let span = rect.width() - 2.0 * r;
    let mid = (span / 6.0).ceil().max(1.0) as usize;
    for i in 1..mid {
        xs.push(rect.left() + r + span * i as f32 / mid as f32);
    }
    for i in 0..=steps {
        xs.push(rect.right() - r + r * (quarter * i as f32 / steps as f32).sin());
    }
    let mut mesh = egui::Mesh::default();
    for &x in &xs {
        let d = inset(x);
        let col = color_at(x);
        mesh.colored_vertex(egui::pos2(x, rect.top() + d), col);
        mesh.colored_vertex(egui::pos2(x, rect.bottom() - d), col);
    }
    for i in 0..xs.len() as u32 - 1 {
        let a = 2 * i;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    let outline: Vec<egui::Pos2> = xs
        .iter()
        .map(|&x| egui::pos2(x, rect.top() + inset(x)))
        .chain(xs.iter().rev().map(|&x| egui::pos2(x, rect.bottom() - inset(x))))
        .collect();
    let feather = 1.0 / painter.ctx().pixels_per_point();
    let base = mesh.vertices.len() as u32;
    for p in &outline {
        let c = egui::pos2(p.x.clamp(rect.left() + r, rect.right() - r), p.y.clamp(rect.top() + r, rect.bottom() - r));
        let n = (*p - c).normalized();
        let inner = *p - n * (feather / 2.0);
        mesh.colored_vertex(inner, color_at(inner.x));
        mesh.colored_vertex(*p + n * (feather / 2.0), Color32::TRANSPARENT);
    }
    let n = outline.len() as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        let (ia, oa, ib, ob) = (base + 2 * i, base + 2 * i + 1, base + 2 * j, base + 2 * j + 1);
        mesh.add_triangle(ia, oa, ib);
        mesh.add_triangle(ib, oa, ob);
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub fn gradient_text(text: &str, size: f32, c0: Color32, c1: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let count = text.chars().count().max(2) - 1;
    for (i, ch) in text.chars().enumerate() {
        let fmt = egui::TextFormat {
            font_id: FontId::new(size, FontFamily::Proportional),
            color: c0.lerp_to_gamma(c1, flow_mix(i as f32 / count as f32)),
            ..Default::default()
        };
        job.append(&ch.to_string(), 0.0, fmt);
    }
    job
}

fn add_light(a: Color32, b: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(
        a.r().saturating_add(b.r()),
        a.g().saturating_add(b.g()),
        a.b().saturating_add(b.b()),
        a.a().saturating_add(b.a()),
    )
}

fn corner_glow(rect: egui::Rect, from: egui::Pos2, reach: f32, pos: egui::Pos2) -> f32 {
    let d = ((pos - from).length() / reach.max(1.0)).clamp(0.0, 1.0);
    let g = 1.0 - d;
    let _ = rect;
    g * g * (3.0 - 2.0 * g)
}

pub fn tinted_glass(painter: &egui::Painter, rect: egui::Rect, radius: f32, tint: f32, rim: f32) {
    tinted_glass_at(painter, rect, radius, tint, rim, egui::pos2(rect.left() + radius * 0.4, rect.top()));
}

pub fn tinted_glass_at(painter: &egui::Painter, rect: egui::Rect, radius: f32, tint: f32, rim: f32, light: egui::Pos2) {
    painter.rect_filled(rect, CornerRadius::same(radius as u8), Color32::from_white_alpha(10));
    let reach = rect.width().max(rect.height() * 2.0) * 0.62;
    let back = egui::pos2(rect.right() - radius * 0.4, rect.bottom());
    let color = move |pos: egui::Pos2| {
        let warm = corner_glow(rect, light, reach, pos);
        let cool = corner_glow(rect, back, reach * 0.8, pos);
        add_light(PINK.gamma_multiply(0.55 * warm * tint), CYAN.gamma_multiply(0.22 * cool * tint))
    };
    if tint > 0.005 {
        let base = rect.shrink(0.6);
        let c = base.center();
        let outline = rounded_outline(base, (radius - 0.6).max(1.0));
        let rings = 6;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(c, color(c));
        for k in 1..=rings {
            let f = k as f32 / rings as f32;
            for p in &outline {
                let q = c + (*p - c) * f;
                mesh.colored_vertex(q, color(q));
            }
        }
        let n = outline.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        for k in 0..(rings as u32 - 1) {
            let a = 1 + k * n;
            let b = a + n;
            for i in 0..n {
                let j = (i + 1) % n;
                mesh.add_triangle(a + i, b + i, a + j);
                mesh.add_triangle(a + j, b + i, b + j);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    let top = rect.top();
    let h = rect.height().max(1.0);
    let stroke = egui::epaint::PathStroke::new_uv(1.0, move |_, pos| {
        let fy = ((pos.y - top) / h).clamp(0.0, 1.0);
        let warm = corner_glow(rect, light, reach * 0.9, pos);
        let edge = Color32::from_white_alpha(((40.0 - 28.0 * fy) * rim) as u8);
        let lit = PINK.lerp_to_gamma(Color32::WHITE, 0.35).gamma_multiply(0.95 * warm * tint.max(0.25) * rim);
        add_light(edge, lit)
    });
    painter.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(rounded_outline(rect.shrink(0.5), radius - 0.5), stroke)));
    let line = egui::Rect::from_min_max(egui::pos2(rect.left() + radius * 0.7, rect.top() + 1.5), egui::pos2(rect.right() - radius * 0.7, rect.top() + 2.5));
    if line.width() > 4.0 {
        painter.rect_filled(line, CornerRadius::same(1), Color32::from_white_alpha((12.0 * rim) as u8));
    }
}

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(42.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
    let dim = if ui.is_enabled() { 1.0 } else { 0.5 };
    if crate::glass::ok() {
        let press = ui.ctx().animate_bool_with_time(resp.id.with("press"), resp.is_pointer_button_down_on(), 0.18);
        let mut track = crate::glass::Look::clear(11.0);
        track.amount = (0.06 + 0.04 * hover) * dim;
        track.refract = 1.5;
        track.blur = 1.0;
        track.rim = (0.6 + 0.3 * hover) * dim;
        track.shadow = 0.0;
        track.glow = 0.5;
        track.color = live(rect);
        track.warm = (0.5 + 0.12 * hover) * t * dim;
        crate::glass::paint(ui.painter(), rect, track);
        let w = 16.0 + 12.0 * press;
        let h = 16.0 + 6.0 * press;
        let cx = rect.left() + 11.0 + t * (rect.width() - 22.0);
        let knob = egui::Rect::from_center_size(egui::pos2(cx, rect.center().y), Vec2::new(w, h));
        let mut look = crate::glass::Look::clear(h / 2.0);
        look.amount = (0.92 * (1.0 - press) + 0.05) * (0.55 + 0.45 * t.max(0.6)) * dim.max(0.7);
        look.rim = 0.6 + 0.8 * press;
        look.refract = 2.0 + 4.0 * press;
        look.blur = 0.4;
        look.zoom = 1.0 - 0.25 * press;
        look.shadow = 0.18;
        look.glow = 0.3 + 0.6 * press;
        crate::glass::paint(ui.painter(), knob, look);
        return resp;
    }
    let cx = rect.left() + 11.0 + t * (rect.width() - 22.0);
    let c = egui::pos2(cx, rect.center().y);
    tinted_glass_at(ui.painter(), rect, 11.0, (1.1 + 0.3 * hover) * t * dim, (0.8 + 0.5 * hover) * dim, c);
    ui.painter().circle_filled(c + Vec2::new(0.0, 1.0), 8.5, Color32::from_black_alpha(70));
    let knob = Color32::from_gray(190).lerp_to_gamma(Color32::WHITE, t);
    ui.painter().circle_filled(c, 7.5, knob.gamma_multiply(dim.max(0.7)));
    resp
}

pub struct Primary(pub String);

impl Widget for Primary {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let galley = ui.painter().layout_no_wrap(self.0, semibold(15.0), Color32::WHITE);
        let size = galley.size() + Vec2::new(40.0, 20.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let radius = rect.height() / 2.0;
        let pos = rect.center() - galley.size() / 2.0;
        if !enabled {
            match soft_glass(rect, radius, 0.5, 0.0) {
                Some(s) => {
                    ui.painter().add(s);
                }
                None => ui.painter().extend(glass_shapes(rect, radius, 0.5, true)),
            }
            ui.painter().galley_with_override_text_color(pos, galley, Color32::from_gray(120));
            return resp;
        }
        let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
        let press = if resp.is_pointer_button_down_on() { 0.2 } else { 0.0 };
        if crate::glass::ok() {
            let mut look = crate::glass::Look::clear(radius);
            look.amount = 0.07 + 0.04 * hover + 0.05 * press;
            look.refract = 2.0;
            look.blur = 1.2;
            look.rim = 0.75 + 0.25 * hover;
            look.shadow = 0.0;
            look.glow = 0.6;
            look.color = live(rect);
            look.warm = 0.42 + 0.12 * hover + 0.1 * press;
            crate::glass::paint(ui.painter(), rect, look);
            ui.painter().galley(pos, galley, Color32::WHITE);
            return resp;
        }
        let shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: PINK.gamma_multiply(0.08 + 0.08 * hover),
        };
        ui.painter().add(shadow.as_shape(rect, CornerRadius::same(radius as u8)));
        tinted_glass(ui.painter(), rect, radius, 1.0 + 0.35 * hover + press, 1.2 + 0.4 * hover);
        let shade = Color32::from_black_alpha(90);
        ui.painter().galley_with_override_text_color(pos + Vec2::new(0.0, 1.0), galley.clone(), shade);
        ui.painter().galley(pos, galley, Color32::WHITE);
        resp
    }
}

pub fn nav_item(ui: &mut Ui, selected: bool, glyph: &str, label: &str, expand: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    let resp = if expand < 0.5 { resp.on_hover_text(label) } else { resp };
    let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
    if selected {
        glass(ui.painter(), rect, WIDGET_RADIUS as f32 + 2.0, 1.0);
    } else if hover > 0.0 {
        ui.painter().rect_filled(rect, CornerRadius::same(WIDGET_RADIUS + 2), Color32::from_white_alpha((12.0 * hover) as u8));
    }
    if selected {
        let bar = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 3.0, rect.center().y),
            Vec2::new(3.0, 18.0),
        );
        ui.painter().rect_filled(bar, CornerRadius::same(2), PINK);
    }
    let color = if selected { Color32::WHITE } else { Color32::from_gray(190) };
    let painter = ui.painter().with_clip_rect(rect);
    paint_icon(&painter, egui::pos2(rect.left() + 20.0, rect.center().y), glyph, 16.0, if selected { PINK } else { color });
    if expand > 0.05 {
        painter.text(
            egui::pos2(rect.left() + 44.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::new(15.0, FontFamily::Proportional),
            color.gamma_multiply(expand),
        );
    }
    resp
}

pub fn chrome_button(ui: &mut Ui, danger: bool) -> (Response, egui::Rect) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(46.0, 36.0), Sense::click());
    if resp.hovered() {
        let fill = if danger { Color32::from_rgb(196, 43, 28) } else { Color32::from_white_alpha(22) };
        ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    }
    (resp, rect)
}

pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.17))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(Margin::symmetric(11, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(13.0));
        })
        .response
}

pub fn backdrop(painter: &egui::Painter, rect: egui::Rect) {
    let k = if acrylic_window() { 0.25 } else { 1.0 };
    let alpha = |a: f32| (a * k).round() as u8;
    let a = Color32::from_rgba_unmultiplied(30, 16, 36, alpha(175.0));
    let b = Color32::from_rgba_unmultiplied(14, 12, 22, alpha(185.0));
    let c = Color32::from_rgba_unmultiplied(8, 10, 18, alpha(200.0));
    let d = Color32::from_rgba_unmultiplied(16, 10, 24, alpha(190.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), a);
    mesh.colored_vertex(rect.right_top(), b);
    mesh.colored_vertex(rect.right_bottom(), c);
    mesh.colored_vertex(rect.left_bottom(), d);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    SCREEN_W.store(rect.width().to_bits(), Ordering::Relaxed);
    let span = rect.width().max(rect.height());
    let a = flow_phase() * std::f32::consts::TAU;
    let drift = Vec2::new(a.cos(), a.sin()) * span * 0.04;
    glow(painter, rect.left_top() + Vec2::new(rect.width() * 0.12, rect.height() * 0.05) + drift, span * 0.55, grad(0.0).gamma_multiply(0.10));
    glow(painter, rect.right_bottom() - Vec2::new(rect.width() * 0.08, rect.height() * 0.1) - drift, span * 0.6, grad(1.0).gamma_multiply(0.09));
}

fn glow(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let rings = [(0.0, 1.0), (0.35, 0.7), (0.65, 0.32), (1.0, 0.0)];
    let segs = 48u32;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for &(r, f) in &rings[1..] {
        for i in 0..segs {
            let a = std::f32::consts::TAU * i as f32 / segs as f32;
            mesh.colored_vertex(center + Vec2::angled(a) * radius * r, color.gamma_multiply(f));
        }
    }
    for i in 0..segs {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % segs);
    }
    for ring in 0..(rings.len() as u32 - 2) {
        let inner = 1 + ring * segs;
        let outer = inner + segs;
        for i in 0..segs {
            let j = (i + 1) % segs;
            mesh.add_triangle(inner + i, outer + i, inner + j);
            mesh.add_triangle(inner + j, outer + i, outer + j);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub fn grad(t: f32) -> Color32 {
    PINK.lerp_to_gamma(CYAN, flow_mix(t))
}

pub fn glass_disc(painter: &egui::Painter, c: egui::Pos2, r: f32, strength: f32, warm: f32, fallback: u8) {
    let rect = egui::Rect::from_center_size(c, Vec2::splat((r - 2.0) * 2.0));
    if crate::glass::ok() {
        let mut look = crate::glass::Look::clear(r - 2.0);
        look.amount = 0.05 * strength.clamp(0.0, 1.6);
        look.refract = 1.5;
        look.blur = 1.0;
        look.rim = 0.0;
        look.shadow = 0.0;
        look.glow = 0.0;
        look.color = live(rect);
        look.warm = warm;
        painter.add(crate::glass::shape(rect, look));
        return;
    }
    painter.circle_filled(c, r, Color32::from_white_alpha(fallback));
}

pub fn power_button(ui: &mut Ui, on: bool, busy: bool) -> Response {
    power_button_at(ui, on, busy, 0.0)
}

pub fn power_button_at(ui: &mut Ui, on: bool, busy: bool, dy: f32) -> Response {
    let d = 92.0;
    let (slot, place) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
    let rect = slot.translate(Vec2::new(0.0, dy));
    let resp = ui.interact(rect, place.id.with("power"), Sense::click());
    let c = rect.center();
    let r = d / 2.0 - 8.0;
    let t = ui.ctx().animate_bool_responsive(resp.id.with("on"), on);
    let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
    let p = ui.painter();
    if t > 0.01 {
        for (k, a) in [(14.0, 0.03), (9.0, 0.06), (5.0, 0.10)] {
            p.circle_filled(c, r + k, grad(0.5).gamma_multiply(a * t));
        }
    }
    glass_disc(p, c, r, 1.0 + 0.4 * hover, 0.16 + 0.12 * hover + 0.14 * t, (10.0 + 8.0 * hover) as u8);
    let time = ui.input(|i| i.time) as f32;
    let spin = if busy { time * 4.0 } else { 0.0 };
    let tau = std::f32::consts::TAU;
    let points: Vec<egui::Pos2> = (0..96).map(|i| c + Vec2::angled(tau * i as f32 / 96.0) * r).collect();
    let rest = 0.32 + 0.12 * hover;
    let stroke = egui::epaint::PathStroke::new_uv(3.5, move |_, pos| {
        let v = pos - c;
        let f = ((v.y.atan2(v.x) + tau / 4.0 - spin).rem_euclid(tau)) / tau;
        let lit = grad(if f < 0.5 { f * 2.0 } else { 2.0 - f * 2.0 });
        if busy {
            let k = if f > 0.7 { (f - 0.7) / 0.3 } else { 0.0 };
            return Color32::from_white_alpha(30).lerp_to_gamma(lit, k);
        }
        lit.gamma_multiply(rest + (1.0 - rest) * t)
    });
    p.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(points, stroke)));
    let glyph = Color32::from_gray(170).lerp_to_gamma(Color32::WHITE, t.max(hover));
    p.text(c, egui::Align2::CENTER_CENTER, icon::POWER, icon_font(28.0), glyph);
    if busy || (t > 0.0 && t < 1.0) || (hover > 0.0 && hover < 1.0) {
        ui.ctx().request_repaint();
    }
    resp
}

pub fn emblem(ui: &mut Ui, on: bool, glyph: &str, d: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
    let c = rect.center();
    let r = d / 2.0 - 6.0;
    let t = ui.ctx().animate_bool_responsive(resp.id.with("on"), on);
    let p = ui.painter();
    if t > 0.01 {
        for (k, a) in [(11.0, 0.03), (7.0, 0.06), (4.0, 0.10)] {
            p.circle_filled(c, r + k, grad(0.5).gamma_multiply(a * t));
        }
    }
    glass_disc(p, c, r, 1.0, 0.14 + 0.14 * t, 10);
    let tau = std::f32::consts::TAU;
    let points: Vec<egui::Pos2> = (0..96).map(|i| c + Vec2::angled(tau * i as f32 / 96.0) * r).collect();
    let stroke = egui::epaint::PathStroke::new_uv(3.0, move |_, pos| {
        let v = pos - c;
        let f = ((v.y.atan2(v.x) + tau / 4.0).rem_euclid(tau)) / tau;
        let lit = grad(if f < 0.5 { f * 2.0 } else { 2.0 - f * 2.0 });
        lit.gamma_multiply(0.32 + 0.68 * t)
    });
    p.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(points, stroke)));
    let col = Color32::from_gray(160).lerp_to_gamma(Color32::WHITE, t);
    paint_icon(p, c, glyph, d * 0.32, col);
    if t > 0.0 && t < 1.0 {
        ui.ctx().request_repaint();
    }
    resp
}

pub fn setting_row(ui: &mut Ui, first: bool, title: &str, sub: &str, tip: &str, on: &mut bool, enabled: bool) -> bool {
    if !first {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    let mut changed = false;
    let resp = ui
        .allocate_ui_with_layout(Vec2::new(ui.available_width(), 50.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                changed = toggle(ui, on).changed();
            });
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(7.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(title).color(Color32::from_gray(232)).size(14.5));
                ui.add(egui::Label::new(RichText::new(sub).color(Color32::from_gray(165)).size(12.5)).truncate().show_tooltip_when_elided(tip.is_empty()));
            });
        })
        .response;
    if !tip.is_empty() {
        resp.on_hover_text(tip);
    }
    changed
}

pub fn fold_row(ui: &mut Ui, first: bool, title: &str, open: &mut bool) {
    if !first {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
    if hover > 0.0 {
        ui.painter().rect_filled(rect.expand2(Vec2::new(6.0, -2.0)), CornerRadius::same(10), Color32::from_white_alpha((8.0 * hover) as u8));
    }
    let glyph = if *open { icon::CHEVRON_UP } else { icon::CHEVRON_DOWN };
    ui.painter().text(egui::pos2(rect.left() + 7.0, rect.center().y), egui::Align2::CENTER_CENTER, glyph, icon_font(10.0), Color32::from_gray(160));
    ui.painter().text(egui::pos2(rect.left() + 22.0, rect.center().y), egui::Align2::LEFT_CENTER, title, FontId::proportional(14.0), Color32::from_gray(225));
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        *open = !*open;
    }
}

pub fn soft_note(ui: &mut Ui, color: Color32, text: &str, action: Option<(&str, bool)>) -> bool {
    let mut clicked = false;
    egui::Frame::new().fill(color.gamma_multiply(0.10)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some((label, enabled)) = action {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clicked = ui.add_enabled(enabled, egui::Button::new(label)).clicked();
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).color(color).size(13.0)).wrap());
                    });
                });
            } else {
                ui.add(egui::Label::new(RichText::new(text).color(color).size(13.0)).wrap());
            }
        });
    });
    clicked
}

fn rounded_outline(rect: egui::Rect, radius: f32) -> Vec<egui::Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (egui::pos2(rect.right() - r, rect.top() + r), -std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (egui::pos2(rect.left() + r, rect.bottom() - r), std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
    ];
    let mut out = Vec::new();
    for (c, a0) in corners {
        for i in 0..=8 {
            let a = a0 + std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
            out.push(c + Vec2::angled(a) * r);
        }
    }
    out
}

pub fn glass_shapes(rect: egui::Rect, radius: f32, strength: f32, fill: bool) -> Vec<egui::Shape> {
    let k = strength.clamp(0.0, 1.6);
    let mut out = Vec::new();
    if fill {
        out.push(egui::Shape::rect_filled(rect, CornerRadius::same(radius as u8), Color32::from_white_alpha((16.0 * k) as u8)));
    }
    let top = rect.top();
    let h = rect.height().max(1.0);
    let stroke = egui::epaint::PathStroke::new_uv(1.0, move |_, pos| {
        let f = ((pos.y - top) / h).clamp(0.0, 1.0);
        Color32::from_white_alpha(((46.0 - 36.0 * f) * k) as u8)
    });
    out.push(egui::Shape::Path(egui::epaint::PathShape::closed_line(rounded_outline(rect.shrink(0.5), radius - 0.5), stroke)));
    let inner = egui::Rect::from_min_max(egui::pos2(rect.left() + radius * 0.6, rect.top() + 1.5), egui::pos2(rect.right() - radius * 0.6, rect.top() + 2.5));
    if inner.width() > 4.0 {
        out.push(egui::Shape::rect_filled(inner, CornerRadius::same(1), Color32::from_white_alpha((10.0 * k) as u8)));
    }
    out
}

pub fn soft_glass(rect: egui::Rect, radius: f32, strength: f32, warm: f32) -> Option<egui::Shape> {
    soft_glass_with(rect, radius, strength, warm, live(rect))
}

pub fn soft_glass_with(rect: egui::Rect, radius: f32, strength: f32, warm: f32, color: Color32) -> Option<egui::Shape> {
    if !crate::glass::ok() {
        return None;
    }
    let k = strength.clamp(0.0, 1.6);
    let mut look = crate::glass::Look::clear(radius);
    look.amount = 0.05 * k;
    look.refract = 1.5;
    look.blur = 1.0;
    look.rim = 0.6 * k;
    look.shadow = 0.0;
    look.glow = 0.45 * k;
    look.color = color;
    look.warm = warm;
    Some(crate::glass::shape(rect, look))
}

pub fn glass(painter: &egui::Painter, rect: egui::Rect, radius: f32, strength: f32) {
    match soft_glass(rect, radius, strength, 0.36) {
        Some(s) => {
            painter.add(s);
        }
        None => painter.extend(glass_shapes(rect, radius, strength, true)),
    }
}

pub struct GlassButton {
    text: egui::WidgetText,
    radius: u8,
    min: Vec2,
}

impl GlassButton {
    pub fn new(text: impl Into<egui::WidgetText>) -> Self {
        GlassButton { text: text.into(), radius: WIDGET_RADIUS, min: Vec2::ZERO }
    }

    pub fn corner_radius(mut self, r: CornerRadius) -> Self {
        self.radius = r.nw;
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min = size;
        self
    }
}

impl Widget for GlassButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let slot = ui.painter().add(egui::Shape::Noop);
        let resp = ui.add(
            egui::Button::new(self.text)
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(self.radius))
                .min_size(self.min),
        );
        let hover = ui.ctx().animate_bool_responsive(resp.id.with("glass"), resp.hovered() && enabled);
        let press = if resp.is_pointer_button_down_on() { 0.3 } else { 0.0 };
        let k = if enabled { 1.0 + 0.45 * hover + press } else { 0.45 };
        let warm = if enabled { 0.06 + 0.2 * hover + 0.1 * press } else { 0.0 };
        let shape = soft_glass(resp.rect, self.radius as f32, k, warm)
            .unwrap_or_else(|| egui::Shape::Vec(glass_shapes(resp.rect, self.radius as f32, k, true)));
        ui.painter().set(slot, shape);
        resp
    }
}

pub fn button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(GlassButton::new(text))
}

pub fn icon_button(ui: &mut Ui, glyph: &str, tip: &str) -> Response {
    icon_button_at(ui, glyph, tip, 0.0)
}

pub fn icon_button_at(ui: &mut Ui, glyph: &str, tip: &str, dy: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let hover = ui.ctx().animate_bool_responsive(resp.id.with("glass"), resp.hovered());
    let press = if resp.is_pointer_button_down_on() { 0.4 } else { 0.0 };
    if hover > 0.01 {
        let disc = rect.translate(Vec2::new(0.0, dy));
        match soft_glass(disc, 15.0, (1.0 + press) * hover, (0.2 + press * 0.3) * hover) {
            Some(s) => {
                ui.painter().add(s);
            }
            None => {
                ui.painter().rect_filled(disc, CornerRadius::same(10), Color32::from_white_alpha((20.0 * hover) as u8));
            }
        }
    }
    let col = Color32::from_gray(185).lerp_to_gamma(Color32::WHITE, hover);
    ui.painter().text(rect.center() + Vec2::new(0.0, dy), egui::Align2::CENTER_CENTER, glyph, icon_font(14.0), col);
    resp.on_hover_text(tip)
}

pub fn solid_popup() -> egui::style::StyleModifier {
    egui::style::StyleModifier::new(|s| s.visuals.window_fill = Color32::from_rgb(30, 27, 38))
}

pub fn segmented(ui: &mut Ui, items: &[String], selected: &mut usize) -> bool {
    let mut changed = false;
    let font = FontId::new(13.5, FontFamily::Proportional);
    let need: f32 = items
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), Color32::WHITE).size().x + 20.0)
        .sum::<f32>()
        + 2.0 * items.len().saturating_sub(1) as f32
        + 8.0;
    if need > ui.available_width() {
        let current = items.get(*selected).cloned().unwrap_or_default();
        egui::ComboBox::from_id_salt(("segmented", items.first().cloned().unwrap_or_default()))
            .selected_text(current)
            .width(ui.available_width().clamp(120.0, 260.0))
            .show_ui(ui, |ui| {
                for (i, label) in items.iter().enumerate() {
                    if ui.selectable_label(*selected == i, label.as_str()).clicked() && *selected != i {
                        *selected = i;
                        changed = true;
                    }
                }
            });
        return changed;
    }
    let (outer, _) = ui.allocate_exact_size(Vec2::new(need, 34.0), Sense::hover());
    ui.painter().rect_filled(outer, CornerRadius::same(17), Color32::from_white_alpha(10));
    let mut x = outer.left() + 4.0;
    let mut cells = Vec::with_capacity(items.len());
    for label in items {
        let galley = ui.painter().layout_no_wrap(label.clone(), font.clone(), Color32::WHITE);
        let rect = egui::Rect::from_min_size(egui::pos2(x, outer.top() + 3.0), Vec2::new(galley.size().x + 20.0, 28.0));
        x = rect.right() + 2.0;
        cells.push((rect, galley));
    }
    let slide_id = ui.scope_id().with(("segmented_slide", items.first().map(String::as_str).unwrap_or("")));
    if let Some((target, _)) = cells.get(*selected) {
        let left = ui.ctx().animate_value_with_time(slide_id.with("l"), target.left(), 0.14);
        let right = ui.ctx().animate_value_with_time(slide_id.with("r"), target.right(), 0.14);
        let pill = egui::Rect::from_x_y_ranges(left..=right, target.y_range());
        glass(ui.painter(), pill, 14.0, 1.0);
    }
    for (i, (rect, galley)) in cells.into_iter().enumerate() {
        let resp = ui.interact(rect, ui.scope_id().with(("segmented", i, items[i].as_str())), Sense::click());
        let on = *selected == i;
        if !on && resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_white_alpha(10));
        }
        let col = if on { Color32::WHITE } else { Color32::from_gray(175) };
        ui.painter().galley(rect.center() - galley.size() / 2.0 - Vec2::new(0.0, 1.0), galley, col);
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}

pub fn paint_pill(painter: &egui::Painter, right_center: egui::Pos2, text: &str, color: Color32) -> egui::Rect {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::new(12.0, FontFamily::Proportional), color);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let rect = egui::Rect::from_min_size(egui::pos2(right_center.x - size.x, right_center.y - size.y / 2.0), size);
    painter.rect_filled(rect, CornerRadius::same(20), color.gamma_multiply(0.17));
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    rect
}

pub fn row<R>(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        add,
    )
    .inner
}

pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn row_label(ui: &mut Ui, text: &str, width: f32) {
    let w = ui.label(RichText::new(text).color(Color32::from_gray(170))).rect.width();
    if w < width {
        ui.add_space(width - w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_shifts_colors_along_gradient() {
        set_flow(0.0);
        assert!((flow_mix(0.0) - 0.0).abs() < 1e-6 && (flow_mix(1.0) - 1.0).abs() < 1e-6 && (flow_mix(0.3) - 0.3).abs() < 1e-6);
        set_flow(0.5);
        assert!((flow_mix(0.0) - 1.0).abs() < 1e-6 && (flow_mix(1.0) - 0.0).abs() < 1e-6);
        set_flow(0.25);
        assert!((flow_mix(0.5) - 1.0).abs() < 1e-6 && (flow_mix(0.0) - 0.5).abs() < 1e-6);
        set_flow(0.0);
        assert_eq!(ellipsize("abcdef", 4), "abc…");
    }
}
