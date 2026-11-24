use super::{App, Lvl, Msg, notify, result_msg};
use crate::core::appcheck::{self, Flow, Judged, Seen};
use crate::core::coverage::{Cache, Verdict};
use crate::core::gamefilter::{self, GameFilter, Mode};
use crate::core::lists::{self, Kind};
use crate::core::service::{self, ServiceState};
use crate::core::strategy::{self, Strategy};
use crate::theme::{self, CYAN, GREEN, RED, YELLOW};
use eframe::egui::{self, Color32, RichText};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(180);
const SKIP: [&str; 3] = ["HolyZapret.exe", "winws.exe", "holyzapret-launcher.exe"];

#[derive(Default)]
pub struct CheckState {
    pub view: usize,
    stop: Option<Arc<AtomicBool>>,
    started: Option<Instant>,
    seen: Arc<Mutex<Vec<Seen>>>,
    error: Arc<Mutex<Option<String>>>,
    names: HashMap<u32, String>,
    names_at: Option<Instant>,
    picked: Option<String>,
    target: Option<Target>,
    cache: Cache,
    judged: HashMap<Flow, (Verdict, Verdict)>,
}

struct Target {
    strategy: Strategy,
    now: String,
    games: String,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    NeedsGames,
    NotListed,
    Touched,
    ByName,
    ByContent,
    Excluded,
    Passed,
    Unseen,
}

fn group_of(j: &Judged) -> Group {
    match j.now {
        Verdict::Touched => Group::Touched,
        Verdict::ByName => Group::ByName,
        Verdict::ByContent => Group::ByContent,
        Verdict::NotListed => Group::NotListed,
        Verdict::Excluded => Group::Excluded,
        Verdict::Passed | Verdict::NoProfile => Group::Passed,
        Verdict::Unseen if j.with_games != Verdict::Unseen => Group::NeedsGames,
        Verdict::Unseen => Group::Unseen,
    }
}

fn describe(g: Group) -> (Color32, &'static str, &'static str) {
    match g {
        Group::NeedsGames => (YELLOW, "Обход не видит: выключен игровой фильтр", "Эти порты обход начнёт обрабатывать, если включить игровой фильтр"),
        Group::NotListed => (YELLOW, "Порт подходит, но адреса нет в «Мои IP»", "Для игровых портов обход трогает только адреса из списка «Мои IP»"),
        Group::Touched => (GREEN, "Обход обрабатывает", "Движок меняет этот трафик. Если программа от этого ломается, её можно исключить"),
        Group::ByName => (CYAN, "Зависит от имени сайта", "Обход сработает, если сайт есть в списках. Проверить сайт можно на вкладке «Сайты»"),
        Group::ByContent => (CYAN, "Зависит от содержимого", "Обход тронет только голос Discord и похожий на него трафик"),
        Group::Excluded => (Color32::from_gray(150), "В исключениях", "Адрес в исключениях IP, обход его пропускает"),
        Group::Passed => (Color32::from_gray(150), "Обход пропускает", "Порт перехватывается, но правила для такого трафика в стратегии нет"),
        Group::Unseen => (Color32::from_gray(150), "Обход не трогает", "Стратегия не перехватывает эти порты"),
    }
}

fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(Color32::from_gray(165)).size(12.5)
}

fn flow_text(f: &Flow) -> String {
    let ip = match f.ip {
        std::net::IpAddr::V6(v) => format!("[{v}]"),
        v => v.to_string(),
    };
    format!("{ip}:{} {}", f.port, if f.udp { "UDP" } else { "TCP" })
}

enum Act {
    Start,
    Stop,
    Games,
    AddMine(Vec<String>),
    Exclude(Vec<String>),
}

impl App {
    fn check_target(&self) -> Option<Target> {
        let p = self.paths.as_ref()?;
        let s = self.rules_strategy()?;
        let gf = gamefilter::load(p);
        let all = GameFilter { mode: Mode::All, ..gf.clone() };
        let now = crate::core::rules::with_rules(p, &s, &gf).ok()?;
        let games = crate::core::rules::with_rules(p, &s, &all).ok()?;
        Some(Target { strategy: s, now, games })
    }

