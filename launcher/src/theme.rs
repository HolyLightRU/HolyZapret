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
pub const RED: Color32 = Color32::from_rgb(244, 100, 110);
pub const YELLOW: Color32 = Color32::from_rgb(240, 196, 90);

pub fn card_fill() -> Color32 {
    Color32::from_white_alpha(13)
}

pub fn card_stroke() -> Color32 {
    Color32::from_white_alpha(22)
}

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
    pub const TOOLS: &str = "\u{E90F}";
    pub const LOG: &str = "\u{E8FD}";
    pub const LISTS: &str = "\u{E8F1}";
    pub const PICK: &str = "\u{E945}";
    pub const EJECT: &str = "\u{E711}";
    pub const WARNING: &str = "\u{E7BA}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const CHAT: &str = "\u{E8BD}";
    pub const DOWNLOAD: &str = "\u{E896}";
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

pub fn icon_label(ui: &mut Ui, glyph: &str, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size + 2.0), Sense::hover());
    paint_icon(ui.painter(), rect.center(), glyph, size, color);
    resp
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = Color32::from_rgba_unmultiplied(32, 32, 36, 235);
    v.window_corner_radius = CornerRadius::same(12);
    v.window_stroke = Stroke::new(1.0, card_stroke());
    v.extreme_bg_color = Color32::from_white_alpha(10);
    v.faint_bg_color = Color32::from_white_alpha(6);
    v.selection.bg_fill = PINK.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, PINK);
    v.hyperlink_color = CYAN;
    let r = CornerRadius::same(8);
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
    v.widgets.inactive.bg_fill = Color32::from_white_alpha(16);
    v.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(16);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(14));
    v.widgets.hovered.bg_fill = Color32::from_white_alpha(30);
    v.widgets.hovered.weak_bg_fill = Color32::from_white_alpha(30);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, PINK.gamma_multiply(0.6));
    v.widgets.active.bg_fill = PINK.gamma_multiply(0.45);
    v.widgets.active.weak_bg_fill = PINK.gamma_multiply(0.45);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(10.0, 10.0);
        s.spacing.button_padding = Vec2::new(16.0, 8.0);
        s.spacing.scroll.bar_width = 6.0;
        s.text_styles.insert(TextStyle::Heading, FontId::new(24.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace));
    });
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(card_fill())
        .stroke(Stroke::new(1.0, card_stroke()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), add).inner
        })
        .inner
}

pub const FLOW_PERIOD: f64 = 40.0;
static FLOW: AtomicU32 = AtomicU32::new(0);

pub fn set_flow(phase: f32) {
    FLOW.store(phase.rem_euclid(1.0).to_bits(), Ordering::Relaxed);
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

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(42.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    ui.painter().rect_filled(rect, CornerRadius::same(11), Color32::from_white_alpha((28.0 * (1.0 - t)) as u8));
    if t > 0.01 {
        gradient_rect(ui.painter(), rect, 11.0, PINK.gamma_multiply(t), CYAN.gamma_multiply(t));
    }
    let cx = rect.left() + 11.0 + t * (rect.width() - 22.0);
    ui.painter().circle_filled(egui::pos2(cx, rect.center().y), 8.0, Color32::WHITE);
    resp
}

pub struct Primary(pub String);

impl Widget for Primary {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let dark = Color32::from_rgb(36, 18, 40);
        let galley = ui.painter().layout_no_wrap(self.0, FontId::new(15.0, FontFamily::Proportional), dark);
        let size = galley.size() + Vec2::new(40.0, 20.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let boost = if enabled && resp.hovered() { 1.0 } else { 0.9 };
        let k = if enabled { boost } else { 0.35 };
        if enabled {
            for (grow, alpha) in [(10.0, 0.05), (6.0, 0.08), (3.0, 0.12)] {
                let glow = rect.expand(grow * 0.6).translate(Vec2::new(0.0, 4.0));
                ui.painter().rect_filled(glow, CornerRadius::same((9.0 + grow) as u8), PINK.gamma_multiply(alpha));
            }
        }
        gradient_rect(ui.painter(), rect, 9.0, PINK.gamma_multiply(k), CYAN.gamma_multiply(k));
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, dark);
        resp
    }
}

pub fn nav_item(ui: &mut Ui, selected: bool, glyph: &str, label: &str, expand: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    let resp = if expand < 0.5 { resp.on_hover_text(label) } else { resp };
    let bg = if selected {
        Color32::from_white_alpha(22)
    } else if resp.hovered() {
        Color32::from_white_alpha(11)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
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
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(13.0));
        })
        .response
}

pub fn backdrop(painter: &egui::Painter, rect: egui::Rect) {
    let a = Color32::from_rgba_unmultiplied(34, 16, 38, 175);
    let b = Color32::from_rgba_unmultiplied(12, 12, 18, 185);
    let c = Color32::from_rgba_unmultiplied(6, 8, 14, 200);
    let d = Color32::from_rgba_unmultiplied(14, 10, 22, 190);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), a);
    mesh.colored_vertex(rect.right_top(), b);
    mesh.colored_vertex(rect.right_bottom(), c);
    mesh.colored_vertex(rect.left_bottom(), d);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

pub fn grad(t: f32) -> Color32 {
    PINK.lerp_to_gamma(CYAN, flow_mix(t))
}

