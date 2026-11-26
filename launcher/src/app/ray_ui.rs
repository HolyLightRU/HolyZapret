use super::{App, Lvl, result_msg};
use crate::core::ray;
use crate::core::service::{self, ServiceState};
use crate::core::settings;
use crate::theme::{self, CYAN, GREEN, ORANGE, RED, YELLOW};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const SYSTEM: [&str; 12] = [
    "system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "fontdrvhost.exe", "dwm.exe", "memory compression", "[system process]",
];

#[derive(Default)]
pub struct RayState {
    loaded: bool,
    lists: [Vec<String>; 2],
    input: [String; 2],
    dirty: bool,
    running: Vec<String>,
    checked_at: Option<Instant>,
    service: Option<ServiceState>,
    last: Option<String>,
    live: Option<(ray::Live, Duration)>,
    experiment: String,
    quic: String,
    auto: bool,
    dns: bool,
    auto_count: usize,
    live_at: Option<Instant>,
    pub(super) icons: HashMap<String, (Option<egui::TextureHandle>, Instant)>,
    windowed: std::collections::HashSet<String>,
    filter: [String; 2],
}

pub(super) fn proc_rows(
    ui: &mut egui::Ui,
    cache: &mut HashMap<String, (Option<egui::TextureHandle>, Instant)>,
    names: Vec<&String>,
    windowed: &std::collections::HashSet<String>,
    chosen: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let ctx = ui.ctx().clone();
    ui.spacing_mut().item_spacing.y = 2.0;
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase())).unwrap_or_default();
    let (apps, rest): (Vec<&String>, Vec<&String>) = names.into_iter().filter(|n| n.to_lowercase() != me).partition(|n| windowed.contains(&n.to_lowercase()));
    let mut pick = None;
    for (title, list, icons) in [("Программы с окнами", apps, true), ("Фоновые процессы", rest, false)] {
        if list.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.label(RichText::new(title).color(Color32::from_gray(140)).size(12.0));
        ui.add_space(2.0);
        for n in list {
            let tex = if icons { app_icon(&ctx, cache, n) } else { None };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
            let on = chosen(n);
            let p = ui.painter();
            if on {
                p.rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(26));
            } else if resp.hovered() {
                p.rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(12));
            }
            let cy = rect.center().y.round();
            let spot = egui::Rect::from_center_size(egui::pos2((rect.left() + 17.0).round(), cy), egui::vec2(16.0, 16.0));
            match &tex {
                Some(t) => {
                    egui::Image::new(t).fit_to_exact_size(spot.size()).paint_at(ui, spot);
                }
                None => {
                    ui.painter().circle_filled(spot.center(), 3.0, Color32::from_gray(95));
                }
            }
            let color = if on || resp.hovered() { Color32::WHITE } else { Color32::from_gray(185) };
            let galley = ui.painter().layout_no_wrap(n.clone(), egui::FontId::proportional(13.5), color);
            let at = egui::pos2((rect.left() + 34.0).round(), (cy - galley.size().y / 2.0 - 1.0).round());
            ui.painter().galley(at, galley, color);
            if resp.clicked() {
                pick = Some(n.clone());
            }
        }
    }
    pick
}

fn split(v: &str) -> Vec<String> {
    v.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

fn norm(v: &str) -> Option<String> {
    let n = v.trim().trim_matches('"').to_lowercase();
    if n.is_empty() || n.contains(['\\', '/', ',', ';']) {
        return None;
    }
    Some(if n.ends_with(".exe") { n } else { format!("{n}.exe") })
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

fn app_icon(ctx: &egui::Context, cache: &mut HashMap<String, (Option<egui::TextureHandle>, Instant)>, name: &str) -> Option<egui::TextureHandle> {
    if let Some((t, at)) = cache.get(name) {
        if t.is_some() || at.elapsed() < Duration::from_secs(15) {
            return t.clone();
        }
    }
    let path = crate::core::procs::find(|n| n.eq_ignore_ascii_case(name)).into_iter().find_map(|p| p.path);
    let tex = path.and_then(|p| crate::core::icons::exe_icon(&p)).map(|i| {
        let img = egui::ColorImage::from_rgba_unmultiplied([i.width, i.height], &i.pixels);
        ctx.load_texture(format!("app-icon-{name}"), img, egui::TextureOptions::LINEAR)
    });
    cache.insert(name.to_string(), (tex.clone(), Instant::now()));
    tex
}

fn icon_or_dot(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, color: Color32) {
    match tex {
        Some(t) => {
            ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(18.0, 18.0)));
        }
        None => {
            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, color);
        }
    }
}

