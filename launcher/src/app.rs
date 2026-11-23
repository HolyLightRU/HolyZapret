mod confirm;
mod diag_tab;
mod lists_tab;
mod phone_tab;
mod picker_tab;
mod rules_ui;
mod share_ui;
mod sites_tab;
mod tools_tab;
mod unpack_ui;
mod tg_ui;
mod trial;
mod warp_ui;
mod whats_new;
mod wizard;

use crate::core::checks::SiteResult;
use crate::core::diagnostics::Finding;
use crate::core::paths::{self, Paths};
use crate::core::service::{self, ServiceState, Status};
use crate::core::strategy::{self, Strategy};
use crate::core::discover::Found;
use crate::core::selfupdate::{self, Release as UpdateRelease};
use crate::core::flowseal::{self, Release};
use crate::core::picker;
use crate::core::settings::{self, Settings};
use crate::core::{gamefilter, update};
use crate::log::{Entry, Lvl, Toast};
use crate::{theme, tray, window};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::mpsc::{channel, Receiver, Sender};
use theme::{CYAN, GREEN, PINK, RED, YELLOW};

const TOAST_LIFE: f64 = 4.5;

enum Msg {
    Status(Status),
    Log(Lvl, String),
    Toast(Lvl, String),
    Site(SiteResult),
    SitesDone,
    Diag(Vec<Finding>),
    Version(Result<UpdateRelease, String>),
    Found(Result<Found, String>),
    FlowsealLatest(Result<Release, String>),
    Pick(picker::Event),
    Rules(crate::core::rules::Event),
    Rescan,
    Updated,
    Autostart(bool),
    Warp(crate::core::warp::State),
    TgForeign(Vec<String>),
    TgClient(Option<crate::core::tgproxy::control::Client>),
    PostInstallCheck,
    TrialStarted(Result<trial::Trial, String>),
    ResetDone,
    Shared,
    WhatsNew(String, String),
    Health(Vec<String>),
    Relocated(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf),
    Unpacked(Result<(std::path::PathBuf, std::path::PathBuf, bool), String>),
    Done,
}

fn health_loop(
    p: Paths,
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    force: std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: Sender<Msg>,
    ctx: egui::Context,
) {
    use std::sync::atomic::Ordering;
    const STEP: u64 = 5;
    const FIRST: u64 = 300;
    const EVERY: u64 = 1200;
    let targets = crate::core::health::targets(&p);
    let mut watch = crate::core::health::Watch::default();
    let mut strategy: Option<String> = None;
    let mut wait = FIRST;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(STEP));
        let forced = force.swap(false, Ordering::Relaxed);
        wait = wait.saturating_sub(STEP);
        if !forced && (wait > 0 || !flag.load(Ordering::Relaxed)) {
            continue;
        }
        let st = service::status(&p);
        let active = (st.service == ServiceState::Running && st.engine_running).then(|| st.active_strategy().map(String::from)).flatten();
        if active != strategy {
            strategy = active.clone();
            if !watch.current().is_empty() {
                if tx.send(Msg::Health(Vec::new())).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
            watch = Default::default();
        }
        if active.is_none() || tray::busy() {
            wait = 60;
            continue;
        }
        wait = EVERY;
        let results = crate::core::health::probe(&targets);
        let changed = watch.step(&results).or_else(|| forced.then(|| watch.current()));
        if let Some(failing) = changed {
            if !failing.is_empty() {
                tray::notify(
                    "Обход перестал работать",
                    &format!("Не открываются {}. Откройте HolyZapret, чтобы подобрать стратегию заново", failing.join(", ")),
                );
            }
            if tx.send(Msg::Health(failing)).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    }
}

fn send(tx: &Sender<Msg>, lvl: Lvl, text: impl Into<String>) {
    let _ = tx.send(Msg::Log(lvl, text.into()));
}

fn notify(tx: &Sender<Msg>, lvl: Lvl, text: impl Into<String>) {
    let _ = tx.send(Msg::Toast(lvl, text.into()));
}

fn result_msg(tx: &Sender<Msg>, r: anyhow::Result<String>, err_prefix: &str) {
    match r {
        Ok(m) => notify(tx, Lvl::Ok, m),
        Err(e) => notify(tx, Lvl::Err, format!("{err_prefix}: {e}")),
    }
}

fn category_label(c: &str) -> String {
    match c {
        "additional" => "Additional".into(),
        "experimental" => "Эксперимент".into(),
        other => other.to_string(),
    }
}

fn plural(n: usize, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    let (m10, m100) = (n % 10, n % 100);
    if m10 == 1 && m100 != 11 {
        one
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        few
    } else {
        many
    }
}

fn lvl_color(l: Lvl) -> Color32 {
    match l {
        Lvl::Info => CYAN,
        Lvl::Ok => GREEN,
        Lvl::Err => RED,
    }
}

fn lvl_icon(l: Lvl) -> &'static str {
    match l {
        Lvl::Info => theme::icon::INFO,
        Lvl::Ok => theme::icon::CHECK,
        Lvl::Err => theme::icon::ERROR,
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Wizard,
    Main,
    Picker,
    Sites,
    Diagnostics,
    Tools,
    Telegram,
    Phone,
    Lists,
    Log,
}