pub fn power_button(ui: &mut Ui, on: bool, busy: bool) -> Response {
    let d = 92.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(d), Sense::click());
    let c = rect.center();
    let r = d / 2.0 - 8.0;
    let t = ui.ctx().animate_bool_responsive(resp.id.with("on"), on);
    let p = ui.painter();
    if t > 0.01 {
        for (k, a) in [(12.0, 0.04), (8.0, 0.07), (4.0, 0.11)] {
            p.circle_filled(c, r + k, grad(0.5).gamma_multiply(a * t));
        }
    }
    let base = if resp.hovered() { 20 } else { 12 };
    p.circle_filled(c, r, Color32::from_white_alpha(base));
    let n = 72;
    let time = ui.input(|i| i.time) as f32;
    let spin = if busy { time * 4.0 } else { 0.0 };
    let tau = std::f32::consts::TAU;
    for i in 0..n {
        let f = i as f32 / n as f32;
        let a0 = -tau / 4.0 + spin + f * tau;
        let a1 = a0 + tau / n as f32 + 0.01;
        let lit = grad(if f < 0.5 { f * 2.0 } else { 2.0 - f * 2.0 });
        let idle = Color32::from_white_alpha(38);
        let mut col = idle.lerp_to_gamma(lit, t);
        if busy {
            col = if f < 0.3 { lit.gamma_multiply(1.0 - f / 0.3 * 0.7) } else { idle };
        }
        p.line_segment([c + Vec2::angled(a0) * r, c + Vec2::angled(a1) * r], Stroke::new(3.5, col));
    }
    let glyph = if on { Color32::WHITE } else { Color32::from_gray(if resp.hovered() { 230 } else { 170 }) };
    p.text(c, egui::Align2::CENTER_CENTER, icon::POWER, icon_font(28.0), glyph);
    if busy || (t > 0.0 && t < 1.0) {
        ui.ctx().request_repaint();
    }
    resp
}

pub fn icon_button(ui: &mut Ui, glyph: &str, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(20));
    }
    let col = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(185) };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, icon_font(14.0), col);
    resp.on_hover_text(tip)
}

pub fn segmented(ui: &mut Ui, items: &[String], selected: &mut usize) -> bool {
    let mut changed = false;
    let font = FontId::new(13.5, FontFamily::Proportional);
    let need: f32 = items
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), Color32::WHITE).size().x + 26.0)
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
    egui::Frame::new()
        .fill(Color32::from_white_alpha(8))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(12)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, label) in items.iter().enumerate() {
                    let font = FontId::new(13.5, FontFamily::Proportional);
                    let galley = ui.painter().layout_no_wrap(label.clone(), font, Color32::WHITE);
                    let size = Vec2::new(galley.size().x + 26.0, 28.0);
                    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                    let on = *selected == i;
                    if on {
                        ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(26));
                        let bar = egui::Rect::from_center_size(
                            egui::pos2(rect.center().x, rect.bottom() - 3.0),
                            Vec2::new(14.0, 2.0),
                        );
                        gradient_rect(ui.painter(), bar, 1.0, PINK, CYAN);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(12));
                    }
                    let col = if on { Color32::WHITE } else { Color32::from_gray(175) };
                    ui.painter().galley(rect.center() - galley.size() / 2.0 - Vec2::new(0.0, 1.0), galley, col);
                    if resp.clicked() && !on {
                        *selected = i;
                        changed = true;
                    }
                }
            });
        });
    changed
}

pub fn paint_pill(painter: &egui::Painter, right_center: egui::Pos2, text: &str, color: Color32) -> egui::Rect {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::new(12.0, FontFamily::Proportional), color);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let rect = egui::Rect::from_min_size(egui::pos2(right_center.x - size.x, right_center.y - size.y / 2.0), size);
    painter.rect_filled(rect, CornerRadius::same(20), color.gamma_multiply(0.16));
    painter.rect_stroke(rect, CornerRadius::same(20), Stroke::new(1.0, color.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    rect
}

pub fn strategy_row(ui: &mut Ui, name: &str, meta: &str, selected: bool, active: bool, score: Option<(String, Color32)>) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    let p = ui.painter();
    if selected {
        gradient_rect(p, rect, 10.0, PINK, CYAN);
        p.rect_filled(rect.shrink(1.5), CornerRadius::same(9), Color32::from_rgba_unmultiplied(24, 20, 32, 245));
        p.rect_filled(rect.shrink(1.5), CornerRadius::same(9), Color32::from_white_alpha(8));
    } else {
        let a = if resp.hovered() { 14 } else { 6 };
        p.rect_filled(rect, CornerRadius::same(10), Color32::from_white_alpha(a));
    }
    let dot_c = egui::pos2(rect.left() + 20.0, rect.center().y);
    if selected {
        gradient_rect(p, egui::Rect::from_center_size(dot_c, Vec2::splat(12.0)), 6.0, PINK, CYAN);
    } else {
        p.circle_stroke(dot_c, 5.5, Stroke::new(1.2, Color32::from_white_alpha(70)));
    }
    let col = if selected { Color32::WHITE } else { Color32::from_gray(215) };
    p.text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        FontId::new(15.0, FontFamily::Proportional),
        col,
    );
    let mut right = egui::pos2(rect.right() - 14.0, rect.center().y);
    if active {
        let pill = paint_pill(p, right, "Активна", GREEN);
        right.x = pill.left() - 10.0;
    }
    if let Some((text, color)) = &score {
        let pill = paint_pill(p, right, text, *color);
        right.x = pill.left() - 10.0;
    }
    p.text(right, egui::Align2::RIGHT_CENTER, meta, FontId::new(12.5, FontFamily::Proportional), Color32::from_gray(140));
    resp
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