enum Act {
    Experiment(String),
    Quic(String),
    Auto(bool),
    Dns(bool),
    ClearAuto,
    Add(usize, String),
    Remove(usize, String),
    Apply,
    OpenLog,
}

impl App {
    fn ray_refresh(&mut self) {
        let st = &mut self.ray;
        if !st.loaded {
            st.loaded = true;
            let (all, skip) = settings::ray_apps();
            st.lists = [split(&all), split(&skip)];
            st.experiment = settings::ray_experiment();
            st.quic = settings::ray_quic();
            if st.quic.is_empty() {
                st.quic = "general".into();
            }
            st.auto = settings::ray_autolist();
            st.dns = settings::ray_dns();
        }
        if st.checked_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            if st.live_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
                st.live_at = Some(Instant::now());
                if let Some(p) = &self.paths {
                    st.live = ray::read_live(p);
                }
            }
            return;
        }
        st.checked_at = Some(Instant::now());
        if let Some(p) = &self.paths {
            st.auto_count = ray::auto_sites(p).len();
        }
        st.service = Some(service::query(service::SERVICE));
        let mut names: Vec<String> = crate::core::procs::names().into_values().map(|n| n.to_lowercase()).filter(|n| !SYSTEM.contains(&n.as_str())).collect();
        names.sort();
        names.dedup();
        self.ray.running = names;
        self.ray.windowed = crate::core::procs::windowed();
        let Some(p) = self.paths.clone() else { return };
        self.ray.last = ray::last_line(&p);
        self.ray.live = ray::read_live(&p);
        self.ray.live_at = Some(Instant::now());
    }

    fn ray_list(&mut self, ui: &mut egui::Ui, idx: usize, title: &str, hint: &str, act: &mut Option<Act>) {
        ui.add_space(6.0);
        ui.label(RichText::new(title).strong());
        ui.label(muted(hint));
        if !self.ray.lists[idx].is_empty() {
            let ctx = ui.ctx().clone();
            ui.horizontal_wrapped(|ui| {
                for n in self.ray.lists[idx].clone() {
                    let text = RichText::new(format!("{n}  ×")).size(12.5);
                    let b = match app_icon(&ctx, &mut self.ray.icons, &n) {
                        Some(t) => egui::Button::image_and_text(egui::Image::new(&t).fit_to_exact_size(egui::vec2(16.0, 16.0)), text),
                        None => egui::Button::new(text),
                    };
                    if ui.add(b).on_hover_text("Убрать").clicked() {
                        *act = Some(Act::Remove(idx, n));
                    }
                }
            });
        }
        theme::row(ui, 34.0, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.ray.input[idx]).hint_text("discord.exe").desired_width(180.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                *act = Some(Act::Add(idx, self.ray.input[idx].clone()));
            }
            if ui.button("Добавить").clicked() {
                *act = Some(Act::Add(idx, self.ray.input[idx].clone()));
            }
            egui::ComboBox::from_id_salt(("ray_pick", idx)).selected_text("Из запущенных").width(260.0).height(380.0).popup_style(crate::theme::solid_popup()).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ray.filter[idx]).hint_text("Поиск по имени").desired_width(f32::INFINITY));
                let f = self.ray.filter[idx].trim().to_lowercase();
                let names: Vec<&String> = self.ray.running.iter().filter(|n| f.is_empty() || n.contains(&f)).collect();
                if let Some(n) = proc_rows(ui, &mut self.ray.icons, names, &self.ray.windowed, &|_| false) {
                    *act = Some(Act::Add(idx, n));
                }
            });
        });
    }

    pub(super) fn ray_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.ray_refresh();
        let exe = self.paths.as_ref().and_then(ray::exe);
        let mut act = None;
        ui.add_space(10.0);
        theme::card(ui, |ui| {
            theme::row(ui, 32.0, |ui| {
                ui.label(RichText::new(theme::icon::DIAGNOSTIC).font(theme::icon_font(17.0)).color(CYAN));
                ui.add_space(2.0);
                ui.label(RichText::new("RayOfLight").size(15.5).strong());
                if crate::core::TEST_BUILD {
                    theme::chip(ui, "тестовая сборка", YELLOW);
                }
            });
            ui.add(
                egui::Label::new(muted(
                    "Движок обхода HolyZapret. Сам подбирает приём для каждого сайта и запоминает рабочий, умеет обходить программы целиком или не трогать их. Работает, пока запущена служба HolyZapret.",
                ))
                .wrap(),
            );
            ui.add_space(4.0);
            let state = match (&exe, &self.ray.service) {
                (None, _) => ("Нет файла bin\\RayOfLight.exe".to_string(), RED),
                (_, Some(ServiceState::Running)) => ("Движок работает".to_string(), GREEN),
                (_, Some(ServiceState::Stopped)) => ("Служба остановлена, движок ждёт её запуска".to_string(), Color32::from_gray(170)),
                _ => ("Заработает, когда вы включите защиту на Главной".to_string(), Color32::from_gray(170)),
            };
            theme::row(ui, 26.0, |ui| {
                ui.label(RichText::new("●").color(state.1).size(13.0));
                ui.label(RichText::new(state.0).color(Color32::from_gray(215)).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.ray.last.is_some() && ui.button("Журнал").clicked() {
                        act = Some(Act::OpenLog);
                    }
                });
            });
            if let Some(l) = &self.ray.last {
                ui.add(egui::Label::new(RichText::new(l).monospace().size(11.5).color(Color32::from_gray(150))).wrap());
            }
            ui.add_space(6.0);
            let font = egui::TextStyle::Body.resolve(ui.style());
            let label_w = ["Приём", "QUIC"].iter().map(|t| ui.painter().layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x).fold(0.0, f32::max);
            theme::row(ui, 34.0, |ui| {
                let w = ui.label(RichText::new("Приём").strong()).rect.width();
                ui.add_space(6.0 + (label_w - w).max(0.0));
                egui::ComboBox::from_id_salt("ray_experiment").selected_text(ray::experiment_label(&self.ray.experiment)).width(280.0).show_ui(ui, |ui| {
                    for (name, label, hint) in ray::EXPERIMENTS {
                        if ui.selectable_label(self.ray.experiment == name, label).on_hover_text(hint).clicked() && self.ray.experiment != name {
                            act = Some(Act::Experiment(name.to_string()));
                        }
                    }
                });
            });
            let hint = ray::EXPERIMENTS.iter().find(|e| e.0 == self.ray.experiment).map(|e| e.2).unwrap_or("");
            ui.label(muted(hint));
            if let Some(t) = ray::start_technique().filter(|_| self.ray.experiment == "adaptive") {
                ui.label(muted(format!("Первым шагом идёт «{}»: его выбрал подбор", ray::experiment_label(&t))));
            }
            theme::row(ui, 34.0, |ui| {
                let w = ui.label(RichText::new("QUIC").strong()).rect.width();
                ui.add_space(6.0 + (label_w - w).max(0.0));
                egui::ComboBox::from_id_salt("ray_quic").selected_text(ray::quic_label(&self.ray.quic)).width(280.0).show_ui(ui, |ui| {
                    for (name, label, hint) in ray::QUIC_EXPERIMENTS {
                        if ui.selectable_label(self.ray.quic == name, label).on_hover_text(hint).clicked() && self.ray.quic != name {
                            act = Some(Act::Quic(name.to_string()));
                        }
                    }
                });
            });
            let qhint = ray::QUIC_EXPERIMENTS.iter().find(|e| e.0 == self.ray.quic).map(|e| e.2).unwrap_or("");
            ui.label(muted(qhint));
            ui.add_space(4.0);
            theme::row(ui, 30.0, |ui| {
                let mut auto = self.ray.auto;
                if ui.checkbox(&mut auto, "Сам находить заблокированные сайты").changed() {
                    act = Some(Act::Auto(auto));
                }
                if self.ray.auto_count > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Очистить").clicked() {
                            act = Some(Act::ClearAuto);
                        }
                        ui.label(muted(format!("в автосписке сайтов: {}", self.ray.auto_count)));
                    });
                }
            });
            ui.label(muted("Сайт не из списков, который дважды не ответил, дальше обходится сам. Если обход ему не нужен, самоподстройка это заметит"));
            theme::row(ui, 30.0, |ui| {
                let mut dns = self.ray.dns;
                if ui.checkbox(&mut dns, "Чинить DNS для сайтов из списков").changed() {
                    act = Some(Act::Dns(dns));
                }
            });
            ui.label(muted("Если провайдер прячет адрес заблокированного сайта (пишет, что такого сайта нет, или подсовывает пустышку), RayOfLight узнаёт настоящий адрес у шифрованного DNS Google или Cloudflare. Обычные ответы не трогаются"));
            self.ray_live(ui, &mut act);
            self.ray_list(ui, 0, "Обходить целиком", "Весь HTTPS этих программ идёт через обход, даже сайты не из списков", &mut act);
            self.ray_list(ui, 1, "Не трогать", "Эти программы обход не трогает никогда, даже на сайтах из списков", &mut act);
            if self.ray.dirty {
                ui.add_space(6.0);
                theme::row(ui, 34.0, |ui| {
                    if ui.add_enabled(!self.busy, theme::Primary("Применить".into())).clicked() {
                        act = Some(Act::Apply);
                    }
                    ui.label(muted("Служба переустановится с новыми настройками"));
                });
            }
        });
        match act {
            Some(Act::Experiment(name)) => {
                self.ray.experiment = name;
                self.ray.dirty = true;
            }
            Some(Act::Auto(on)) => {
                self.ray.auto = on;
                self.ray.dirty = true;
            }
            Some(Act::Dns(on)) => {
                self.ray.dns = on;
                self.ray.dirty = true;
            }
            Some(Act::ClearAuto) => {
                if let Some(p) = &self.paths {
                    ray::clear_autolist(p);
                }
                self.ray.auto_count = 0;
                self.ray.dirty = true;
            }
            Some(Act::Quic(name)) => {
                self.ray.quic = name;
                self.ray.dirty = true;
            }
            Some(Act::Add(idx, raw)) => match norm(&raw) {
                Some(n) => {
                    if !self.ray.lists[idx].contains(&n) {
                        self.ray.lists[idx].push(n.clone());
                        self.ray.lists[1 - idx].retain(|x| x != &n);
                        self.ray.dirty = true;
                    }
                    self.ray.input[idx].clear();
                }
                None => self.toast(ctx, Lvl::Err, "Укажите имя программы, например discord.exe"),
            },
            Some(Act::Remove(idx, n)) => {
                self.ray.lists[idx].retain(|x| x != &n);
                self.ray.dirty = true;
            }
            Some(Act::Apply) => self.ray_apply(ctx),
            Some(Act::OpenLog) => {
                if let Some(p) = &self.paths {
                    let _ = std::process::Command::new("notepad.exe").arg(ray::log_path(p)).spawn();
                }
            }
            None => {}
        }
    }

    fn ray_live(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        ui.ctx().request_repaint_after(Duration::from_secs(2));
        ui.add_space(8.0);
        ui.label(RichText::new("Что обходится сейчас").strong());
        let live = self.ray.live.clone();
        let Some((l, age)) = live.filter(|(l, age)| l.running && *age < Duration::from_secs(10)) else {
            ui.label(muted("Движок сейчас не работает. Данные появятся, когда служба запущена и программы открывают сайты"));
            return;
        };
        ui.label(muted(format!(
            "{} · обработано {}, пропущено {} с запуска службы · обновлено {} с назад",
            ray::experiment_label(&l.mode),
            l.handled,
            l.skipped,
            age.as_secs()
        )));
        if l.answered + l.reset + l.silent + l.frozen > 0 {
            theme::row(ui, 24.0, |ui| {
                ui.label(RichText::new(format!("ответ {}", l.answered)).color(GREEN).size(12.5));
                ui.label(RichText::new(format!("сброс {}", l.reset)).color(RED).size(12.5));
                ui.label(RichText::new(format!("тишина {}", l.silent)).color(YELLOW).size(12.5));
                if l.frozen > 0 {
                    ui.label(RichText::new(format!("заморозка {}", l.frozen)).color(ORANGE).size(12.5))
                        .on_hover_text("Сервер ответил, но после 12–32 КБ замолчал: так провайдер режет загрузку с некоторых хостингов. Движок пробует другой адрес сервера, потом WARP");
                }
                if l.techniques.len() > 1 {
                    let per: Vec<String> = l
                        .techniques
                        .iter()
                        .map(|t| format!("{} {}", ray::experiment_inline(&t.name), ray::tech_result(t)))
                        .collect();
                    ui.label(muted(format!("· {}", per.join(", "))));
                }
            });
        }
        let apps = ray::by_app(&l);
        if apps.is_empty() {
            ui.label(muted("Пока ни одна программа не открывала HTTPS-сайты"));
            return;
        }
        let ctx = ui.ctx().clone();
        ui.add_space(2.0);
        for a in apps.iter().take(8) {
            let known = a.app != "неизвестная программа";
            let tex = if known { app_icon(&ctx, &mut self.ray.icons, &a.app) } else { None };
            let color = if a.handled > 0 { GREEN } else { Color32::from_gray(140) };
            theme::row(ui, 34.0, |ui| {
                icon_or_dot(ui, tex.as_ref(), color);
                ui.add_space(2.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&a.app).color(Color32::from_gray(225)).size(13.5));
                    let sites = if a.hosts.is_empty() { String::new() } else { format!(" · {}", a.hosts.join(", ")) };
                    let res = if a.answered + a.failed > 0 { format!(", ответ {} из {}", a.answered, a.answered + a.failed) } else { String::new() };
                    ui.label(muted(format!("обработано {}, пропущено {}{res}{sites}", a.handled, a.skipped)));
                });
                if known {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let skip = self.ray.lists[1].contains(&a.app);
                        let all = self.ray.lists[0].contains(&a.app);
                        if skip {
                            ui.label(RichText::new("не трогается").color(Color32::from_gray(170)).size(12.5));
                        } else if ui.small_button("Не трогать").clicked() {
                            *act = Some(Act::Add(1, a.app.clone()));
                        }
                        if all {
                            ui.label(RichText::new("обходится целиком").color(GREEN).size(12.5));
                        } else if !skip && ui.small_button("Обходить целиком").clicked() {
                            *act = Some(Act::Add(0, a.app.clone()));
                        }
                    });
                }
            });
        }
        egui::CollapsingHeader::new(RichText::new("Последние решения").color(Color32::from_gray(190)).size(13.0)).id_salt("ray_recent").show(ui, |ui| {
            for i in l.recent.iter().rev().take(14) {
                theme::row(ui, 22.0, |ui| {
                    ui.label(RichText::new(&i.time).monospace().size(11.5).color(Color32::from_gray(120)));
                    let dot = match i.outcome.as_deref() {
                        Some("answered") => GREEN,
                        Some("reset") => RED,
                        Some("silent") => YELLOW,
                        Some("frozen") => ORANGE,
                        _ if i.handled => CYAN,
                        _ => Color32::from_gray(130),
                    };
                    ui.label(RichText::new("●").color(dot).size(11.0));
                    let who = format!("{} · {}", i.host.as_deref().unwrap_or("без имени"), i.app.as_deref().unwrap_or("?"));
                    ui.label(RichText::new(who).size(12.5).color(Color32::from_gray(210)));
                    let mut why = i.why.clone();
                    if let Some(t) = &i.technique {
                        why.push_str(&format!(" · {}", ray::experiment_inline(t)));
                    }
                    if let Some(o) = &i.outcome {
                        why.push_str(&format!(" · {}", ray::outcome_label(o)));
                    }
                    ui.label(muted(why));
                });
            }
        });
    }

    fn ray_apply(&mut self, ctx: &egui::Context) {
        settings::set_ray_apps(&self.ray.lists[0].join(","), &self.ray.lists[1].join(","));
        settings::set_ray_experiment(&self.ray.experiment);
        settings::set_ray_quic(&self.ray.quic);
        settings::set_ray_autolist(self.ray.auto);
        settings::set_ray_dns(self.ray.dns);
        self.ray.dirty = false;
        self.ray.checked_at = None;
        let installed = self.status.as_ref().is_some_and(|s| s.active_strategy().is_some());
        match (installed, self.ray_current()) {
            (true, Some(s)) => self.spawn(ctx, move |p, tx| {
                result_msg(tx, service::install(p, &s), "Не удалось переустановить службу");
            }),
            _ => self.toast(ctx, Lvl::Info, "Настройки сохранены: они заработают, когда вы включите защиту на Главной"),
        }
    }

    pub(super) fn ray_current(&self) -> Option<crate::core::strategy::Strategy> {
        let id = format!("rayoflight:{}", settings::ray_experiment());
        let mut s = self.strategies.iter().find(|s| s.id == id).or_else(|| self.strategies.iter().find(|s| s.id == ray::DEFAULT_ID)).cloned()?;
        s.ray = None;
        Some(s)
    }
}
