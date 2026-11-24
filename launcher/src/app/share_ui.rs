use super::{App, Msg, notify, send};
use crate::core::share::{self, Share};
use crate::core::strategy::Strategy;
use crate::core::{flowseal, gamefilter, service};
use crate::log::Lvl;
use crate::theme;
use eframe::egui::{self, Color32, CornerRadius, RichText};

#[derive(Default)]
pub struct ShareState {
    pub open: bool,
    pub input: String,
    pub error: Option<String>,
    pub preview: Option<Preview>,
}

pub struct Preview {
    share: Share,
    found: Option<Strategy>,
    use_strategy: bool,
    use_lists: bool,
    use_rules: bool,
    use_game: bool,
}

fn short_name(id: &str) -> &str {
    id.rsplit('\\').next().unwrap_or(id)
}

fn sample(items: &[String]) -> String {
    let mut s = items.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > 5 {
        s.push_str(&format!(" и ещё {}", items.len() - 5));
    }
    s
}

fn summary(sh: &Share) -> String {
    let mut parts = Vec::new();
    if let Some(s) = &sh.strategy {
        parts.push(format!("стратегия «{}»", short_name(s)));
    }
    let n = sh.list_count();
    if n > 0 {
        parts.push(format!("записей в списках: {n}"));
    }
    if !sh.rules.is_empty() {
        parts.push(format!("правил для сайтов: {}", sh.rules.len()));
    }
    if sh.game.is_some() {
        parts.push("игровой фильтр".into());
    }
    parts.join(", ")
}

fn muted(text: impl Into<String>) -> egui::Label {
    egui::Label::new(RichText::new(text.into()).color(Color32::from_gray(160)).size(12.5)).wrap()
}

impl App {
    pub(super) fn copy_share_code(&mut self, ctx: &egui::Context) {
        let Some(p) = self.paths.clone() else { return };
        let strategy = self
            .status
            .as_ref()
            .and_then(|s| s.active_strategy())
            .and_then(|id| self.strategies.iter().find(|s| s.matches(id)))
            .map(|s| s.id.clone());
        let game = self.status.as_ref().map(|s| s.game.clone()).unwrap_or_default();
        let sh = Share::collect(&p, strategy, &game);
        if sh.is_empty() {
            self.toast(ctx, Lvl::Info, "Делиться пока нечем: служба не установлена, списки и правила пустые");
            return;
        }
        ctx.copy_text(sh.encode());
        self.toast(ctx, Lvl::Ok, format!("Код скопирован: {}. Отправьте его другу, он вставит его в Настройки → «Код от друга»", summary(&sh)));
    }

    fn check_share_code(&mut self) {
        match share::decode(&self.share.input) {
            Ok(sh) => {
                let found = sh.strategy.as_ref().and_then(|id| self.strategies.iter().find(|s| s.matches(id)).cloned());
                self.share.error = None;
                self.share.preview = Some(Preview {
                    use_strategy: found.is_some(),
                    use_lists: sh.list_count() > 0,
                    use_rules: !sh.rules.is_empty(),
                    use_game: sh.game.is_some(),
                    found,
                    share: sh,
                });
            }
            Err(e) => self.share.error = Some(e),
        }
    }

