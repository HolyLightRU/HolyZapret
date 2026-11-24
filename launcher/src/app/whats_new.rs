use super::App;
use crate::theme::{self, CYAN};
use eframe::egui::{self, Color32, CornerRadius, RichText};

pub const RELEASES: &str = "https://github.com/HolyLightRU/HolyZapret/releases/tag/v";

pub enum Line {
    Heading(String),
    Bullet(String),
    Text(String),
}

pub fn parse(body: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('<') || l.starts_with("![") {
            continue;
        }
        let clean = |s: &str| s.replace("**", "").replace('`', "").trim().to_string();
        if let Some(h) = l.strip_prefix('#') {
            let h = clean(h.trim_start_matches('#')).trim_end_matches(':').to_string();
            if h.to_lowercase().contains("скрин") {
                break;
            }
            if !h.is_empty() {
                out.push(Line::Heading(h));
            }
        } else if let Some(b) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
            out.push(Line::Bullet(clean(b)));
        } else {
            out.push(Line::Text(clean(l)));
        }
    }
    out
}

impl App {
    pub(super) fn draw_whats_new(&mut self, ctx: &egui::Context) {
        let Some((version, body)) = self.whats_new.as_ref() else { return };
        let lines = parse(body);
        let version = version.clone();
        let mut close = false;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        let modal = egui::Modal::new(egui::Id::NULL.with("whats_new")).frame(frame).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.label(theme::gradient_text(&format!("Что нового в HolyZapret {version}"), 18.0, theme::PINK, CYAN));
            ui.label(RichText::new("Лаунчер обновился. Вот что изменилось").color(Color32::from_gray(170)).size(12.5));
            ui.add_space(8.0);
            let h = (ctx.content_rect().height() - 220.0).clamp(160.0, 380.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for l in &lines {
                    match l {
                        Line::Heading(t) => {
                            ui.add_space(4.0);
                            ui.label(RichText::new(t).size(15.0).strong());
                        }
                        Line::Bullet(t) => {
                            ui.horizontal_top(|ui| {
                                ui.label(RichText::new("•").color(CYAN));
                                ui.add(egui::Label::new(RichText::new(t).color(Color32::from_gray(205))).wrap());
                            });
                        }
                        Line::Text(t) => {
                            ui.add(egui::Label::new(RichText::new(t).color(Color32::from_gray(205))).wrap());
                        }
                    }
                }
            });
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::Primary("Понятно".into())).clicked() {
                    close = true;
                }
                if ui.button("Открыть на GitHub").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(format!("{RELEASES}{version}")));
                }
            });
        });
        if close || super::modal_close(ctx, &modal) {
            crate::core::settings::set_seen_version(&version);
            self.whats_new = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_parsed() {
        let body = "# Изменения:\r\n- Правила для **сайтов**\r\n- Набор `Эксперимент`\r\n# Доп. комментарии:\r\nСпасибо 🩷\r\n# Скрины:\r\n<img src=\"x\" />\r\n- не показывать";
        let lines = parse(body);
        assert_eq!(lines.len(), 5);
        assert!(matches!(&lines[0], Line::Heading(h) if h == "Изменения"));
        assert!(matches!(&lines[1], Line::Bullet(b) if b == "Правила для сайтов"));
        assert!(matches!(&lines[2], Line::Bullet(b) if b == "Набор Эксперимент"));
        assert!(matches!(&lines[3], Line::Heading(h) if h == "Доп. комментарии"));
        assert!(matches!(&lines[4], Line::Text(t) if t == "Спасибо 🩷"));
    }
}