pub struct App {
    paths: Option<Paths>,
    strategies: Vec<Strategy>,
    selected: Option<usize>,
    filter: String,
    status: Option<Status>,
    tab: Tab,
    log: Vec<Entry>,
    log_filter: usize,
    toasts: Vec<Toast>,
    category: usize,
    preselected: bool,
    busy: bool,
    sites: Vec<SiteResult>,
    sites_running: bool,
    sites_total: usize,
    sites_announce: Option<&'static str>,
    trial: Option<trial::Trial>,
    wizard: wizard::WizardState,
    wizard_done: bool,
    autostart: Option<bool>,
    last_pick: Option<crate::core::pickstore::LastPick>,
    warp: Option<crate::core::warp::State>,
    tg: tg_ui::TgState,
    flow_last: f64,
    flow: bool,
    flow_phase: f32,
    last_action: f64,
    confirm: Option<confirm::Confirm>,
    site_input: String,
    sites_filter: usize,
    game_edit: Option<(gamefilter::GameFilter, String, String)>,
    game_err: Option<String>,
    diag: diag_tab::DiagState,
    local_version: Option<String>,
    remote_version: Option<String>,
    remote_release: Option<UpdateRelease>,
    version_error: Option<String>,
    close_to_tray: bool,
    sidebar_open: bool,
    sidebar_peek: bool,
    advanced: bool,
    saved: Settings,
    lists: lists_tab::ListsState,
    picker: picker_tab::PickerState,
    rules: rules_ui::RulesState,
    whats_new: Option<(String, String)>,
    watch: bool,
    watch_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    health_force: std::sync::Arc<std::sync::atomic::AtomicBool>,
    health_alert: Vec<String>,
    old_folder: Option<std::path::PathBuf>,
    unpack: Option<unpack_ui::UnpackState>,
    share: share_ui::ShareState,
    phone: phone_tab::PhoneState,
    flowseal_version: Option<String>,
    flowseal_latest: Option<Release>,
    flowseal_checking: bool,
    window_ready: bool,
    tray: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, icon: Option<egui::IconData>) -> Self {
        theme::apply(&cc.egui_ctx);
        tray::set_ctx(&cc.egui_ctx);
        let (tx, rx) = channel();
        let paths = paths::find_root().map(|root| Paths { root });
        let strategies = paths.as_ref().map(strategy::discover).unwrap_or_default();
        let local_version = paths.as_ref().and_then(update::local_version);
        let flowseal_version = paths.as_ref().and_then(flowseal::installed);
        let tray = icon.is_some_and(|i| tray::create(i.rgba, i.width, i.height, paths.clone()));
        let saved = settings::load();
        let mut app = Self {
            paths,
            strategies,
            selected: None,
            filter: String::new(),
            status: None,
            tab: Tab::Main,
            log: Vec::new(),
            log_filter: 0,
            toasts: Vec::new(),
            category: 0,
            preselected: false,
            busy: false,
            sites: Vec::new(),
            sites_running: false,
            sites_total: 0,
            sites_announce: None,
            trial: None,
            wizard: Default::default(),
            wizard_done: saved.wizard_done,
            autostart: None,
            last_pick: crate::core::pickstore::load(),
            warp: None,
            tg: tg_ui::TgState::detect(),
            flow_last: 0.0,
            flow: saved.flow,
            flow_phase: 0.0,
            last_action: 0.0,
            confirm: None,
            site_input: String::new(),
            sites_filter: 0,
            game_edit: None,
            game_err: None,
            diag: Default::default(),
            local_version,
            remote_version: None,
            remote_release: None,
            version_error: None,
            close_to_tray: saved.close_to_tray && tray,
            sidebar_open: saved.sidebar_open,
            sidebar_peek: false,
            advanced: saved.advanced,
            saved,
            lists: Default::default(),
            picker: Default::default(),
            rules: Default::default(),
            whats_new: None,
            watch: saved.watch,
            watch_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(saved.watch)),
            health_force: Default::default(),
            health_alert: Vec::new(),
            old_folder: None,
            unpack: None,
            share: Default::default(),
            phone: Default::default(),
            flowseal_version,
            flowseal_latest: None,
            flowseal_checking: false,
            window_ready: false,
            tray,
            tx,
            rx,
        };
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == crate::core::relocate::MOVED_ARG) {
            let old = args.get(i + 1).cloned().unwrap_or_default();
            let root = app.paths.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
            app.log.push(Entry::new(Lvl::Ok, format!("HolyZapret перенесён в {root}. Старую папку {old} можно удалить")));
            app.toasts.push(crate::log::Toast { lvl: Lvl::Ok, text: format!("HolyZapret перенесён в {root}"), born: 0.0 });
            let old = std::path::PathBuf::from(&old);
            if app.paths.as_ref().is_some_and(|p| crate::core::relocate::can_remove_old(&old, &p.root)) {
                app.old_folder = Some(old);
                app.confirm = Some(confirm::Confirm::DeleteOld);
            }
        }
        if args.iter().any(|a| a == crate::core::unpack::UNPACKED_ARG) {
            let root = app.paths.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
            app.log.push(Entry::new(Lvl::Ok, format!("HolyZapret распакован в {root}")));
            app.toasts.push(crate::log::Toast { lvl: Lvl::Ok, text: format!("HolyZapret распакован в {root}"), born: 0.0 });
        }
        if app.paths.is_none() {
            app.unpack = unpack_ui::UnpackState::detect();
            app.log.push(Entry::new(Lvl::Err, "Не найдена папка с bin\\HolyZapret.exe — положите лаунчер внутрь HolyZapret"));
        }
        match crate::core::lanproxy::control::autostart() {
            Some(Ok(m)) => app.log.push(Entry::new(Lvl::Ok, m)),
            Some(Err(e)) => app.log.push(Entry::new(Lvl::Err, format!("Раздача на телефон не запустилась: {e}"))),
            None => {}
        }
        match crate::core::tgproxy::control::autostart() {
            Some(Ok(m)) => app.log.push(Entry::new(Lvl::Ok, m)),
            Some(Err(e)) => app.log.push(Entry::new(Lvl::Err, format!("Telegram-прокси не запустился: {e}"))),
            None => {}
        }
        if let Ok(h) = cc.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                tray::set_hwnd(w.hwnd.get());
                crate::instance::remember(w.hwnd.get());
                window::apply_backdrop(cc, w.hwnd.get());
                app.window_ready = true;
            }
        }
        let hidden_start = std::env::args().any(|a| a == crate::core::autostart::TRAY_ARG);
        if hidden_start && !app.tray {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        {
            let (tx, ctx) = (app.tx.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Autostart(crate::core::autostart::enabled()));
                let _ = tx.send(Msg::Warp(crate::core::warp::query()));
                let _ = tx.send(Msg::TgForeign(tg_ui::foreign_names()));
                ctx.request_repaint();
            });
        }
        app.refresh(&cc.egui_ctx);
        app.check_version(&cc.egui_ctx);
        let current = env!("CARGO_PKG_VERSION");
        match settings::seen_version().or_else(|| app.wizard_done.then(|| "0".to_string())) {
            _ if crate::core::TEST_BUILD || app.paths.is_none() => {}
            None => settings::set_seen_version(current),
            Some(seen) if selfupdate::is_newer(&seen, current) => {
                let (tx, ctx) = (app.tx.clone(), cc.egui_ctx.clone());
                std::thread::spawn(move || {
                    if let Ok(body) = selfupdate::notes(current) {
                        let _ = tx.send(Msg::WhatsNew(current.to_string(), body));
                        ctx.request_repaint();
                    }
                });
            }
            Some(_) => {}
        }
        if let Some(p) = app.paths.clone() {
            std::thread::spawn(move || {
                selfupdate::cleanup_old(&p);
            });
        }
        if !app.wizard_done && app.paths.is_some() {
            app.open_wizard();
        }
        if let Some(p) = app.paths.clone() {
            let (tx, ctx) = (app.tx.clone(), cc.egui_ctx.clone());
            let (flag, force) = (app.watch_flag.clone(), app.health_force.clone());
            std::thread::spawn(move || health_loop(p, flag, force, tx, ctx));
        }
        if let Some(p) = app.paths.clone() {
            let (tx, ctx) = (app.tx.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || {
                let mut tick: u32 = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    tick = tick.wrapping_add(1);
                    let st = service::status(&p);
                    tray::publish(&st);
                    if tick % 3 == 0 {
                        let _ = tx.send(Msg::Warp(crate::core::warp::query()));
                    }
                    let _ = tx.send(Msg::TgForeign(tg_ui::foreign_names()));
                    let _ = tx.send(Msg::TgClient(crate::core::tgproxy::control::desktop_client()));
                    if tx.send(Msg::Status(st)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        app
    }

    fn refresh(&self, ctx: &egui::Context) {
        let Some(p) = self.paths.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Status(service::status(&p)));
            ctx.request_repaint();
        });
    }

    fn update_available(&self) -> Option<String> {
        if crate::core::TEST_BUILD {
            return None;
        }
        let r = self.remote_version.as_ref()?;
        let l = self.local_version.clone().unwrap_or_default();
        selfupdate::is_newer(&l, r).then(|| r.clone())
    }

    fn check_version(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = selfupdate::latest().map_err(|e| e.to_string());
            let _ = tx.send(Msg::Version(r));
            ctx.request_repaint();
        });
    }

    fn spawn<F>(&mut self, ctx: &egui::Context, job: F)
    where
        F: FnOnce(&Paths, &Sender<Msg>) + Send + 'static,
    {
        let Some(p) = self.paths.clone() else { return };
        self.busy = true;
        self.last_action = ctx.input(|i| i.time);
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            job(&p, &tx);
            let _ = tx.send(Msg::Done);
            let _ = tx.send(Msg::Status(service::status(&p)));
            ctx.request_repaint();
        });
    }

    fn note(&mut self, lvl: Lvl, text: impl Into<String>) {
        self.log.push(Entry::new(lvl, text));
    }

    fn toast(&mut self, ctx: &egui::Context, lvl: Lvl, text: impl Into<String>) {
        let text = text.into();
        self.log.push(Entry::new(lvl, text.clone()));
        let born = ctx.input(|i| i.time);
        self.toasts.push(Toast { lvl, text, born });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Status(s) => {
                    let was_on = self.status.as_ref().is_some_and(|x| x.service == ServiceState::Running);
                    let quiet = ctx.input(|i| i.time) - self.last_action > 15.0 && tray::seconds_since_action() > 15.0;
                    if was_on && s.service != ServiceState::Running && quiet && !self.busy && !self.picker.running() {
                        self.toast(ctx, Lvl::Err, "Защита отключилась: служба zapret больше не работает");
                    }
                    tray::publish(&s);
                    if !self.preselected {
                        if let Some(n) = s.active_strategy() {
                            self.selected = self.strategies.iter().position(|x| x.matches(n));
                        }
                        self.preselected = true;
                    }
                    self.status = Some(s);
                }
                Msg::Log(l, t) => self.note(l, t),
                Msg::Toast(l, t) => self.toast(ctx, l, t),
                Msg::Site(r) => self.sites.push(r),
                Msg::SitesDone => {
                    self.sites_running = false;
                    if let Some(what) = self.sites_announce.take() {
                        let ok = self.sites.iter().filter(|s| s.ok).count();
                        let total = self.sites.len();
                        let failed: Vec<String> =
                            self.sites.iter().filter(|s| !s.ok).take(3).map(|s| sites_tab::host_of(&s.url)).collect();
                        if failed.is_empty() {
                            self.toast(ctx, Lvl::Ok, format!("{what}: доступны все {total} сайтов"));
                        } else {
                            self.toast(
                                ctx,
                                Lvl::Err,
                                format!("{what}: доступно {ok} из {total}, не открываются {}", failed.join(", ")),
                            );
                        }
                    }
                }
                Msg::PostInstallCheck => {
                    self.sites_announce = Some("Проверка после установки");
                    self.start_site_check(ctx);
                }
                Msg::TrialStarted(Ok(t)) => {
                    self.toast(ctx, Lvl::Ok, format!("Пробный запуск «{}»: проверяю сайты…", t.name));
                    self.trial = Some(t);
                    self.sites_announce = Some("Пробный запуск");
                    self.start_site_check(ctx);
                }
                Msg::TrialStarted(Err(e)) => self.toast(ctx, Lvl::Err, format!("Пробный запуск: {e}")),
                Msg::Diag(d) => {
                    self.diag.findings = d;
                    self.diag.running = false;
                }
                Msg::Found(Ok(f)) => self.lists.found(f),
                Msg::Found(Err(e)) => {
                    self.lists.search_failed();
                    self.toast(ctx, Lvl::Err, format!("Поиск не удался: {e}"));
                }
                Msg::Rules(e) => self.on_rules_event(ctx, e),
                Msg::Pick(e) => {
                    let fin = matches!(e, picker::Event::Finished { .. } | picker::Event::Failed(_));
                    let completed = matches!(e, picker::Event::Finished { cancelled: false, .. });
                    if let Some((l, m)) = self.picker.on_event(e) {
                        self.toast(ctx, l, m);
                    }
                    if completed {
                        let snap = self.picker.snapshot();
                        if !snap.scores.is_empty() {
                            if let Err(e) = crate::core::pickstore::save(&snap) {
                                self.note(Lvl::Err, format!("Не удалось сохранить результаты подбора: {e}"));
                            }
                            self.last_pick = Some(snap);
                        }
                    }
                    if fin {
                        self.busy = false;
                        self.refresh(ctx);
                    }
                }
                Msg::FlowsealLatest(r) => {
                    self.flowseal_checking = false;
                    match r {
                        Ok(rel) => self.flowseal_latest = Some(rel),
                        Err(e) => self.note(Lvl::Err, format!("Flowseal: не удалось узнать последнюю версию: {e}")),
                    }
                }
                Msg::Autostart(on) => self.autostart = Some(on),
                Msg::Warp(w) => self.warp = Some(w),
                Msg::TgForeign(f) => self.tg.foreign = f,
                Msg::TgClient(c) => {
                    if c.is_none() && self.tab == Tab::Telegram {
                        self.tab = Tab::Tools;
                    }
                    self.tg.client = c;
                }
                Msg::Rescan => self.rescan(),
                Msg::Updated => {
                    self.local_version = self.paths.as_ref().and_then(update::local_version);
                    self.rescan();
                }
                Msg::Done => self.busy = false,
                Msg::WhatsNew(v, body) => self.whats_new = Some((v, body)),
                Msg::Relocated(root, exe, old) => {
                    let spawned = std::process::Command::new(&exe)
                        .arg(crate::core::relocate::MOVED_ARG)
                        .arg(&old)
                        .current_dir(&root)
                        .spawn();
                    match spawned {
                        Ok(_) => tray::exit(),
                        Err(e) => self.toast(ctx, Lvl::Err, format!("Папка перенесена в {}, но новый лаунчер не запустился: {e}", root.display())),
                    }
                }
                Msg::Unpacked(r) => self.on_unpacked(ctx, r),
                Msg::Health(failing) => {
                    if failing.is_empty() {
                        if !self.health_alert.is_empty() {
                            self.toast(ctx, Lvl::Ok, "Обход снова работает");
                        }
                    } else if failing == self.health_alert {
                        self.toast(ctx, Lvl::Err, format!("Обход всё ещё не работает: не открываются {}", failing.join(", ")));
                    } else {
                        self.note(Lvl::Err, format!("Обход перестал работать: не открываются {}", failing.join(", ")));
                    }
                    self.health_alert = failing;
                }
                Msg::Shared => {
                    self.game_edit = None;
                    self.lists = Default::default();
                    self.rules = Default::default();
                    self.rescan();
                    self.refresh(ctx);
                }
                Msg::ResetDone => {
                    let d = Settings::default();
                    self.advanced = d.advanced;
                    self.close_to_tray = d.close_to_tray && self.tray;
                    self.sidebar_open = d.sidebar_open;
                    self.flow = d.flow;
                    self.watch = d.watch;
                    self.health_alert.clear();
                    self.last_pick = None;
                    self.game_edit = None;
                    self.lists = Default::default();
                    self.rules = Default::default();
                    self.tg.forget();
                    self.sites.clear();
                    self.sites_total = 0;
                    self.rescan();
                    self.refresh(ctx);
                    self.open_wizard();
                }
                Msg::Version(Ok(v)) => {
                    self.version_error = None;
                    self.remote_version = Some(v.version.clone());
                    self.remote_release = Some(v);
                }
                Msg::Version(Err(e)) => {
                    self.note(Lvl::Err, format!("Проверка версии: {e}"));
                    self.version_error = Some(e);
                }
            }
        }
    }

    fn save_game_filter(&mut self, ctx: &egui::Context, gf: gamefilter::GameFilter) {
        let Some(p) = &self.paths else { return };
        match gamefilter::save(p, &gf) {
            Ok(()) => {
                self.game_edit = Some((gf.clone(), gf.tcp.clone(), gf.udp.clone()));
                self.game_err = None;
                let msg = if self.service_installed() {
                    format!("Игровой фильтр: {}. Переустановите службу, чтобы применить", gf.summary().to_lowercase())
                } else {
                    format!("Игровой фильтр: {}", gf.summary().to_lowercase())
                };
                self.toast(ctx, Lvl::Info, msg);
            }
            Err(e) => self.toast(ctx, Lvl::Err, format!("Игровой фильтр: {e}")),
        }
        self.refresh(ctx);
    }

    fn rescan(&mut self) {
        self.picker.invalidate();
        self.flowseal_version = self.paths.as_ref().and_then(flowseal::installed);
        let keep = self.selected.and_then(|i| self.strategies.get(i)).map(|s| s.name.clone());
        self.strategies = self.paths.as_ref().map(strategy::discover).unwrap_or_default();
        self.selected = keep.and_then(|n| self.strategies.iter().position(|s| s.name == n));
    }

    fn categories(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in &self.strategies {
            if !v.contains(&s.category) {
                v.push(s.category.clone());
            }
        }
        v
    }

    fn install_selected(&mut self, ctx: &egui::Context) {
        let Some(s) = self.selected.and_then(|i| self.strategies.get(i)).cloned() else {
            self.toast(ctx, Lvl::Info, "Выберите стратегию в списке");
            return;
        };
        self.spawn(ctx, move |p, tx| {
            send(tx, Lvl::Info, format!("Устанавливаю службу: {}", s.name));
            match service::install(p, &s) {
                Ok(m) => {
                    notify(tx, Lvl::Ok, m);
                    let mut others: Vec<String> = crate::core::rules::load(p)
                        .into_iter()
                        .map(|r| r.origin)
                        .filter(|o| !o.is_empty() && !s.matches(o))
                        .collect();
                    others.dedup();
                    if !others.is_empty() {
                        notify(
                            tx,
                            Lvl::Info,
                            format!("Часть правил для сайтов подобрана под «{}»: перепроверьте их кнопкой «Перепроверить» на вкладке «Сайты»", others.join("», «")),
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    let _ = tx.send(Msg::PostInstallCheck);
                }
                Err(e) => notify(tx, Lvl::Err, format!("Ошибка установки: {e}")),
            }
        });
    }

    fn power(&mut self, ctx: &egui::Context, state: ServiceState) {
        match state {
            ServiceState::Running => self.confirm = Some(confirm::Confirm::PowerOff),
            ServiceState::Stopped => self.spawn(ctx, |_, tx| {
                result_msg(tx, service::start().map(|_| "Служба запущена".to_string()), "Не удалось запустить службу")
            }),
            ServiceState::NotInstalled => self.install_selected(ctx),
        }
    }

    fn setup_window(&mut self, frame: &eframe::Frame) {
        if self.window_ready {
            return;
        }
        if let Ok(h) = frame.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                tray::set_hwnd(w.hwnd.get());
                crate::instance::remember(w.hwnd.get());
                window::apply_backdrop(frame, w.hwnd.get());
            }
        }
        self.window_ready = true;
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, expand: f32) {
        ui.add_space(4.0);
        let mut items = vec![
            (Tab::Main, theme::icon::HOME, "Главная"),
            (Tab::Picker, theme::icon::PICK, "Подбор"),
        ];
        if self.tg.client.is_some() {
            items.push((Tab::Telegram, theme::icon::TELEGRAM, "Telegram"));
        }
        items.push((Tab::Phone, theme::icon::PHONE, "Телефон"));
        items.extend([
            (Tab::Sites, theme::icon::GLOBE, "Сайты"),
            (Tab::Diagnostics, theme::icon::DIAGNOSTIC, "Диагностика"),
            (Tab::Tools, theme::icon::TOOLS, "Инструменты"),
        ]);
        if self.advanced {
            items.push((Tab::Lists, theme::icon::LISTS, "Списки"));
        }
        items.push((Tab::Log, theme::icon::LOG, "Журнал"));
        for (t, glyph, name) in items {
            if theme::nav_item(ui, self.tab == t, glyph, name, expand).clicked() {
                self.tab = t;
            }
        }
        if expand < 0.6 {
            return;
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(6.0);
            if let Some(r) = self.update_available().filter(|_| self.paths.is_some()) {
                theme::chip(ui, &format!("Доступна {r}"), YELLOW);
            }
        });
    }

    fn hero(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(s) = self.status.clone() else {
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Читаю состояние…");
                });
            });
            return;
        };
        let running = s.service == ServiceState::Running;
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                let tip = match s.service {
                    ServiceState::Running => "Выключить: удалить службу",
                    ServiceState::Stopped => "Запустить службу",
                    ServiceState::NotInstalled => "Установить выбранную стратегию",
                };
                let resp = theme::power_button(ui, running, self.busy || tray::busy()).on_hover_text(tip);
                if resp.clicked() && !self.busy && !tray::busy() && self.paths.is_some() {
                    self.power(ctx, s.service.clone());
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    let title = match s.service {
                        ServiceState::Running => "Защита активна",
                        ServiceState::Stopped => "Служба остановлена",
                        ServiceState::NotInstalled => "Защита выключена",
                    };
                    ui.label(RichText::new(title).size(22.0).strong().color(Color32::WHITE));
                    let sub = match (s.foreign_dir(), s.active_strategy()) {
                        (Some(d), _) => format!(
                            "Служба из другой папки: {}{}",
                            d.display(),
                            s.installed_strategy.as_deref().map(|n| format!(" · {n}")).unwrap_or_default()
                        ),
                        (None, Some(n)) => format!("Стратегия: {n}"),
                        _ => "Выберите стратегию и нажмите на кнопку старта".to_string(),
                    };
                    ui.label(RichText::new(sub).color(Color32::from_gray(160)));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if s.foreign_dir().is_some() {
                            theme::chip(ui, "Чужая служба", YELLOW)
                                .on_hover_text("Скорее всего это оригинальный Flowseal. Её можно удалить кнопкой старта или в Диагностике");
                        } else if !s.foreign_engines.is_empty() {
                            theme::chip(ui, "Запущен другой обход", YELLOW).on_hover_text(
                                s.foreign_engines.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join("\n"),
                            );
                        } else if s.engine_running {
                            theme::chip(ui, "Движок работает", GREEN);
                        } else {
                            theme::chip(ui, "Движок остановлен", RED);
                        }
                        let (c, t) = match s.windivert {
                            ServiceState::Running => (GREEN, "WinDivert загружен"),
                            ServiceState::Stopped => (YELLOW, "WinDivert на паузе"),
                            ServiceState::NotInstalled => (RED, "WinDivert не загружен"),
                        };
                        theme::chip(ui, t, c);
                        if s.windivert != ServiceState::NotInstalled
                            && !s.engine_running
                            && theme::icon_button(ui, theme::icon::EJECT, "Выгрузить драйвер WinDivert").clicked()
                            && !self.busy
                        {
                            self.spawn(ctx, |_, tx| {
                                result_msg(
                                    tx,
                                    service::unload_windivert().map(|_| "Драйвер WinDivert выгружен".to_string()),
                                    "Не удалось выгрузить WinDivert",
                                )
                            });
                        }
                        if theme::icon_button(ui, theme::icon::REFRESH, "Обновить статус").clicked() {
                            self.refresh(ctx);
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut on = s.game.enabled();
                    if theme::toggle(ui, &mut on).changed() {
                        let mut gf = s.game.clone();
                        gf.mode = if on { gamefilter::Mode::All } else { gamefilter::Mode::Disabled };
                        self.save_game_filter(ctx, gf);
                    }
                    ui.add_space(4.0);
                    ui.allocate_ui_with_layout(egui::vec2(140.0, 40.0), egui::Layout::top_down(egui::Align::Max), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label("Игровой фильтр");
                        ui.label(RichText::new(s.game.summary()).weak().size(12.5));
                    });
                });
            });
        });
    }

    fn main_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.health_card(ui, ctx);
        self.trial_card(ui, ctx);
        self.hero(ui, ctx);
        self.warp_row(ui, ctx);
        let cats = self.categories();
        let pick_tab = self.last_pick.as_ref().is_some_and(|p| self.strategies.iter().any(|s| p.scores.contains_key(&s.path)));
        let offset = if pick_tab { 2 } else { 1 };
        let mut labels = vec!["Все".to_string()];
        if pick_tab {
            labels.push("По подбору".to_string());
        }
        labels.extend(cats.iter().map(|c| category_label(c)));
        if self.category >= labels.len() {
            self.category = 0;
        }
        let installed = self.status.as_ref().and_then(|s| s.active_strategy().map(String::from));
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Стратегии").size(17.0).strong());
                ui.label(RichText::new(format!("{}", self.strategies.len())).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Поиск").desired_width(190.0));
                });
            });
            theme::segmented(ui, &labels, &mut self.category);
            let pick_view = pick_tab && self.category == 1;
            if pick_view {
                let when = self.last_pick.as_ref().map(|p| p.when.clone()).unwrap_or_default();
                ui.label(RichText::new(format!("Лучшие сверху, по подбору от {when}")).color(Color32::from_gray(150)).size(12.5));
            }
            ui.add_space(2.0);
            let filter = self.filter.to_lowercase();
            let cat = if self.category < offset { None } else { cats.get(self.category - offset).cloned() };
            let scores = self.last_pick.as_ref().map(|p| p.scores.clone()).unwrap_or_default();
            let mut order: Vec<usize> = (0..self.strategies.len()).collect();
            if pick_view {
                order.retain(|&i| scores.contains_key(&self.strategies[i].path));
                order.sort_by_key(|&i| scores[&self.strategies[i].path].key());
            }
            let mut chosen = self.selected;
            let mut install_now = false;
            let h = (ui.available_height() - 58.0).max(80.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let mut shown = 0;
                for &i in &order {
                    let s = &self.strategies[i];
                    if cat.as_ref().is_some_and(|c| c != &s.category) {
                        continue;
                    }
                    if !filter.is_empty() && !s.name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    shown += 1;
                    let mut meta = format!("{} · {} {}", s.lists_label, s.blocks, plural(s.blocks, "блок", "блока", "блоков"));
                    if s.game_filter {
                        meta.push_str(" · игры");
                    }
                    let active = installed.as_deref().is_some_and(|n| s.matches(n));
                    let score = scores.get(&s.path).map(|sc| {
                        if sc.failed {
                            ("не запустилась".to_string(), RED)
                        } else {
                            let color = if sc.ok == sc.total {
                                GREEN
                            } else if sc.ok * 5 >= sc.total * 4 {
                                YELLOW
                            } else {
                                RED
                            };
                            (format!("{}/{}", sc.ok, sc.total), color)
                        }
                    });
                    let resp = theme::strategy_row(ui, &s.name, &meta, chosen == Some(i), active, score);
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                    if resp.double_clicked() && !self.busy {
                        chosen = Some(i);
                        install_now = true;
                    }
                }
                if shown == 0 {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Ничего не найдено").weak());
                    });
                }
            });
            self.selected = chosen;
            if install_now {
                self.install_selected(ctx);
            }
            ui.add_space(4.0);
            let bar = egui::vec2(ui.available_width(), 40.0);
            ui.allocate_ui_with_layout(bar, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let sel_s = self.selected.and_then(|i| self.strategies.get(i)).cloned();
                let sel = sel_s.as_ref().map(|s| s.name.clone());
                let has_service = self.status.as_ref().is_some_and(|s| s.service != ServiceState::NotInstalled);
                if ui.add_enabled(has_service && !self.busy, egui::Button::new("Удалить службу")).clicked() {
                    self.confirm = Some(confirm::Confirm::RemoveService);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reinstall = sel_s.as_ref().is_some_and(|s| installed.as_deref().is_some_and(|n| s.matches(n)));
                    let label = if reinstall { "Переустановить" } else { "Установить службу" };
                    ui.add_enabled_ui(sel.is_some() && !self.busy && self.paths.is_some(), |ui| {
                        if ui.add(theme::Primary(label.into())).clicked() {
                            self.install_selected(ctx);
                        }
                    });
                    let can_try = sel_s.is_some() && !self.busy && self.trial.is_none() && self.paths.is_some();
                    if ui
                        .add_enabled(can_try, egui::Button::new("Попробовать"))
                        .on_hover_text("Запустить стратегию без установки службы и проверить сайты")
                        .clicked()
                    {
                        self.start_trial(ctx);
                    }
                    if ui.button("Подобрать").on_hover_text("Проверить стратегии по очереди и найти лучшую").clicked() {
                        self.tab = Tab::Picker;
                    }
                });
            });
        });
    }

    fn log_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        theme::card(ui, |ui| {
            theme::row(ui, 36.0, |ui| {
                ui.label(RichText::new("Журнал").size(17.0).strong());
                ui.label(RichText::new(format!("{}", self.log.len())).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, theme::icon::DELETE, "Очистить журнал").clicked() {
                        self.log.clear();
                    }
                    if theme::icon_button(ui, theme::icon::COPY, "Скопировать всё").clicked() {
                        let text: Vec<String> =
                            self.log.iter().map(|e| format!("{} [{}] {}", e.time, e.lvl.tag(), e.text)).collect();
                        ctx.copy_text(text.join("\r\n"));
                        self.toast(ctx, Lvl::Ok, "Журнал скопирован");
                    }
                });
            });
            let labels: Vec<String> = ["Все", "Успех", "Ошибки", "Инфо"].iter().map(|s| s.to_string()).collect();
            theme::segmented(ui, &labels, &mut self.log_filter);
            let want = match self.log_filter {
                1 => Some(Lvl::Ok),
                2 => Some(Lvl::Err),
                3 => Some(Lvl::Info),
                _ => None,
            };
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let mut any = false;
                for e in self.log.iter().filter(|e| want.is_none_or(|w| w == e.lvl)) {
                    any = true;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(5));
                    let y = rect.center().y;
                    p.text(
                        egui::pos2(rect.left() + 10.0, y),
                        egui::Align2::LEFT_CENTER,
                        &e.time,
                        egui::FontId::monospace(12.5),
                        Color32::from_gray(120),
                    );
                    let color = lvl_color(e.lvl);
                    let pill = theme::paint_pill(p, egui::pos2(rect.left() + 128.0, y), e.lvl.tag(), color);
                    let galley = p.layout(
                        e.text.clone(),
                        egui::FontId::proportional(14.0),
                        Color32::from_gray(220),
                        rect.right() - pill.right() - 24.0,
                    );
                    let gy = y - galley.rows.first().map_or(galley.size().y, |r| r.rect().height()) / 2.0;
                    p.with_clip_rect(rect).galley(egui::pos2(pill.right() + 12.0, gy), galley, Color32::from_gray(220));
                }
                if !any {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Здесь появятся события лаунчера").weak());
                    });
                }
            });
        });
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| now - t.born < TOAST_LIFE);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
        let mut dismiss = None;
        egui::Area::new(egui::Id::NULL.with("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -18.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                for (i, t) in self.toasts.iter().enumerate() {
                    let age = now - t.born;
                    let alpha = ((TOAST_LIFE - age) / 0.4).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0);
                    ui.scope(|ui| {
                        ui.multiply_opacity(alpha as f32);
                        let color = lvl_color(t.lvl);
                        let r = Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(22, 20, 28, 245))
                            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(24)))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin { left: 16, right: 14, top: 10, bottom: 10 })
                            .show(ui, |ui| {
                                ui.set_max_width(320.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(lvl_icon(t.lvl)).font(theme::icon_font(15.0)).color(color));
                                    ui.add(egui::Label::new(RichText::new(&t.text).color(Color32::from_gray(230))).wrap());
                                });
                            });
                        let rect = r.response.rect;
                        let bar = egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 5.0, rect.top() + 9.0),
                            egui::pos2(rect.left() + 8.0, rect.bottom() - 9.0),
                        );
                        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
                        if ui.interact(rect, egui::Id::NULL.with(("toast", i)), egui::Sense::click()).clicked() {
                            dismiss = Some(i);
                        }
                    });
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

}