    fn apply_share(&mut self, ctx: &egui::Context) {
        if self.trial.is_some() || self.picker.running() {
            self.toast(ctx, Lvl::Info, "Сначала остановите пробный запуск и подбор");
            return;
        }
        let Some(pv) = self.share.preview.take() else { return };
        self.share = ShareState::default();
        let installed = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from));
        let current = installed.and_then(|id| self.strategies.iter().find(|s| s.matches(&id)).cloned());
        let target = if pv.use_strategy { pv.found.clone() } else { None }.or(current);
        let Preview { share: sh, use_lists, use_rules, use_game, .. } = pv;
        self.spawn(ctx, move |p, tx| {
            send(tx, Lvl::Info, "Применяю настройку друга…");
            let mut done = Vec::new();
            if use_lists {
                match share::apply_lists(p, &sh) {
                    Ok(n) => done.push(format!("новых записей в списках: {n}")),
                    Err(e) => notify(tx, Lvl::Err, format!("Списки не сохранились: {e}")),
                }
            }
            if use_rules {
                match share::apply_rules(p, &sh) {
                    Ok(n) => done.push(format!("правил для сайтов: {n}")),
                    Err(e) => notify(tx, Lvl::Err, format!("Правила не сохранились: {e}")),
                }
            }
            if let Some(g) = sh.game.as_ref().filter(|_| use_game) {
                match gamefilter::save(p, g) {
                    Ok(()) => done.push(format!("игровой фильтр: {}", g.summary())),
                    Err(e) => notify(tx, Lvl::Err, format!("Игровой фильтр не сохранился: {e}")),
                }
            }
            if let Some(s) = target {
                match service::install(p, &s) {
                    Ok(_) => done.push(format!("служба запущена со стратегией «{}»", s.name)),
                    Err(e) => notify(tx, Lvl::Err, format!("Службу не удалось установить: {e}")),
                }
            }
            if !done.is_empty() {
                notify(tx, Lvl::Ok, format!("Настройка друга применена: {}", done.join(", ")));
            }
            let _ = tx.send(Msg::Shared);
        });
    }

    pub(super) fn draw_share(&mut self, ctx: &egui::Context) {
        if !self.share.open {
            return;
        }
        let mut check = false;
        let mut apply = false;
        let mut back = false;
        let mut close = false;
        let busy = self.busy;
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(26, 22, 32, 250))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(28)))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20));
        let st = &mut self.share;
        let modal = egui::Modal::new(egui::Id::NULL.with("share")).frame(frame).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Настройка от друга").size(17.0).strong());
            ui.add_space(4.0);
            match st.preview.as_mut() {
                None => {
                    ui.add(muted("Вставьте код, который прислал друг. Он начинается с HZ1: и целиком копируется из сообщения."));
                    ui.add_space(8.0);
                    ui.add(egui::TextEdit::multiline(&mut st.input).desired_rows(4).desired_width(f32::INFINITY).hint_text("HZ1:…"));
                    if let Some(e) = &st.error {
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(RichText::new(e).color(theme::RED).size(12.5)).wrap());
                    }
                    ui.add_space(14.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!st.input.trim().is_empty(), theme::Primary("Посмотреть".into())).clicked() {
                            check = true;
                        }
                        if ui.button("Отмена").clicked() {
                            close = true;
                        }
                    });
                }
                Some(pv) => {
                    ui.add(muted("Отметьте, что взять. Ваши списки и правила не удаляются, новое добавится к ним."));
                    ui.add_space(8.0);
                    if let Some(id) = &pv.share.strategy {
                        match &pv.found {
                            Some(s) => {
                                ui.checkbox(&mut pv.use_strategy, format!("Стратегия «{}»: служба переустановится с ней", s.name));
                                ui.indent("s", |ui| {
                                    ui.add(muted("Стратегию подбирали под провайдера друга. Если у вас другой провайдер, она может не подойти, тогда запустите подбор."));
                                });
                            }
                            None => {
                                let hint = if id.to_lowercase().starts_with(&format!("{}\\", flowseal::DIR)) {
                                    "скачайте стратегии Flowseal в Настройках и вставьте код ещё раз"
                                } else {
                                    "обновите HolyZapret и вставьте код ещё раз"
                                };
                                ui.add(egui::Label::new(RichText::new(format!("Стратегии «{}» у вас нет: {hint}", short_name(id))).color(theme::YELLOW).size(12.5)).wrap());
                            }
                        }
                        ui.add_space(4.0);
                    }
                    let n = pv.share.list_count();
                    if n > 0 {
                        ui.checkbox(&mut pv.use_lists, format!("Добавить в мои списки, записей: {n}"));
                        let all: Vec<String> = pv.share.lists.iter().flatten().cloned().collect();
                        ui.indent("l", |ui| {
                            ui.add(muted(sample(&all)));
                        });
                        ui.add_space(4.0);
                    }
                    if !pv.share.rules.is_empty() {
                        ui.checkbox(&mut pv.use_rules, format!("Правила для сайтов: {}", pv.share.rules.len()));
                        let domains: Vec<String> = pv.share.rules.iter().flat_map(|r| r.domains.clone()).collect();
                        ui.indent("r", |ui| {
                            ui.add(muted(sample(&domains)));
                        });
                        ui.add_space(4.0);
                    }
                    if let Some(g) = &pv.share.game {
                        ui.checkbox(&mut pv.use_game, format!("Игровой фильтр: {}", g.summary()));
                    }
                    let any = pv.use_strategy || pv.use_lists || pv.use_rules || pv.use_game;
                    ui.add_space(14.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(any && !busy, theme::Primary("Применить".into())).clicked() {
                            apply = true;
                        }
                        if ui.button("Назад").clicked() {
                            back = true;
                        }
                    });
                }
            }
        });
        if super::modal_close(ctx, &modal) {
            close = true;
        }
        if check {
            self.check_share_code();
        }
        if back {
            self.share.preview = None;
        }
        if apply {
            self.apply_share(ctx);
        }
        if close {
            self.share = ShareState::default();
        }
    }
}