    fn check_reset(&mut self) {
        self.check.target = self.check_target();
        self.check.cache = Cache::default();
        self.check.judged.clear();
    }

    fn check_start(&mut self, ctx: &egui::Context) {
        let Some(p) = self.paths.clone() else { return };
        self.check_stop();
        self.check_reset();
        *self.check.error.lock().unwrap() = None;
        *self.check.seen.lock().unwrap() = appcheck::tcp_now();
        self.check.names = crate::core::procs::names();
        self.check.names_at = Some(Instant::now());
        let stop = Arc::new(AtomicBool::new(false));
        self.check.stop = Some(stop.clone());
        self.check.started = Some(Instant::now());
        let (seen, error, ctx2) = (self.check.seen.clone(), self.check.error.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = appcheck::watch(&p.bin(), stop.clone(), |s| {
                seen.lock().unwrap().push(s);
                ctx2.request_repaint();
            });
            if let Err(e) = r {
                *error.lock().unwrap() = Some(e.to_string());
            }
            stop.store(true, Ordering::Relaxed);
            ctx2.request_repaint();
        });
    }

    pub(super) fn check_stop(&mut self) {
        if let Some(s) = self.check.stop.take() {
            s.store(true, Ordering::Relaxed);
        }
        self.check.started = None;
    }

    fn check_running(&self) -> bool {
        self.check.stop.as_ref().is_some_and(|s| !s.load(Ordering::Relaxed))
    }

    pub(super) fn check_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let running = self.check_running();
        if !running && self.check.started.is_some() {
            self.check.started = None;
        }
        if let Some(t) = self.check.started {
            if t.elapsed() > LIMIT {
                self.check_stop();
            } else {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        if running && self.check.names_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            self.check.names.extend(crate::core::procs::names());
            self.check.names_at = Some(Instant::now());
        }
        let seen = self.check.seen.lock().unwrap().clone();
        let apps = appcheck::group(&seen, &self.check.names, &SKIP);
        if self.check.picked.as_ref().is_none_or(|p| !apps.iter().any(|a| a.name.eq_ignore_ascii_case(p))) {
            self.check.picked = apps.first().map(|a| a.name.clone());
        }
        let error = self.check.error.lock().unwrap().clone();
        let mut act = None;
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Проверка программы").size(17.0).strong());
                ui.add_space(4.0);
                if let Some(t) = self.check.started.filter(|_| running) {
                    ui.spinner();
                    let s = t.elapsed().as_secs();
                    ui.label(RichText::new(format!("Слушаю {}:{:02}", s / 60, s % 60)).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if running {
                        if ui.button("Остановить").clicked() {
                            act = Some(Act::Stop);
                        }
                    } else if ui.add_enabled(self.paths.is_some(), theme::Primary(if seen.is_empty() { "Начать" } else { "Начать заново" }.into())).clicked() {
                        act = Some(Act::Start);
                    }
                });
            });
            ui.add(
                egui::Label::new(muted(
                    "Нажмите «Начать», запустите программу и сделайте то, что не работает: зайдите в матч, позвоните, откройте видео. Лаунчер запишет, куда она подключается, и покажет, трогает ли этот трафик обход. Заблокирована ли сама программа, проверка сказать не может.",
                ))
                .wrap(),
            );
            if let Some(e) = &error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(format!("Не удалось слушать соединения: {e}")).color(RED).size(13.0)).wrap());
            }
            match &self.check.target {
                Some(t) if !seen.is_empty() => {
                    let off = self.status.as_ref().is_some_and(|s| !s.engine_running);
                    let text = if off {
                        format!("Сверяю со стратегией «{}». Обход сейчас выключен", t.strategy.name)
                    } else {
                        format!("Сверяю со стратегией «{}»", t.strategy.name)
                    };
                    ui.add_space(2.0);
                    ui.label(muted(text));
                }
                None if !seen.is_empty() => {
                    ui.add_space(2.0);
                    ui.label(RichText::new("Стратегия не выбрана, сверять не с чем").color(YELLOW).size(12.5));
                }
                _ => {}
            }
        });
        if !apps.is_empty() {
            ui.add_space(10.0);
            theme::card(ui, |ui| {
                theme::row(ui, 34.0, |ui| {
                    ui.label(RichText::new("Программа").color(Color32::from_gray(170)));
                    ui.add_space(6.0);
                    let current = self.check.picked.clone().unwrap_or_default();
                    let label = apps.iter().find(|a| a.name.eq_ignore_ascii_case(&current)).map(|a| format!("{} · {}", a.name, a.flows.len())).unwrap_or_default();
                    egui::ComboBox::from_id_salt("appcheck_pick").selected_text(label).width(260.0).show_ui(ui, |ui| {
                        for a in &apps {
                            let on = a.name.eq_ignore_ascii_case(&current);
                            if ui.selectable_label(on, format!("{} · {}", a.name, a.flows.len())).clicked() {
                                self.check.picked = Some(a.name.clone());
                            }
                        }
                    });
                });
                let Some(app) = apps.iter().find(|a| self.check.picked.as_ref().is_some_and(|p| a.name.eq_ignore_ascii_case(p))) else { return };
                let judged: Vec<Judged> = match &self.check.target {
                    Some(t) => app
                        .flows
                        .keys()
                        .map(|f| {
                            let (now, games) = *self.check.judged.entry(*f).or_insert_with(|| {
                                (
                                    crate::core::coverage::verdict(&t.now, f.udp, f.ip, f.port, &mut self.check.cache),
                                    crate::core::coverage::verdict(&t.games, f.udp, f.ip, f.port, &mut self.check.cache),
                                )
                            });
                            Judged { flow: *f, now, with_games: games }
                        })
                        .collect(),
                    None => Vec::new(),
                };
                let flows: Vec<Flow> = app.flows.keys().copied().collect();
                ui.label(muted(format!("Порты: {}", appcheck::ports(&flows))));
                ui.add_space(6.0);
                let mut groups: Vec<(Group, Vec<&Judged>)> = Vec::new();
                for j in &judged {
                    let g = group_of(j);
                    match groups.iter_mut().find(|(x, _)| *x == g) {
                        Some((_, v)) => v.push(j),
                        None => groups.push((g, vec![j])),
                    }
                }
                groups.sort_by_key(|(g, _)| *g);
                egui::ScrollArea::vertical().max_height((ui.available_height() - 60.0).max(120.0)).auto_shrink([false, true]).show(ui, |ui| {
                    for (g, items) in &groups {
                        let (col, title, text) = describe(*g);
                        theme::row(ui, 28.0, |ui| {
                            ui.label(RichText::new("●").color(col).size(13.0));
                            ui.label(RichText::new(title).color(Color32::from_gray(225)));
                            ui.label(muted(format!("· {}", items.len())));
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            ui.add(egui::Label::new(muted(text)).wrap());
                        });
                        let shown: Vec<String> = items.iter().take(6).map(|j| flow_text(&j.flow)).collect();
                        let more = if items.len() > 6 { format!(" и ещё {}", items.len() - 6) } else { String::new() };
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            ui.add(egui::Label::new(RichText::new(format!("{}{more}", shown.join(", "))).color(Color32::from_gray(140)).size(12.0).monospace()).wrap());
                        });
                        ui.add_space(6.0);
                    }
                });
                if self.check.target.is_none() || judged.is_empty() {
                    return;
                }
                let needs_games = judged.iter().any(|j| group_of(j) == Group::NeedsGames);
                let missing = appcheck::addresses(&judged, |j| j.now == Verdict::NotListed || (j.now == Verdict::Unseen && j.with_games == Verdict::NotListed));
                let all = appcheck::addresses(&judged, |j| j.now != Verdict::Excluded);
                let enabled = !self.busy && !running;
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if needs_games && ui.add_enabled(enabled, theme::Primary("Включить игровой фильтр".into())).clicked() {
                        act = Some(Act::Games);
                    }
                    if !missing.is_empty()
                        && ui
                            .add_enabled(enabled, egui::Button::new(format!("Добавить адреса в «Мои IP» · {}", missing.len())))
                            .on_hover_text("Обход начнёт обрабатывать игровой трафик к этим серверам")
                            .clicked()
                    {
                        act = Some(Act::AddMine(missing.clone()));
                    }
                    if !all.is_empty()
                        && ui
                            .add_enabled(enabled, egui::Button::new(format!("Исключить программу · {}", all.len())))
                            .on_hover_text("Адреса её серверов попадут в «Мои исключения IP». Обход перестанет трогать эти адреса для всех программ, а правила по имени сайта продолжат работать")
                            .clicked()
                    {
                        act = Some(Act::Exclude(all.clone()));
                    }
                });
                if running && (needs_games || !missing.is_empty() || !all.is_empty()) {
                    ui.label(muted("Остановите прослушивание, чтобы применить изменения"));
                }
            });
        }
        match act {
            Some(Act::Start) => self.check_start(ctx),
            Some(Act::Stop) => self.check_stop(),
            Some(Act::Games) => self.check_games(ctx),
            Some(Act::AddMine(v)) => self.check_add(ctx, "ipset-all-user.txt", v, "«Мои IP»"),
            Some(Act::Exclude(v)) => self.check_add(ctx, "ipset-exclude-user.txt", v, "«Мои исключения IP»"),
            None => {}
        }
    }

    fn check_games(&mut self, ctx: &egui::Context) {
        let Some(t) = self.check.target.as_ref() else { return };
        let s = t.strategy.clone();
        let installed = self.status.as_ref().is_some_and(|st| st.service != ServiceState::NotInstalled);
        self.spawn(ctx, move |p, tx| {
            let gf = GameFilter { mode: Mode::All, ..gamefilter::load(p) };
            if let Err(e) = gamefilter::save(p, &gf) {
                notify(tx, Lvl::Err, format!("Игровой фильтр: {e}"));
                return;
            }
            if installed {
                result_msg(tx, service::install(p, &s).map(|_| "Игровой фильтр включён, служба переустановлена".into()), "Игровой фильтр включён, но служба не переустановилась");
            } else {
                notify(tx, Lvl::Ok, "Игровой фильтр включён");
            }
            let _ = tx.send(Msg::CheckChanged);
        });
    }

    fn check_add(&mut self, ctx: &egui::Context, file: &'static str, values: Vec<String>, label: &'static str) {
        let Some(t) = self.check.target.as_ref() else { return };
        let s = t.strategy.clone();
        let running = self.status.as_ref().is_some_and(|st| st.service == ServiceState::Running);
        self.spawn(ctx, move |p, tx| {
            let dir = strategy::lists_dir(&s, p);
            let r = (|| -> anyhow::Result<String> {
                lists::ensure_user_files(&dir)?;
                let path = dir.join(file);
                let mut doc = lists::Doc::load(&path);
                let added = doc.add(&values);
                doc.save(&path, Kind::Ips)?;
                if added == 0 {
                    return Ok(format!("Эти адреса уже есть в {label}"));
                }
                if running {
                    service::restart()?;
                    return Ok(format!("Добавлено в {label}: {added}. Служба перезапущена"));
                }
                Ok(format!("Добавлено в {label}: {added}"))
            })();
            result_msg(tx, r, "Не удалось изменить список");
            let _ = tx.send(Msg::CheckChanged);
        });
    }

    pub(super) fn check_changed(&mut self) {
        self.check_reset();
        self.lists_reload();
    }
}