fn line(p: &egui::Painter, a: egui::Pos2, b: egui::Pos2) {
    p.line_segment([a, b], egui::Stroke::new(1.2, Color32::from_gray(225)));
}

fn title_bar(ui: &mut egui::Ui, ctx: &egui::Context, sidebar_open: &mut bool, version: &str) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let full = ui.available_rect_before_wrap();
    let drag = ui.interact(full, ui.make_persistent_id("title_drag"), egui::Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    let v = egui::vec2;
    ui.allocate_ui_with_layout(full.size(), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(9.0);
        let (resp, r) = theme::chrome_button(ui, false);
        let c = r.center();
        let p = ui.painter();
        for dy in [-5.0, 0.0, 5.0] {
            line(p, c + v(-7.0, dy), c + v(7.0, dy));
        }
        if resp.on_hover_text("Свернуть или развернуть меню").clicked() {
            *sidebar_open = !*sidebar_open;
        }
        title_text(ui, full.center().x, c.y, version);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (resp, r) = theme::chrome_button(ui, true);
            let c = r.center();
            line(ui.painter(), c + v(-5.0, -5.0), c + v(5.0, 5.0));
            line(ui.painter(), c + v(-5.0, 5.0), c + v(5.0, -5.0));
            if resp.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            let (resp, r) = theme::chrome_button(ui, false);
            let c = r.center();
            let stroke = egui::Stroke::new(1.2, Color32::from_gray(225));
            if maximized {
                let back = egui::Rect::from_center_size(c + v(1.5, -1.5), v(8.0, 8.0));
                let front = egui::Rect::from_center_size(c + v(-1.0, 1.0), v(8.0, 8.0));
                line(ui.painter(), back.left_top(), back.right_top());
                line(ui.painter(), back.right_top(), back.right_bottom());
                ui.painter().rect_stroke(front, CornerRadius::ZERO, stroke, egui::StrokeKind::Middle);
            } else {
                let rect = egui::Rect::from_center_size(c, v(10.0, 10.0));
                ui.painter().rect_stroke(rect, CornerRadius::ZERO, stroke, egui::StrokeKind::Middle);
            }
            if resp.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            let (resp, r) = theme::chrome_button(ui, false);
            let c = r.center();
            line(ui.painter(), c + v(-5.0, 0.0), c + v(5.0, 0.0));
            if resp.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        });
    });
}

fn baseline(g: &egui::Galley) -> f32 {
    g.rows.first().map(|r| r.pos.y + r.row.glyphs.first().map(|gl| gl.pos.y).unwrap_or(0.0)).unwrap_or(0.0)
}

fn title_text(ui: &egui::Ui, center_x: f32, center_y: f32, version: &str) {
    const SIZE: f32 = 16.0;
    const CAP: f32 = 0.7;
    let p = ui.painter();
    let title = p.layout_job(theme::gradient_text("HolyZapret", SIZE, PINK, CYAN));
    let ver = p.layout_no_wrap(format!("v{version}"), egui::FontId::proportional(13.0), Color32::from_gray(140));
    let gap = 6.0;
    let width = title.size().x + gap + ver.size().x;
    let line = (center_y + SIZE * CAP / 2.0).round();
    let left = (center_x - width / 2.0).round();
    let t_top = line - baseline(&title);
    let v_top = line - baseline(&ver);
    p.galley(egui::pos2(left, t_top), title.clone(), Color32::WHITE);
    p.galley(egui::pos2(left + title.size().x + gap, v_top), ver, Color32::from_gray(140));
}

fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.content_rect();
    let m = 6.0;
    let left = pos.x < r.left() + m;
    let right = pos.x > r.right() - m;
    let top = pos.y < r.top() + m;
    let bottom = pos.y > r.bottom() - m;
    use egui::{CursorIcon, ResizeDirection as D};
    let hit = match (left, right, top, bottom) {
        (true, _, true, _) => Some((D::NorthWest, CursorIcon::ResizeNwSe)),
        (_, true, true, _) => Some((D::NorthEast, CursorIcon::ResizeNeSw)),
        (true, _, _, true) => Some((D::SouthWest, CursorIcon::ResizeNeSw)),
        (_, true, _, true) => Some((D::SouthEast, CursorIcon::ResizeNwSe)),
        (true, ..) => Some((D::West, CursorIcon::ResizeHorizontal)),
        (_, true, ..) => Some((D::East, CursorIcon::ResizeHorizontal)),
        (_, _, true, _) => Some((D::North, CursorIcon::ResizeVertical)),
        (_, _, _, true) => Some((D::South, CursorIcon::ResizeVertical)),
        _ => None,
    };
    if let Some((dir, cursor)) = hit {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tray::clear_notice();
        self.stop_trial_now();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.setup_window(frame);
        self.poll(&ctx);
        if tray::take_phone() && self.paths.is_some() {
            self.tab = Tab::Phone;
        }
        if tray::take_quit() {
            self.confirm = Some(confirm::Confirm::QuitSharing);
        }
        let tray_events = tray::take_events();
        if !tray_events.is_empty() {
            for (ok, text) in tray_events {
                self.toast(&ctx, if ok { Lvl::Ok } else { Lvl::Err }, text);
            }
            self.refresh(&ctx);
        }
        if tray::busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let now = ctx.input(|i| i.time);
        let dt = (now - self.flow_last).clamp(0.0, 0.1);
        self.flow_last = now;
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if self.flow && focused && theme::animations_allowed() && tray::window_visible() {
            self.flow_phase = (self.flow_phase + (dt / theme::FLOW_PERIOD) as f32).rem_euclid(1.0);
            theme::set_flow(self.flow_phase);
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            if self.close_to_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if !tray::hide_window() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            } else if crate::core::lanproxy::control::running() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(confirm::Confirm::QuitSharing);
            } else if self.picker.running() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.toast(
                    &ctx,
                    Lvl::Info,
                    "Идёт подбор: дождитесь окончания или отмените его, чтобы служба вернулась в прежнее состояние",
                );
            }
        }

        theme::backdrop(&ctx.layer_painter(egui::LayerId::background()), ctx.content_rect());

        egui::Panel::top("title")
            .exact_size(36.0)
            .frame(Frame::new().fill(Color32::from_black_alpha(30)))
            .show(ui, |ui| {
                let mut version = self.local_version.clone().unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
                if crate::core::TEST_BUILD {
                    version.push_str(" · тестовая сборка");
                }
                title_bar(ui, &ctx, &mut self.sidebar_open, &version)
            });

        let expand = ctx.animate_bool_responsive(ui.make_persistent_id("sidebar_anim"), self.sidebar_open);
        let panel = egui::Panel::left("nav")
            .exact_size(64.0 + 136.0 * expand)
            .resizable(false)
            .frame(Frame::new().fill(Color32::from_black_alpha(70)).inner_margin(Margin::same(12)))
            .show(ui, |ui| self.sidebar(ui, expand));
        if !self.sidebar_open && self.confirm.is_none() {
            let base = panel.response.rect;
            let peek = ctx.animate_bool_with_time(ui.make_persistent_id("sidebar_peek"), self.sidebar_peek, 0.16);
            let width = 64.0 + 136.0 * peek;
            let zone = egui::Rect::from_min_size(base.min, egui::vec2(width, base.height()));
            self.sidebar_peek = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| zone.contains(p));
            if peek > 0.01 {
                egui::Area::new(egui::Id::NULL.with("nav_peek")).order(egui::Order::Middle).fixed_pos(base.min).show(&ctx, |ui| {
                    Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(20, 16, 26, 246))
                        .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(18 + (10.0 * peek) as u8)))
                        .shadow(egui::epaint::Shadow { offset: [6, 0], blur: 18, spread: 0, color: Color32::from_black_alpha((90.0 * peek) as u8) })
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(width - 24.0);
                            ui.set_height(base.height() - 24.0);
                            self.sidebar(ui, peek);
                        });
                });
            }
        } else {
            self.sidebar_peek = false;
        }

        resize_edges(&ctx);

        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::same(16)).corner_radius(CornerRadius::ZERO))
            .show(ui, |ui| match self.tab {
                Tab::Wizard => self.wizard_tab(ui, &ctx),
                Tab::Main => self.main_tab(ui, &ctx),
                Tab::Picker => self.picker_tab(ui, &ctx),
                Tab::Sites => self.sites_tab(ui, &ctx),
                Tab::Diagnostics => self.diag_tab(ui, &ctx),
                Tab::Tools => self.tools_tab(ui, &ctx),
                Tab::Telegram => self.tg_tab(ui, &ctx),
                Tab::Phone => self.phone_tab(ui, &ctx),
                Tab::Lists => self.lists_tab(ui, &ctx),
                Tab::Log => self.log_tab(ui, &ctx),
            });
        self.draw_toasts(&ctx);
        self.draw_confirm(&ctx);
        self.draw_unpack(&ctx);
        self.draw_share(&ctx);
        if self.confirm.is_none() && self.tab != Tab::Wizard && self.unpack.is_none() {
            self.draw_whats_new(&ctx);
        }
        let now = Settings {
            advanced: self.advanced,
            close_to_tray: self.close_to_tray,
            sidebar_open: self.sidebar_open,
            wizard_done: self.wizard_done,
            flow: self.flow,
            watch: self.watch,
        };
        if now != self.saved {
            settings::save(&now);
            self.saved = now;
            self.watch_flag.store(self.watch, std::sync::atomic::Ordering::Relaxed);
            if !self.watch {
                self.health_alert.clear();
            }
        }
    }
}
