// momputer: a mom for you, your computer and your AIs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mom_core::containers::{self, Machines};
use mom_core::kill::{self, KillReport};
use mom_core::procs::Sampler;
use mom_core::scan::{Group, Status};
use mom_core::util::{human_bytes, now_secs};
use mom_core::history::{self, kind};
use mom_core::{hooks, rules, tidy, trash, Config, Paths, Snapshot};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

const TRAY_CALM: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_ALERT: &[u8] = include_bytes!("../icons/tray-alert.png");

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Note {
    at: u64,
    text: String,
}

struct Shared {
    paths: Paths,
    config: Mutex<Config>,
    sampler: Mutex<Sampler>,
    snapshot: Mutex<Option<Snapshot>>,
    trash: Mutex<Vec<trash::Spot>>,
    trash_busy: AtomicBool,
    /// Asked for while something was being measured: spots to measure again right after ("" is
    /// all of them), so a change made meanwhile (something to keep, more to list) isn't lost.
    trash_next: Mutex<HashSet<String>>,
    /// Spots listing more than the usual number of items, and how many times as many.
    trash_scale: Mutex<HashMap<String, usize>>,
    /// The clean-up running now: how it's going, and its stop button.
    cleaning: trash::Live,
    /// Looking for copies and big forgotten files, and its stop button.
    files_busy: AtomicBool,
    files_stop: AtomicBool,
    /// Docker, WSL and Hyper-V. Asking them can take many seconds, so it happens on its own thread.
    machines: Mutex<Machines>,
    machines_busy: AtomicBool,
    machines_at: Mutex<Option<Instant>>,
    rooms: Mutex<BTreeMap<String, tidy::RoomReport>>,
    /// How each agent was set up when mom last looked. A file can only be read if it's in here.
    agents: Mutex<Vec<mom_core::setup::AgentSetup>>,
    paused: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    leftover_since: Mutex<HashMap<String, u64>>,
    notified: Mutex<Option<HashSet<String>>>,
    last_toast: Mutex<u64>,
    last_alarm: Mutex<u64>,
    /// When leftovers started burning more CPU than the alarm allows, if they still are.
    melting_since: Mutex<Option<u64>>,
    tray_state: Mutex<(bool, String)>,
    diary: Mutex<VecDeque<Note>>,
    placed: AtomicBool,
    /// The stand-in card says it's drawn (BIG MODE's change of size waits for that).
    card_ready: (Mutex<bool>, Condvar),
    /// When the new shape has opened, while a change of size is under way.
    morph_until: Mutex<Option<Instant>>,
    /// The stand-in is on the screen.
    card_up: AtomicBool,
}

type S = Arc<Shared>;

/// A panic elsewhere must never leave mom unable to read her own notes.
trait Lk<T> {
    fn lk(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> Lk<T> for Mutex<T> {
    fn lk(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Shared {
    fn cfg(&self) -> Config {
        self.config.lk().clone()
    }
    fn note(&self, text: impl Into<String>) {
        let mut d = self.diary.lk();
        d.push_front(Note { at: now_secs(), text: text.into() });
        d.truncate(60);
    }
    fn rescan_now(&self) {
        let (lock, cv) = &self.wake;
        *lock.lk() = true;
        cv.notify_all();
    }
}

// ------------------------------------------------------------------ window & tray

/// Where a widget of this size sits: by the tray, a little in from the corner.
fn widget_spot(monitor: &tauri::Monitor, size: tauri::PhysicalSize<u32>) -> tauri::PhysicalPosition<i32> {
    let area = monitor.work_area();
    let margin = (16.0 * monitor.scale_factor()) as i32;
    let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    tauri::PhysicalPosition::new(x.max(area.position.x), y.max(area.position.y))
}

fn place_window(win: &tauri::WebviewWindow) {
    if let (Ok(Some(monitor)), Ok(size)) = (win.primary_monitor(), win.outer_size()) {
        let _ = win.set_position(widget_spot(&monitor, size));
    }
}

/// How long the change of shape takes on the stand-in: the old shape closing, the new one opening.
const MORPH: Duration = Duration::from_millis(300);
/// The page's content fades out before its window is swapped for the stand-in, and the stand-in
/// dissolves once the window is back under it.
const FADE_OUT: Duration = Duration::from_millis(90);
const DISSOLVE: Duration = Duration::from_millis(140);

/// The stand-in for BIG MODE's change of size: a see-through window that's never clicked or
/// typed in, with two plain cards on it. Made once, a moment after mom starts, and kept hidden.
///
/// Why a stand-in: a real window can't be resized smoothly. Every size on the way makes its page
/// draw itself again, a few dozen times a second at best, with scrollbars flashing by, and the
/// page's surface lags behind the frame, which leaves scraps on the desktop. So the real window
/// isn't resized on screen at all. A card is put exactly over it, the window is taken off the
/// screen and sent to its new place and size unseen, the cards do what a window does when it
/// closes and opens (the graphics card moves them, at the screen's own rate), and the window is
/// put back under the new card, which dissolves into it.
fn make_card(app: &AppHandle) {
    if app.get_webview_window("glide").is_some() {
        return;
    }
    let built = tauri::WebviewWindowBuilder::new(app, "glide", tauri::WebviewUrl::App("glide.html".into()))
        .title("momputer")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .inner_size(320.0, 240.0)
        .build();
    if let Ok(card) = built {
        let _ = card.set_ignore_cursor_events(true);
    }
}

/// Puts the stand-in over the window, takes the window off the screen, starts the change of
/// shape and sends the real window to its new place, unseen. False if it couldn't (no stand-in,
/// or it didn't answer in time): then nothing has changed and the caller does it the plain way.
/// `size`: the page's new size. `place`: where a window of that whole size goes.
#[cfg(windows)]
fn morph(app: &AppHandle, s: &S, win: &tauri::WebviewWindow, scale: f64, bg: &str, size: tauri::PhysicalSize<u32>, place: impl Fn(tauri::PhysicalSize<u32>) -> tauri::PhysicalPosition<i32>) -> bool {
    use mom_core::util::window;
    let asked = Instant::now();
    let (Some(card), Ok(hwnd), Ok(inner)) = (app.get_webview_window("glide"), win.hwnd(), win.inner_size()) else { return false };
    let Ok(card_hwnd) = card.hwnd() else { return false };
    let (hwnd, card_hwnd) = (hwnd.0 as isize, card_hwnd.0 as isize);
    let (Some(whole), Some(seen)) = (window::whole(hwnd), window::seen(hwnd)) else { return false };
    // The unseen edge Windows keeps around a window: the same before and after.
    let edge = (seen.0 - whole.0, seen.1 - whole.1, whole.2 - seen.2, whole.3 - seen.3);
    let goal = tauri::PhysicalSize::new((size.width as i32 + whole.2 - inner.width as i32).max(1) as u32, (size.height as i32 + whole.3 - inner.height as i32).max(1) as u32);
    let to = place(goal);
    let seen_there = (to.x + edge.0, to.y + edge.1, goal.width as i32 - edge.2, goal.height as i32 - edge.3);
    // The stand-in's window covers where the window is and where it's going, with room for a shadow.
    const ROOM: i32 = 56;
    let (left, top) = (seen.0.min(seen_there.0) - ROOM, seen.1.min(seen_there.1) - ROOM);
    let (right, bottom) = ((seen.0 + seen.2).max(seen_there.0 + seen_there.2) + ROOM, (seen.1 + seen.3).max(seen_there.1 + seen_there.3) + ROOM);
    window::place(card_hwnd, left, top, right - left, bottom - top);
    let Some(page) = window::page(card_hwnd) else { return false };
    // In the page's own units, measured from its corner.
    let on_page = |r: (i32, i32, i32, i32)| serde_json::json!({ "x": f64::from(r.0 - page.0) / scale, "y": f64::from(r.1 - page.1) / scale, "w": f64::from(r.2) / scale, "h": f64::from(r.3) / scale });
    // The colour comes from the widget's page: only what a colour is made of gets through.
    let bg: String = bg.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | '(' | ')' | ',' | '.' | ' ' | '%')).take(40).collect();
    *s.card_ready.0.lk() = false;
    if card.eval(format!("glide.at({}, {}, {})", on_page(seen), on_page(seen_there), serde_json::json!({ "bg": bg }))).is_err() {
        return false;
    }
    // The page under it is fading its content out meanwhile: by the time that's done, the old
    // shape is on the glass, exactly over the window and the same colour, so the swap can't be seen.
    window::show_quietly(card_hwnd, true);
    let _ = card.eval("glide.ack()");
    let on_glass = {
        let (lock, cv) = &s.card_ready;
        let (ready, _) = cv.wait_timeout_while(lock.lk(), Duration::from_millis(300), |ready| !*ready).unwrap_or_else(|e| e.into_inner());
        *ready
    };
    if !on_glass {
        let _ = card.eval("glide.done()");
        window::show_quietly(card_hwnd, false);
        return false;
    }
    std::thread::sleep(FADE_OUT.saturating_sub(asked.elapsed()));
    window::cloak(hwnd, true);
    s.card_up.store(true, Ordering::SeqCst);
    let _ = card.eval(format!("glide.go({})", MORPH.as_millis()));
    // (A moment more than it takes: it's told to go by a message, which takes a moment too.)
    *s.morph_until.lk() = Some(Instant::now() + MORPH + Duration::from_millis(30));
    window::place(hwnd, to.x, to.y, goal.width as i32, goal.height as i32);
    let _ = win.set_size(size);
    let _ = win.set_position(to);
    // Whatever happens to the widget's page meanwhile, the window comes back.
    let (app, s) = (app.clone(), s.clone());
    std::thread::spawn(move || {
        std::thread::sleep(MORPH + Duration::from_secs(2));
        arrive(&app, &s);
        card_gone(&app, &s);
    });
    true
}

#[cfg(not(windows))]
fn morph(_: &AppHandle, _: &S, _: &tauri::WebviewWindow, _: f64, _: &str, _: tauri::PhysicalSize<u32>, _: impl Fn(tauri::PhysicalSize<u32>) -> tauri::PhysicalPosition<i32>) -> bool {
    false
}

/// The new shape has opened (or it's time it had): the real window is put back under it. Safe to
/// call twice.
fn arrive(app: &AppHandle, s: &S) {
    let Some(until) = s.morph_until.lk().take() else { return };
    std::thread::sleep(until.saturating_duration_since(Instant::now()));
    #[cfg(windows)]
    if let Some(Ok(hwnd)) = app.get_webview_window("main").map(|w| w.hwnd()) {
        mom_core::util::window::cloak(hwnd.0 as isize, false);
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// The stand-in dissolves into the window under it and is put away. Safe to call twice.
fn card_gone(app: &AppHandle, s: &S) {
    if !s.card_up.swap(false, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    if let Some(card) = app.get_webview_window("glide") {
        let _ = card.eval(format!("glide.away({})", DISSOLVE.as_millis()));
        std::thread::sleep(DISSOLVE + Duration::from_millis(20));
        let _ = card.eval("glide.done()");
        if let Ok(h) = card.hwnd() {
            mom_core::util::window::show_quietly(h.0 as isize, false);
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let s = app.state::<S>();
        if !s.placed.swap(true, Ordering::SeqCst) {
            place_window(&win);
        }
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn toggle_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) && win.is_focused().unwrap_or(false) {
            let _ = win.hide();
        } else {
            show_main(app);
        }
    }
}

fn update_tray(app: &AppHandle, s: &Shared, snap: &Snapshot) {
    let alert = snap.summary.leftovers > 0;
    let tip = if snap.summary.leftovers == 0 {
        "momputer · all tidy".to_string()
    } else {
        format!(
            "momputer · {} leftover{} · {}",
            snap.summary.leftovers,
            if snap.summary.leftovers == 1 { "" } else { "s" },
            human_bytes(snap.summary.leftover_mem)
        )
    };
    let mut last = s.tray_state.lk();
    if last.0 == alert && last.1 == tip {
        return;
    }
    if let Some(tray) = app.tray_by_id("mom") {
        if last.0 != alert || last.1.is_empty() {
            if let Ok(img) = tauri::image::Image::from_bytes(if alert { TRAY_ALERT } else { TRAY_CALM }) {
                let _ = tray.set_icon(Some(img));
            }
        }
        let _ = tray.set_tooltip(Some(&tip));
    }
    *last = (alert, tip);
}

/// Windows names a notification after the app id it was sent with. The notification plugin only
/// uses ours once the app has been through the installer, so a momputer run from anywhere else
/// would speak as "Windows PowerShell". Mom registers her own name and picture instead.
#[cfg(windows)]
mod toasts {
    use std::path::Path;

    pub const APP_ID: &str = "dev.momputer.app";
    const PICTURE: &[u8] = include_bytes!("../icons/128x128@2x.png");
    const FACE: &[u8] = include_bytes!("../icons/toast.png");

    /// Mom's round face, shown big in every notification.
    pub fn face(state_dir: &Path) -> std::path::PathBuf {
        state_dir.join("mom-face.png")
    }

    fn keep(path: &Path, bytes: &[u8]) {
        if std::fs::read(path).map(|b| b != bytes).unwrap_or(true) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
            let _ = std::fs::write(path, bytes);
        }
    }

    pub fn register(state_dir: &Path) {
        let picture = state_dir.join("mom.png");
        keep(&picture, PICTURE);
        keep(&face(state_dir), FACE);
        if let Ok(key) = windows_registry::CURRENT_USER.create(format!(r"SOFTWARE\Classes\AppUserModelId\{APP_ID}")) {
            let _ = key.set_string("DisplayName", "mom");
            let _ = key.set_string("IconBackgroundColor", "0");
            let _ = key.set_string("IconUri", picture.to_string_lossy());
        }
    }
}

fn toast(app: &AppHandle, title: &str, body: &str) {
    #[cfg(windows)]
    {
        let handle = app.clone();
        let face = toasts::face(&app.state::<S>().paths.state_dir());
        let sent = tauri_winrt_notification::Toast::new(toasts::APP_ID)
            .icon(&face, tauri_winrt_notification::IconCrop::Circular, "mom")
            .title(title)
            .text1(body)
            .on_activated(move |_| {
                show_main(&handle);
                Ok(())
            })
            .show();
        if sent.is_ok() {
            return;
        }
    }
    let _ = app.notification().builder().title(title).body(body).show();
}

// ------------------------------------------------------------------ the watcher

fn scanner(app: AppHandle, s: S) {
    let mut codex = mom_core::codex::CodexState::default();
    let mut codex_at: Option<Instant> = None;
    let mut stumbles = 0u32;
    loop {
        let cfg = s.cfg();
        if !s.paused.load(Ordering::Relaxed) {
            let looked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                look_once(&app, &s, &cfg, &mut codex, &mut codex_at)
            }));
            weekly(&app, &s);
            if looked.is_err() {
                stumbles += 1;
                if stumbles <= 3 {
                    s.note("I tripped over something while looking around. Still watching.");
                }
            }
        }
        let (lock, cv) = &s.wake;
        let guard = lock.lk();
        let interval = Duration::from_secs(cfg.ui.scan_interval_secs.clamp(1, 60));
        let (mut woke, _) = cv.wait_timeout_while(guard, interval, |w| !*w).unwrap_or_else(|e| e.into_inner());
        *woke = false;
    }
}

/// Ask Docker, WSL and Hyper-V what's running, without making the watcher wait for the answer.
fn refresh_machines(s: &S, table: &mom_core::procs::ProcTable) {
    let due = s.machines_at.lk().map(|t| t.elapsed() > Duration::from_secs(20)).unwrap_or(true);
    if !due || s.machines_busy.swap(true, Ordering::SeqCst) {
        return;
    }
    *s.machines_at.lk() = Some(Instant::now());
    let (s, table) = (s.clone(), table.clone());
    std::thread::spawn(move || {
        let m = containers::machines(&table);
        *s.machines.lk() = m;
        s.machines_busy.store(false, Ordering::SeqCst);
    });
}

fn look_once(app: &AppHandle, s: &S, cfg: &Config, codex: &mut mom_core::codex::CodexState, codex_at: &mut Option<Instant>) {
    {
        {
            let table = s.sampler.lk().sample();
            let listeners = mom_core::ports::listeners();
            refresh_machines(s, &table);
            let machines = s.machines.lk().clone();
            if codex_at.map(|t| t.elapsed() > Duration::from_secs(10)).unwrap_or(true) {
                *codex = mom_core::codex::state(&s.paths.codex_dir());
                *codex_at = Some(Instant::now());
            }
            let snap = mom_core::build_snapshot(&table, &listeners, cfg, machines.clone());
            let stopped = auto_stop(app, s, cfg, &table, &snap);
            let mut snap = if stopped {
                let table = s.sampler.lk().sample();
                mom_core::build_snapshot(&table, &mom_core::ports::listeners(), cfg, machines.clone())
            } else {
                snap
            };
            mom_core::attach_codex(&mut snap, codex.clone());
            alerts(app, s, cfg, &snap);
            waiting_tick(app, s, cfg);
            update_tray(app, s, &snap);
            let _ = app.emit("snapshot", &snap);
            *s.snapshot.lk() = Some(snap);
        }
    }
}

fn auto_stop(app: &AppHandle, s: &Shared, cfg: &Config, table: &mom_core::procs::ProcTable, snap: &Snapshot) -> bool {
    let now = now_secs();
    let mut since = s.leftover_since.lk();
    let current: HashSet<&str> = snap.groups.iter().filter(|g| g.is_leftover()).map(|g| g.id.as_str()).collect();
    since.retain(|id, _| current.contains(id.as_str()));
    for id in &current {
        since.entry(id.to_string()).or_insert(now);
    }
    if !cfg.leftovers.auto_stop {
        return false;
    }
    let grace = cfg.leftovers.auto_stop_after_minutes * 60;
    let due: Vec<&Group> = snap
        .groups
        .iter()
        .filter(|g| {
            g.is_leftover()
                && (g.status == Status::LeftBehind || cfg.leftovers.auto_stop_orphans)
                && since.get(&g.id).map(|t| now.saturating_sub(*t) >= grace).unwrap_or(false)
        })
        .collect();
    if due.is_empty() {
        return false;
    }
    let names: Vec<String> = due.iter().take(4).map(|g| g.name.clone()).collect();
    let report = kill::stop_groups(table, &due);
    if report.groups == 0 {
        return false;
    }
    history::stopped(&s.paths, &names, &report, true, now);
    let text = format!(
        "Turned off {} thing{} nobody came back for: {}{}. {} back.",
        report.groups,
        if report.groups == 1 { "" } else { "s" },
        names.join(", "),
        if due.len() > 4 { ", …" } else { "" },
        human_bytes(report.freed_mem)
    );
    s.note(text.clone());
    if cfg.ui.notifications && cfg.ui.notify_cleanups {
        toast(app, "Mom cleaned up", &text);
    }
    true
}

/// Tells you about leftovers without nagging. A leftover has to sit there a while first (agents
/// often tidy up a minute later), everything new is gathered into one note, and there's at most
/// one note per `notify_every_minutes`. The CPU alarm needs the heat to last, and rings hourly at most.
fn alerts(app: &AppHandle, s: &Shared, cfg: &Config, snap: &Snapshot) {
    let now = now_secs();
    let ui = &cfg.ui;
    let leftovers: Vec<&Group> = snap.groups.iter().filter(|g| g.is_leftover()).collect();
    let ids: HashSet<String> = leftovers.iter().map(|g| g.id.clone()).collect();
    let mut notified = s.notified.lk();
    let Some(seen) = notified.as_mut() else {
        // First look after starting: just remember what's there; the tray says the rest.
        *notified = Some(ids);
        return;
    };
    seen.retain(|id| ids.contains(id));

    let wait = ui.notify_after_minutes * 60;
    let ripe: Vec<&Group> = {
        let since = s.leftover_since.lk();
        leftovers
            .iter()
            .copied()
            .filter(|g| !seen.contains(&g.id) && since.get(&g.id).is_some_and(|t| now.saturating_sub(*t) >= wait))
            .collect()
    };
    let mut last_toast = s.last_toast.lk();
    let gap = ui.notify_every_minutes.max(1) * 60;
    if ui.notifications && ui.notify_leftovers && !ripe.is_empty() && now.saturating_sub(*last_toast) >= gap {
        let agents: HashSet<&str> = ripe.iter().map(|g| g.owner.agent.map(|a| a.nickname()).unwrap_or("Somebody")).collect();
        let who = if agents.len() == 1 { agents.into_iter().next().unwrap_or("Somebody") } else { "Your agents" };
        let what: Vec<String> = ripe
            .iter()
            .take(3)
            .map(|g| {
                let port = g.ports.first().map(|l| format!(" :{}", l.port)).unwrap_or_default();
                format!("{}{port}", g.name)
            })
            .collect();
        let more = if ripe.len() > 3 { format!(" and {} more", ripe.len() - 3) } else { String::new() };
        let mem: u64 = ripe.iter().map(|g| g.mem).sum();
        let body = format!(
            "{who} left {} thing{} running: {}{more} ({}). Open momputer and I'll turn {} off.",
            ripe.len(),
            if ripe.len() == 1 { "" } else { "s" },
            what.join(", "),
            human_bytes(mem),
            if ripe.len() == 1 { "it" } else { "them" }
        );
        toast(app, if ripe.len() == 1 { "Something is still running" } else { "A few things are still running" }, &body);
        *last_toast = now;
        seen.extend(ripe.iter().map(|g| g.id.clone()));
    }

    let mut melting_since = s.melting_since.lk();
    if snap.summary.leftover_cpu >= ui.cpu_alert_percent {
        melting_since.get_or_insert(now);
    } else {
        *melting_since = None;
    }
    let mut last_alarm = s.last_alarm.lk();
    let lasting = melting_since.is_some_and(|t| now.saturating_sub(t) >= 120);
    if ui.notifications && ui.notify_meltdown && lasting && now.saturating_sub(*last_alarm) >= 3600 {
        let body = format!(
            "Leftovers have been using {:.0}% of your CPU for a while ({} things). Open momputer and I'll clear them.",
            snap.summary.leftover_cpu, snap.summary.leftovers
        );
        toast(app, "Your computer is working hard", &body);
        s.note(format!("Said so: leftovers at {:.0}% CPU.", snap.summary.leftover_cpu));
        *last_alarm = now;
    }
}

// ------------------------------------------------------------------ commands

type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command(async)]
fn get_snapshot(s: State<'_, S>) -> Option<Snapshot> {
    s.snapshot.lk().clone()
}

#[tauri::command(async)]
fn refresh(s: State<'_, S>) {
    s.rescan_now();
}

fn stop_where(s: &Shared, pick: impl Fn(&Group) -> bool) -> KillReport {
    let cfg = s.cfg();
    let table = s.sampler.lk().sample();
    let analysis = mom_core::scan::analyze(&table, &mom_core::ports::listeners(), &cfg);
    let chosen: Vec<&Group> = analysis.groups.iter().filter(|g| pick(g)).collect();
    // How to start the servers again, before they're gone.
    let since = now_secs();
    mom_core::restart::remember(&s.paths, &table, &chosen);
    let report = kill::stop_groups(&table, &chosen);
    history::stopped(&s.paths, &chosen.iter().map(|g| g.name.clone()).collect::<Vec<_>>(), &report, false, since);
    if report.groups > 0 {
        let names: Vec<&str> = chosen.iter().take(4).map(|g| g.name.as_str()).collect();
        s.note(format!("Turned off {} ({} back).", names.join(", "), human_bytes(report.freed_mem)));
    }
    s.rescan_now();
    report
}

#[tauri::command]
async fn stop_groups(ids: Vec<String>, s: State<'_, S>) -> Res<KillReport> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || stop_where(&s, |g| ids.contains(&g.id))).await.map_err(err)
}

#[tauri::command]
async fn stop_leftovers(s: State<'_, S>) -> Res<KillReport> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || stop_where(&s, |g| g.is_leftover())).await.map_err(err)
}

/// Servers mom stopped that she can start again.
#[tauri::command(async)]
fn get_stopped(s: State<'_, S>) -> Vec<mom_core::restart::Stopped> {
    mom_core::restart::list(&s.paths)
}

/// Starts a stopped server again in a new terminal window, in its folder.
#[tauri::command(async)]
fn start_again(id: String, s: State<'_, S>) -> Res<String> {
    let msg = mom_core::restart::start(&s.paths, &id)?;
    history::did(&s.paths, kind::STARTED, msg.clone());
    s.note(msg.clone());
    s.rescan_now();
    Ok(msg)
}

/// Forgets a stopped server.
#[tauri::command(async)]
fn forget_stopped(id: String, s: State<'_, S>) -> Res<()> {
    mom_core::restart::forget(&s.paths, &id).map_err(err)
}

#[tauri::command(async)]
fn keep_group(signature: String, keep: bool, s: State<'_, S>) -> Res<()> {
    let mut cfg = s.config.lk();
    cfg.kept.retain(|k| k != &signature);
    if keep {
        cfg.kept.push(signature);
    }
    cfg.save(&s.paths).map_err(err)?;
    drop(cfg);
    s.rescan_now();
    Ok(())
}

#[tauri::command]
async fn stop_container(id: String, s: State<'_, S>) -> Res<()> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        containers::docker_stop(std::slice::from_ref(&id)).map_err(err)?;
        history::did(&s.paths, kind::STOPPED, format!("Stopped container {id}"));
        s.note(format!("Stopped container {id}."));
        s.rescan_now();
        Ok(())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn stop_wsl(name: Option<String>, s: State<'_, S>) -> Res<()> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        match &name {
            Some(n) => containers::wsl_terminate(n).map_err(err)?,
            None => containers::wsl_shutdown().map_err(err)?,
        }
        history::did(&s.paths, kind::STOPPED, name.as_deref().map_or("Shut WSL down".to_string(), |n| format!("Put WSL distro {n} to bed")));
        s.note(match name {
            Some(n) => format!("Put WSL distro {n} to bed."),
            None => "Shut WSL down.".into(),
        });
        s.rescan_now();
        Ok(())
    })
    .await
    .map_err(err)?
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashState {
    spots: Vec<trash::Spot>,
    busy: bool,
    /// Every clean-up mom has is in `spots` (a saved measurement from an older version isn't).
    complete: bool,
}

fn trash_state(s: &Shared) -> TrashState {
    let spots = s.trash.lk().clone();
    let complete = trash::spot_ids().iter().all(|id| spots.iter().any(|x| x.id == *id));
    TrashState { spots, busy: s.trash_busy.load(Ordering::Relaxed), complete }
}

#[tauri::command(async)]
fn get_trash(s: State<'_, S>) -> TrashState {
    trash_state(&s)
}

fn emit_trash(app: &AppHandle, s: &Shared) {
    let _ = app.emit("trash", trash_state(s));
}

fn measure_trash_now(app: AppHandle, s: S, only: Option<String>) {
    {
        // Busy already: this one is next. (Both sides hold `trash_next` while they look at
        // `trash_busy`, so a request can't slip in just as the measuring ends and be forgotten.)
        let mut next = s.trash_next.lk();
        if s.trash_busy.swap(true, Ordering::SeqCst) {
            next.insert(only.unwrap_or_default());
            return;
        }
    }
    std::thread::spawn(move || {
        let mut only = only;
        loop {
            // Settings as they are now: what to keep may have changed since the last round.
            let cfg = s.cfg();
            let table = s.sampler.lk().sample();
            let mut ctx = trash::TrashCtx::new(&s.paths, &cfg, &table);
            let ids: Vec<String> = match only.take() {
                Some(id) => vec![id],
                None => trash::spot_ids().into_iter().map(String::from).collect(),
            };
            for id in ids {
                ctx.list_scale = s.trash_scale.lk().get(&id).copied().unwrap_or(1);
                if let Some(spot) = trash::measure(&id, &ctx) {
                    let mut list = s.trash.lk();
                    match list.iter_mut().find(|x| x.id == spot.id) {
                        Some(x) => *x = spot,
                        None => list.push(spot),
                    }
                }
                emit_trash(&app, &s);
            }
            let mut next = s.trash_next.lk();
            if next.is_empty() {
                s.trash_busy.store(false, Ordering::SeqCst);
                break;
            }
            // All of them if anyone asked for that; otherwise one spot now, the rest after it.
            only = if next.remove("") {
                next.clear();
                None
            } else {
                let id = next.iter().next().cloned();
                if let Some(id) = &id {
                    next.remove(id);
                }
                id
            };
        }
        trash::save(&s.paths, &s.trash.lk());
        emit_trash(&app, &s);
    });
}

#[tauri::command(async)]
fn measure_trash(app: AppHandle, s: State<'_, S>) {
    measure_trash_now(app, s.inner().clone(), None);
}

/// Measures one spot again (the refresh button on its sheet). With `more`, it lists four times
/// as many items as it does now ("Show all").
#[tauri::command(async)]
fn measure_spot(id: String, more: Option<bool>, app: AppHandle, s: State<'_, S>) {
    if more == Some(true) && trash::spot_ids().contains(&id.as_str()) {
        let mut scale = s.trash_scale.lk();
        let now = scale.get(&id).copied().unwrap_or(1);
        scale.insert(id.clone(), (now * trash::MORE).min(trash::MAX_SCALE));
    }
    measure_trash_now(app, s.inner().clone(), Some(id));
}

/// Keeps one thing from now on, whatever clean-up finds it (or, with `keep` false, lets it go
/// again), then measures the spot it was in again. Without a spot (from Settings), the spots
/// that list it.
#[tauri::command(async)]
fn keep_item(path: String, keep: bool, spot: Option<String>, app: AppHandle, s: State<'_, S>) -> Res<()> {
    {
        let mut cfg = s.config.lk();
        let before = cfg.trash.keep.clone();
        // It's kept once it's written down. If it can't be (or can't be kept at all), nothing
        // changes and the widget is told, so it never shows something as kept that isn't.
        let changed = trash::set_keep(&mut cfg.trash, &path, keep)?;
        if changed {
            if let Err(why) = cfg.save(&s.paths) {
                cfg.trash.keep = before;
                return Err(format!("Couldn't save that, so nothing changed: {why}"));
            }
        }
    }
    let ids: Vec<String> = match spot {
        Some(id) => vec![id],
        None => {
            let want = mom_core::util::norm_path(&path);
            s.trash.lk().iter().filter(|x| x.items.iter().any(|i| mom_core::util::norm_path(&i.path) == want)).map(|x| x.id.clone()).collect()
        }
    };
    for id in ids {
        measure_trash_now(app.clone(), s.inner().clone(), Some(id));
    }
    Ok(())
}

/// Lets go of everything you said to keep, and measures again.
#[tauri::command(async)]
fn keep_nothing(app: AppHandle, s: State<'_, S>) -> Res<()> {
    {
        let mut cfg = s.config.lk();
        if cfg.trash.keep.is_empty() {
            return Ok(());
        }
        let before = std::mem::take(&mut cfg.trash.keep);
        if let Err(why) = cfg.save(&s.paths) {
            cfg.trash.keep = before;
            return Err(format!("Couldn't save that, so nothing changed: {why}"));
        }
    }
    measure_trash_now(app, s.inner().clone(), None);
    Ok(())
}

/// How the clean-up running now is going: counts, the item at hand, and what's gone since
/// `since` (what the widget already has).
#[tauri::command(async)]
fn clean_progress(since: usize, s: State<'_, S>) -> trash::LiveNow {
    s.cleaning.now(since)
}

/// Stops the clean-up running now after what it's doing.
#[tauri::command(async)]
fn stop_clean(s: State<'_, S>) {
    s.cleaning.stop();
}

#[tauri::command]
async fn clean_trash(id: String, pick: Option<trash::Pick>, app: AppHandle, s: State<'_, S>) -> Res<trash::CleanReport> {
    let s = s.inner().clone();
    let s2 = s.clone();
    let (since, picked) = (now_secs(), pick.clone());
    let report = tauri::async_runtime::spawn_blocking(move || {
        if trash::needs_admin(&id) && !mom_core::elevate::is_elevated() {
            return clean_as_admin(&s, &id, pick.as_ref());
        }
        let cfg = s.cfg();
        let table = s.sampler.lk().sample();
        let mut ctx = trash::TrashCtx::new(&s.paths, &cfg, &table);
        s.cleaning.reset();
        ctx.live = Some(&s.cleaning);
        let r = trash::clean(&id, &ctx, pick.as_ref());
        if r.freed > 0 {
            s.note(format!("Took out {} of trash ({id}).", human_bytes(r.freed)));
        }
        r
    })
    .await
    .map_err(err)?;
    history::cleaned(&s2.paths, &report, picked.as_ref(), false, since);
    measure_trash_now(app, s2, Some(report.id.clone()));
    Ok(report)
}

/// Cleans a spot only an administrator can: Windows asks the person, then `mom` (next to the
/// app) runs elevated and cleans exactly what it finds itself, under the usual rules and only
/// where no one else could have swapped a folder for a link. It writes nothing back: this
/// notes what would go beforehand and looks at what's gone afterwards.
fn clean_as_admin(s: &Shared, id: &str, pick: Option<&trash::Pick>) -> trash::CleanReport {
    let mut r = trash::CleanReport { id: id.to_string(), ..Default::default() };
    let Some(mom) = std::env::current_exe().ok().map(|e| e.with_file_name("mom.exe")).filter(|m| m.is_file()) else {
        r.message = "mom.exe isn't next to the app, so it can't clean this as an administrator.".into();
        return r;
    };
    let before = {
        let cfg = s.cfg();
        let table = s.sampler.lk().sample();
        let ctx = trash::TrashCtx::new(&s.paths, &cfg, &table);
        let before = trash::plan(id, &ctx, pick);
        // First, as you, what's in folders anyone can change: no administrator needed for those.
        trash::clean(id, &ctx, pick);
        before
    };
    let gone = |targets: &[std::path::PathBuf]| targets.iter().all(|t| std::fs::symlink_metadata(t).is_err());
    let mut note = None;
    if before.iter().all(|(t, _)| gone(t)) {
        return tally_gone(id, before, r, None);
    }
    // Then the rest as administrator, told what you said to keep: an administrator who isn't you
    // (Windows asked for a password) reads their own settings, not yours.
    let mut args = format!("trash clean {id} --yes --elevated");
    let mut keep = s.cfg().trash.keep;
    // And what the widget's own list showed as kept, should the two ever differ.
    keep.extend(pick.iter().flat_map(|p| p.keep.iter().cloned()).filter(|k| !keep.contains(k)).collect::<Vec<_>>());
    let with_keep = (!keep.is_empty()).then(|| trash::Pick { keep, ..pick.cloned().unwrap_or_default() });
    if let Some(p) = with_keep.as_ref().or(pick) {
        let hex = mom_core::elevate::to_hex(&serde_json::to_string(p).unwrap_or_default());
        if hex.len() > 30_000 {
            note = Some("That's too many ticked and unticked items to hand an administrator at once; tick all or untick all, then change a few.".to_string());
        } else {
            args.push_str(&format!(" --pick-hex {hex}"));
        }
    }
    if note.is_none() {
        // Only a mom.exe nothing running as you could have swapped, or your "yes" goes to it.
        let app = std::env::current_exe().unwrap_or_default();
        if let Err(why) = mom_core::elevate::safe_to_elevate(&mom, &app).and_then(|()| mom_core::elevate::run_as_admin(&mom, &args)) {
            note = Some(why);
        }
    }
    tally_gone(id, before, r, note)
}

// ------------------------------------------------------------ files: copies and big forgotten files

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilesState {
    report: Option<mom_core::files::FilesReport>,
    busy: bool,
}

#[tauri::command(async)]
fn files_state(s: State<'_, S>) -> FilesState {
    let mut report = mom_core::files::load_saved(&s.paths);
    if let Some(r) = report.as_mut() {
        mom_core::files::mark_kept(r, &s.cfg().trash.keep);
    }
    FilesState { report, busy: s.files_busy.load(Ordering::Relaxed) }
}

/// Looks through your folders on its own thread: "files-progress" while it works, "files" when
/// it's done (or stopped).
#[tauri::command(async)]
fn scan_files(app: AppHandle, s: State<'_, S>) {
    let s = s.inner().clone();
    if s.files_busy.swap(true, Ordering::SeqCst) {
        return;
    }
    s.files_stop.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let opts = mom_core::files::ScanOpts::for_config(&s.paths, &s.cfg().files);
        let progress = |p: mom_core::files::Progress| {
            let _ = app.emit("files-progress", p);
        };
        let mut r = mom_core::files::scan(&s.paths, &opts, &s.files_stop, &progress);
        mom_core::files::save(&s.paths, &r);
        mom_core::files::mark_kept(&mut r, &s.cfg().trash.keep);
        s.files_busy.store(false, Ordering::SeqCst);
        let _ = app.emit("files", FilesState { report: Some(r), busy: false });
    });
}

#[tauri::command(async)]
fn stop_files_scan(s: State<'_, S>) {
    s.files_stop.store(true, Ordering::SeqCst);
}

/// Sends what's ticked to the Recycle Bin, never the last copy of anything and nothing you said
/// to keep (see files::recycle).
#[tauri::command(async)]
fn recycle_files(paths: Vec<String>, s: State<'_, S>) -> mom_core::files::RecycleReport {
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let r = mom_core::files::recycle(&s.paths, &files, &s.cfg().trash.keep);
    history::copies(&s.paths, &r);
    if r.freed > 0 {
        s.note(format!("Moved {} of copies and old files to the Recycle Bin.", human_bytes(r.freed)));
    }
    r
}

// ------------------------------------------------------------ Windows settings

#[tauri::command(async)]
fn get_winsettings(s: State<'_, S>) -> Vec<mom_core::winsettings::Finding> {
    mom_core::winsettings::check(&s.paths)
}

/// Puts one setting back the way Windows ships it. Your own settings change right away (with a
/// backup for undo); the machine's go through an administrator `mom`, the same careful way as
/// admin clean-ups, and only from mom's own list with her own values.
#[tauri::command(async)]
fn fix_setting(id: String, s: State<'_, S>) -> Res<String> {
    use mom_core::winsettings;
    let Some(f) = winsettings::check(&s.paths).into_iter().find(|f| f.id == id) else {
        return Ok("That one is already fine.".into());
    };
    let Some(fix) = f.fix else {
        return Err("Mom can't change that one herself; the settings button opens the right place.".into());
    };
    let after = fix.after.map(|a| format!(" {a}")).unwrap_or_default();
    if !fix.admin {
        let msg = winsettings::fix(&s.paths, &id).map_err(err)?;
        let line = format!("Put back a Windows setting: {}", f.title.trim_end_matches('.'));
        if fix.undo { history::fixed(&s.paths, kind::SETTING, "setting", &id, line) } else { history::did(&s.paths, kind::SETTING, line) }
        s.note(format!("Put back a Windows setting: {}.", f.title));
        return Ok(msg);
    }
    if !winsettings::is_admin_fix(&id) || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':') {
        return Err("Mom doesn't change that one as administrator.".into());
    }
    let app = std::env::current_exe().unwrap_or_default();
    let mom = app.with_file_name("mom.exe");
    mom_core::elevate::safe_to_elevate(&mom, &app)?;
    winsettings::record_admin_fix(&s.paths, &id)?;
    mom_core::elevate::run_as_admin(&mom, &format!("settings fix {id} --elevated"))?;
    if winsettings::check(&s.paths).iter().any(|f| f.id == id) {
        return Err("Windows let mom try, but the setting didn't change.".into());
    }
    history::did(&s.paths, kind::SETTING, format!("Put back a Windows setting: {}", f.title.trim_end_matches('.')));
    s.note(format!("Put back a Windows setting: {}.", f.title));
    Ok(format!("Done: {}.{after}", f.title.trim_end_matches('.')))
}

#[tauri::command(async)]
fn undo_setting(id: String, s: State<'_, S>) -> Res<String> {
    let msg = mom_core::winsettings::undo(&s.paths, &id).map_err(err)?;
    history::put_back(&s.paths, kind::SETTING_UNDO, "setting", Some(&id), msg.clone());
    Ok(msg)
}

/// Opens one of Windows' own settings programs (the ones the checkup points to), Event Viewer, or
/// the graphics card maker's app.
#[tauri::command(async)]
fn open_program(name: String) -> Res<()> {
    if let Some(app) = mom_core::speed::gpu_app_path(&name) {
        return mom_core::elevate::open(&app, "");
    }
    let Some((program, hand)) = mom_core::winsettings::program(&name) else {
        return Err("mom only opens Windows' own settings programs".into());
    };
    let windows = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    mom_core::elevate::open(&windows.join("System32").join(program), hand)
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Is there a newer momputer? Asks GitHub at most once a day, and not at all if that's switched
/// off in Settings, unless `now` (the person pressed "Check now").
#[tauri::command]
async fn get_update(now: bool, s: State<'_, S>) -> Res<mom_core::updates::Update> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let current = env!("CARGO_PKG_VERSION");
        if now || s.cfg().ui.check_updates {
            mom_core::updates::check(&s.paths, current, now)
        } else {
            mom_core::updates::known(&s.paths, current)
        }
    })
    .await
    .map_err(err)
}

/// Why mom can't clean as administrator from this copy of the app, if she can't: shown before
/// anyone presses the button.
#[tauri::command(async)]
fn admin_check() -> Option<String> {
    let app = std::env::current_exe().unwrap_or_default();
    mom_core::elevate::safe_to_elevate(&app.with_file_name("mom.exe"), &app).err()
}

/// What a clean-up took, from what's gone of what it planned to take.
fn tally_gone(id: &str, before: Vec<(Vec<std::path::PathBuf>, u64)>, mut r: trash::CleanReport, note: Option<String>) -> trash::CleanReport {
    for (targets, bytes) in before {
        if targets.iter().all(|t| std::fs::symlink_metadata(t).is_err()) {
            r.deleted += 1;
            r.freed += bytes;
        } else {
            r.skipped += 1;
        }
    }
    r.message = match (r.deleted, note) {
        (0, Some(why)) => why,
        (_, Some(why)) => format!("{} {why}", trash::result_message(id, &r)),
        (_, None) => trash::result_message(id, &r),
    };
    r
}

/// Where old conversations go, how much room is left there, and what's been moved.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveInfo {
    dir: Option<String>,
    free: Option<u64>,
    reachable: bool,
    moved: Vec<mom_core::archive::MovedNow>,
}

fn archive_info_of(s: &Shared) -> ArchiveInfo {
    let dir = s.cfg().trash.archive_dir;
    ArchiveInfo {
        free: dir.as_deref().and_then(mom_core::archive::free_space),
        reachable: dir.as_deref().is_some_and(|d| d.is_dir()),
        dir: dir.map(|d| d.to_string_lossy().to_string()),
        moved: mom_core::archive::status(&s.paths),
    }
}

#[tauri::command(async)]
fn archive_info(s: State<'_, S>) -> ArchiveInfo {
    archive_info_of(&s)
}

/// Picks the folder old conversations go to. A whole drive gets a momputer-archive folder.
#[tauri::command]
async fn choose_archive_dir(app: AppHandle, s: State<'_, S>) -> Res<Option<ArchiveInfo>> {
    let Some(picked) = app.dialog().file().set_title("Where should mom keep old conversations?").blocking_pick_folder() else {
        return Ok(None);
    };
    let mut dir = picked.into_path().map_err(err)?;
    if dir.parent().is_none() {
        dir = dir.join("momputer-archive");
    }
    if mom_core::archive::same_drive(&dir, &s.paths.home) {
        return Err("That's the drive your conversations are on already, so moving them there frees nothing. Pick another one, like an external drive.".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    {
        let mut cfg = s.config.lk();
        cfg.trash.archive_dir = Some(dir);
        cfg.save(&s.paths).map_err(err)?;
    }
    measure_trash_now(app, s.inner().clone(), Some("old-conversations".into()));
    Ok(Some(archive_info_of(&s)))
}

/// Puts a moved conversation folder back where it was.
#[tauri::command]
async fn bring_back(from: String, app: AppHandle, s: State<'_, S>) -> Res<String> {
    let s = s.inner().clone();
    let s2 = s.clone();
    let m = tauri::async_runtime::spawn_blocking(move || mom_core::archive::bring_back(&s.paths, std::path::Path::new(&from)))
        .await
        .map_err(err)??;
    history::record(&s2.paths, history::Entry::new(kind::BRING_BACK, format!("Brought {} back from the archive", m.name)).bytes(m.bytes).items(1));
    s2.note(format!("Brought {} back from the archive.", m.name));
    measure_trash_now(app, s2, Some("old-conversations".into()));
    Ok(format!("{} is back where it was ({}).", m.name, human_bytes(m.bytes)))
}

/// Deletes a moved folder from the archive for good (and its link).
#[tauri::command]
async fn delete_moved(from: String, app: AppHandle, s: State<'_, S>) -> Res<String> {
    let s = s.inner().clone();
    let s2 = s.clone();
    let m = tauri::async_runtime::spawn_blocking(move || mom_core::archive::delete_moved(&s.paths, std::path::Path::new(&from)))
        .await
        .map_err(err)??;
    history::record(&s2.paths, history::Entry::new(kind::ARCHIVE_DELETE, format!("Deleted {} from the archive for good", m.name)).bytes(m.bytes).items(1));
    s2.note(format!("Deleted {} from the archive.", m.name));
    measure_trash_now(app, s2, None);
    Ok(format!("Deleted {} ({}) for good.", m.name, human_bytes(m.bytes)))
}

/// Opens a program's own uninstaller, named by its uninstall entry.
#[tauri::command(async)]
fn uninstall_app(key: String) -> Res<String> {
    let r = mom_core::installed::uninstall(&key);
    mom_core::installed::forget_unused();
    r
}

/// Disks, memory, uptime, startup apps and the checkups built from them.
#[tauri::command]
async fn get_health(s: State<'_, S>) -> Res<mom_core::health::Health> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let table = s.sampler.lk().sample();
        mom_core::health::check(&table, &s.paths)
    })
    .await
    .map_err(err)
}

/// What's slowing the PC down. `fresh` reads the event log and the installed tools again
/// instead of using what was read in the last five minutes.
#[tauri::command]
async fn get_speed(fresh: bool, s: State<'_, S>) -> Res<mom_core::speed::Speed> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if fresh {
            mom_core::speed::forget();
        }
        let table = s.sampler.lk().sample();
        mom_core::speed::check(&table, &s.paths)
    })
    .await
    .map_err(err)
}

/// The rest of the Windows checkup: hardware, security, updates, registry. Several seconds the
/// first time (or with `fresh`); kept for half an hour.
#[tauri::command]
async fn get_system(fresh: bool) -> Res<mom_core::system::System> {
    tauri::async_runtime::spawn_blocking(move || mom_core::system::get(fresh)).await.map_err(err)
}

/// Your browsers, their profiles and every extension. A few seconds the first time; kept five
/// minutes unless `fresh`.
#[tauri::command]
async fn get_browsers(fresh: bool, s: State<'_, S>) -> Res<Vec<mom_core::browsers::Browser>> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let table = s.sampler.lk().sample();
        mom_core::browsers::list(&table, fresh)
    })
    .await
    .map_err(err)
}

/// Removes your own registry leftovers of one kind, keeping a copy.
#[tauri::command(async)]
fn fix_registry(id: String, s: State<'_, S>) -> Res<String> {
    let msg = mom_core::system::fix_registry(&s.paths, &id)?;
    history::fixed(&s.paths, kind::REGISTRY, "registry", &id, msg.clone());
    Ok(msg)
}

/// Puts back what the last registry fix of that kind removed.
#[tauri::command(async)]
fn undo_registry(id: String, s: State<'_, S>) -> Res<String> {
    let msg = mom_core::system::undo_registry(&s.paths, &id)?;
    history::put_back(&s.paths, kind::REGISTRY_UNDO, "registry", Some(&id), msg.clone());
    Ok(msg)
}

/// Where the Files scan looks: your usual folders, each drive, and folders you added.
#[tauri::command(async)]
fn files_places(s: State<'_, S>) -> Vec<mom_core::files::Place> {
    mom_core::files::places(&s.paths, &s.cfg().files)
}

/// Switches a place on or off.
#[tauri::command(async)]
fn set_files_place(path: String, on: bool, s: State<'_, S>) -> Res<Vec<mom_core::files::Place>> {
    let mut cfg = s.config.lk();
    mom_core::files::set_place(&s.paths, &mut cfg.files, std::path::Path::new(&path), on)?;
    cfg.save(&s.paths).map_err(err)?;
    Ok(mom_core::files::places(&s.paths, &cfg.files))
}

/// Adds a folder you pick.
#[tauri::command]
async fn add_files_place(app: AppHandle, s: State<'_, S>) -> Res<Option<Vec<mom_core::files::Place>>> {
    let Some(picked) = app.dialog().file().set_title("Which folder should mom look through for copies?").blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(err)?;
    let mut cfg = s.config.lk();
    mom_core::files::set_place(&s.paths, &mut cfg.files, &path, true)?;
    cfg.save(&s.paths).map_err(err)?;
    Ok(Some(mom_core::files::places(&s.paths, &cfg.files)))
}

/// Every program running, as one line each.
#[tauri::command]
async fn get_programs(s: State<'_, S>) -> Res<Vec<mom_core::programs::Program>> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let table = s.sampler.lk().sample();
        mom_core::programs::list(&table)
    })
    .await
    .map_err(err)
}

/// Programs' own icons, by .exe path, as data: URIs (kept for as long as mom runs).
#[tauri::command]
async fn get_icons(exes: Vec<String>) -> Res<std::collections::HashMap<String, Option<String>>> {
    tauri::async_runtime::spawn_blocking(move || mom_core::icons::programs(&exes)).await.map_err(err)
}

/// Restarts, quits or ends a program from `get_programs` (only what it offers for it).
#[tauri::command]
async fn program_action(id: String, action: String, s: State<'_, S>) -> Res<mom_core::programs::ActReport> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let table = s.sampler.lk().sample();
        let r = mom_core::programs::act(&table, &id, &action);
        if r.ok {
            history::did(&s.paths, kind::PROGRAM, r.message.clone());
        }
        r
    })
    .await
    .map_err(err)
}

/// What starts with Windows.
#[tauri::command]
async fn get_startup(s: State<'_, S>) -> Res<Vec<mom_core::startup::Startup>> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let table = s.sampler.lk().sample();
        mom_core::startup::list(&s.paths, &table)
    })
    .await
    .map_err(err)
}

/// Switches one of them on or off, the way Task Manager does.
#[tauri::command(async)]
fn set_startup(id: String, on: bool, s: State<'_, S>) -> Res<String> {
    let msg = mom_core::startup::set(&s.paths, &id, on)?;
    history::did(&s.paths, kind::STARTUP, msg.clone());
    Ok(msg)
}

#[tauri::command(async)]
fn get_rooms(s: State<'_, S>) -> Vec<tidy::RoomReport> {
    let rooms = s.cfg().rooms;
    let cache = s.rooms.lk();
    rooms
        .iter()
        .filter_map(|r| cache.get(&r.to_string_lossy().to_string()).cloned())
        .collect()
}

#[tauri::command(async)]
fn room_paths(s: State<'_, S>) -> Vec<String> {
    s.cfg().rooms.iter().map(|r| r.to_string_lossy().to_string()).collect()
}

#[tauri::command]
async fn inspect_room(path: String, s: State<'_, S>) -> Res<tidy::RoomReport> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = s.cfg();
        if tidy::is_moms_room(&PathBuf::from(&path), &s.paths) {
            return Ok(tidy::moms_room(&s.paths));
        }
        let dir = tidy::check_room(&PathBuf::from(&path), &s.paths)?;
        let report = tidy::inspect(&dir, &cfg.tidy).map_err(err)?;
        s.rooms.lk().insert(path, report.clone());
        Ok(report)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn add_room(app: AppHandle, s: State<'_, S>) -> Res<Option<String>> {
    let Some(picked) = app.dialog().file().set_title("Which folder should mom keep an eye on?").blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(err)?;
    // Not a drive, Windows, programs or apps' own data: say why, and don't add it. Mom's own
    // folder may be added; she has something to say about it.
    let path = if tidy::is_moms_room(&path, &s.paths) { s.paths.mom.clone() } else { tidy::check_room(&path, &s.paths)? };
    let mut cfg = s.config.lk();
    if !cfg.rooms.contains(&path) {
        cfg.rooms.push(path.clone());
        cfg.save(&s.paths).map_err(err)?;
    }
    Ok(Some(path.to_string_lossy().to_string()))
}

#[tauri::command(async)]
fn remove_room(path: String, s: State<'_, S>) -> Res<()> {
    let mut cfg = s.config.lk();
    cfg.rooms.retain(|r| r.to_string_lossy() != path);
    cfg.save(&s.paths).map_err(err)?;
    s.rooms.lk().remove(&path);
    Ok(())
}

#[tauri::command]
async fn apply_tidy(path: String, ids: Vec<String>, s: State<'_, S>) -> Res<tidy::ApplyReport> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let report = s.rooms.lk().get(&path).cloned().ok_or("Look at the room first.")?;
        tidy::check_room(&PathBuf::from(&report.path), &s.paths)?;
        let r = tidy::apply(&report, &ids, &s.paths.journal_dir());
        history::tidied(&s.paths, &report.name, &r);
        s.note(format!("Tidied {}: {}", report.name, r.message));
        if let Ok(fresh) = tidy::inspect(&PathBuf::from(&path), &s.cfg().tidy) {
            s.rooms.lk().insert(path, fresh);
        }
        Ok::<_, String>(r)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn undo_tidy(s: State<'_, S>) -> Res<tidy::UndoReport> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = tidy::undo_last(&s.paths.journal_dir());
        if r.restored > 0 {
            history::put_back(&s.paths, kind::TIDY_UNDO, "tidy", None, r.message.clone());
        }
        s.note(r.message.clone());
        let rooms: Vec<String> = s.rooms.lk().keys().cloned().collect();
        let cfg = s.cfg();
        for room in rooms {
            if let Ok(fresh) = tidy::inspect(&PathBuf::from(&room), &cfg.tidy) {
                s.rooms.lk().insert(room, fresh);
            }
        }
        r
    })
    .await
    .map_err(err)
}

#[tauri::command(async)]
fn chore_prompt(path: String, s: State<'_, S>) -> Res<String> {
    let report = s.rooms.lk().get(&path).cloned().ok_or("Look at the room first.")?;
    Ok(tidy::chore_prompt(&report))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RulesState {
    text: String,
    customized: bool,
    targets: Vec<rules::Target>,
    integrations: Vec<hooks::Integration>,
    /// The skills that come with momputer, and the line that installs them in Claude Code.
    skills: Vec<mom_core::skills::Skill>,
    skills_install: &'static str,
    skills_page: &'static str,
    /// For the copies that are behind: what to type to catch up, and what to hand an agent for
    /// the ones copied by hand.
    skills_update: Vec<String>,
    skills_catch_up: Option<String>,
}

#[tauri::command(async)]
fn get_rules(s: State<'_, S>) -> RulesState {
    let skills = mom_core::skills::list(&s.paths);
    let late: Vec<_> = skills.iter().filter(|k| k.behind).cloned().collect();
    RulesState {
        text: rules::current(&s.paths),
        customized: rules::is_customized(&s.paths),
        targets: rules::targets(&s.paths),
        integrations: hooks::integrations(&s.paths),
        skills_update: mom_core::skills::update_lines(&late),
        skills_catch_up: mom_core::skills::catch_up(&late),
        skills,
        skills_install: mom_core::skills::INSTALL,
        skills_page: mom_core::skills::PAGE,
    }
}

/// How each agent on this PC is set up, and what's worth a look: MCP servers, hooks, settings,
/// skills. Only looks.
#[tauri::command(async)]
fn get_agents(s: State<'_, S>) -> Vec<mom_core::setup::AgentSetup> {
    let agents = mom_core::setup::check(&s.paths);
    *s.agents.lk() = agents.clone();
    agents
}

/// Asks Claude Code to try each of its MCP servers (its own check, the one its /mcp screen
/// runs), then looks at the set-ups again with what it said. Only when the person presses the
/// button: Claude Code starts each server for a moment to do it.
#[tauri::command]
async fn check_agents(s: State<'_, S>) -> Res<Vec<mom_core::setup::AgentSetup>> {
    let s = s.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let asked = mom_core::setup::ask_claude(&s.paths);
        let agents = mom_core::setup::check(&s.paths);
        *s.agents.lk() = agents.clone();
        asked.map(|_| agents)
    })
    .await
    .map_err(err)?
}

/// One of the files `get_agents` listed, by its id, with keys blanked. Never a path from the
/// widget: only what mom herself found in an agent's set-up can be read.
#[tauri::command(async)]
fn read_doc(id: String, s: State<'_, S>) -> Res<mom_core::setup::DocText> {
    let mut agents = s.agents.lk().clone();
    if agents.is_empty() {
        agents = mom_core::setup::check(&s.paths);
        *s.agents.lk() = agents.clone();
    }
    mom_core::setup::read(&agents, &id)
}

/// Says so, once, when an agent's copy of a skill is older than the one this momputer knows, and
/// again only when that changes. Looks a minute after starting and then every few hours: it's a
/// handful of small files.
fn skills_watch(app: AppHandle, s: S) {
    std::thread::sleep(Duration::from_secs(60));
    loop {
        let late = mom_core::skills::behind(&s.paths);
        let told = s.paths.state_dir().join("skills-told.txt");
        let now: String = late.iter().map(|k| format!("{}@{}", k.name, k.version)).collect::<Vec<_>>().join(",");
        if now != std::fs::read_to_string(&told).unwrap_or_default() {
            let _ = mom_core::util::write_atomic(&told, &now);
            let _ = app.emit("skills", late.len());
            let cfg = s.cfg();
            if !late.is_empty() && cfg.ui.notifications && cfg.ui.notify_skills {
                let text = match late.as_slice() {
                    [one] => format!("{} has a newer version than the copy your agent has. House rules, Skills, has the line to catch up.", one.title),
                    many => format!("{} skills have newer versions than the copies your agents have. House rules, Skills, has the line to catch up.", many.len()),
                };
                s.note(text.clone());
                toast(&app, "Skills", &text);
            }
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    }
}

#[tauri::command(async)]
fn save_rules(text: String, s: State<'_, S>) -> Res<String> {
    rules::save(&s.paths, &text).map_err(err)?;
    // Agents that already have the rules get the new version too.
    for t in rules::targets(&s.paths).into_iter().filter(|t| t.installed) {
        rules::install(&s.paths, t.agent).map_err(err)?;
    }
    Ok("Saved. Mom will hold everyone to it.".into())
}

#[tauri::command(async)]
fn reset_rules(s: State<'_, S>) -> Res<String> {
    rules::reset(&s.paths).map_err(err)?;
    for t in rules::targets(&s.paths).into_iter().filter(|t| t.installed) {
        rules::install(&s.paths, t.agent).map_err(err)?;
    }
    Ok("Back to mom's original rules.".into())
}

fn parse_agent(a: &str) -> Res<mom_core::agents::AgentKind> {
    use mom_core::agents::AgentKind;
    match a {
        "claude-code" | "claude" => Ok(AgentKind::ClaudeCode),
        "codex" => Ok(AgentKind::Codex),
        "gemini" => Ok(AgentKind::Gemini),
        _ => Err(format!("unknown agent {a}")),
    }
}

#[tauri::command(async)]
fn teach_rules(agent: String, on: bool, s: State<'_, S>) -> Res<String> {
    let a = parse_agent(&agent)?;
    let msg = if on { rules::install(&s.paths, a) } else { rules::remove(&s.paths, a) };
    msg.map_err(err)
}

// 0.5.0 A1: the Homework check, and agents waiting for you.

/// What each agent session did this week, from its own log. Kept a minute unless `fresh`.
#[tauri::command(async)]
fn get_homework(fresh: Option<bool>, s: State<'_, S>) -> Vec<mom_core::homework::Receipt> {
    mom_core::homework::recent(&s.paths, fresh.unwrap_or(false))
}

/// Agents waiting for you right now (when that switch is on).
#[tauri::command(async)]
fn get_waiting(s: State<'_, S>) -> Vec<mom_core::waiting::Waiting> {
    mom_core::waiting::current(&s.paths)
}

/// Says when an agent has been waiting for you half a minute. Looks at most every ten seconds,
/// and only at mom's own small folder of records, which is empty unless the switch is on.
fn waiting_tick(app: &AppHandle, s: &Shared, cfg: &Config) {
    use std::sync::atomic::AtomicU64;
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = now_secs();
    if now.saturating_sub(LAST.load(Ordering::Relaxed)) < 10 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    let due = mom_core::waiting::due(&s.paths);
    if due.is_empty() {
        return;
    }
    for w in &due {
        let (title, body) = mom_core::waiting::sentence(w);
        s.note(format!("{title} · {body}"));
        if cfg.ui.notifications {
            toast(app, &title, &body);
        }
    }
    let _ = app.emit("waiting", due.len());
}

fn bundled_cli() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.with_file_name(if cfg!(windows) { "mom.exe" } else { "mom" })
}

#[tauri::command(async)]
fn set_integration(id: String, on: bool, s: State<'_, S>) -> Res<String> {
    let msg = if on { hooks::install(&s.paths, &id, &bundled_cli()) } else { hooks::uninstall(&s.paths, &id) };
    let msg = msg.map_err(err)?;
    s.note(msg.clone());
    Ok(msg)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    config: Config,
    autostart: bool,
    version: String,
    config_path: String,
}

#[tauri::command(async)]
fn get_settings(app: AppHandle, s: State<'_, S>) -> Settings {
    Settings {
        config: s.cfg(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: env!("CARGO_PKG_VERSION").to_string(),
        config_path: s.paths.config_file().to_string_lossy().to_string(),
    }
}

#[tauri::command(async)]
fn set_settings(mut config: Config, app: AppHandle, s: State<'_, S>) -> Res<()> {
    // What the settings sheet doesn't edit stays as it is: those lists have buttons of their own,
    // and the widget's copy of the settings can be older than the last thing they changed.
    {
        let now = s.config.lk();
        config.kept = now.kept.clone();
        config.rooms = now.rooms.clone();
        config.files = now.files.clone();
        config.trash.archive_dir = now.trash.archive_dir.clone();
        config.trash.keep = now.trash.keep.clone();
    }
    config.save(&s.paths).map_err(err)?;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(config.ui.always_on_top);
    }
    *s.config.lk() = config;
    s.rescan_now();
    Ok(())
}

#[tauri::command(async)]
fn set_autostart(on: bool, app: AppHandle) -> Res<bool> {
    let al = app.autolaunch();
    if on { al.enable() } else { al.disable() }.map_err(err)?;
    Ok(al.is_enabled().unwrap_or(on))
}

#[tauri::command(async)]
fn get_diary(s: State<'_, S>) -> Vec<Note> {
    s.diary.lk().iter().cloned().collect()
}

// ------------------------------------------------------------------ what mom did (0.5.0 C2)

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryState {
    entries: Vec<history::Shown>,
    letter: Option<mom_core::letter::Letter>,
    letter_unread: bool,
    autopilot: Option<trash::autopilot::Run>,
}

/// What mom did, newest first, her last letter, and Autopilot's last run.
#[tauri::command(async)]
fn get_history(s: State<'_, S>) -> HistoryState {
    HistoryState {
        entries: history::shown(&s.paths, 300),
        letter: mom_core::letter::latest(&s.paths),
        letter_unread: mom_core::letter::unread(&s.paths),
        autopilot: trash::autopilot::last(&s.paths),
    }
}

/// Puts back one thing from "What I did", named by its id in mom's own record (never a path).
#[tauri::command]
async fn undo_history(id: String, app: AppHandle, s: State<'_, S>) -> Res<String> {
    let s = s.inner().clone();
    let s2 = s.clone();
    let msg = tauri::async_runtime::spawn_blocking(move || {
        let msg = history::undo(&s.paths, &id)?;
        // What came back may show elsewhere: rooms, conversations in the archive, servers.
        let rooms: Vec<String> = s.rooms.lk().keys().cloned().collect();
        let cfg = s.cfg();
        for room in rooms {
            if let Ok(fresh) = tidy::inspect(&PathBuf::from(&room), &cfg.tidy) {
                s.rooms.lk().insert(room, fresh);
            }
        }
        Ok::<_, String>(msg)
    })
    .await
    .map_err(err)??;
    s2.note(msg.clone());
    s2.rescan_now();
    measure_trash_now(app, s2, None);
    Ok(msg)
}

/// You opened the letter.
#[tauri::command(async)]
fn letter_read(s: State<'_, S>) {
    mom_core::letter::mark_read(&s.paths);
}

/// Once an hour (and once right after starting), on its own thread: Autopilot's weekly clean-up
/// when it's due, and Sunday's letter. The watcher never waits for either.
fn weekly(app: &AppHandle, s: &S) {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = now_secs();
    let last = LAST.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < 3600 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    let (app, s) = (app.clone(), s.clone());
    std::thread::spawn(move || {
        let cfg = s.cfg();
        if cfg.trash.autopilot && !mom_core::elevate::is_elevated() {
            let table = s.sampler.lk().sample();
            let ctx = trash::TrashCtx::new(&s.paths, &cfg, &table);
            let run = trash::autopilot::run_if_due(&ctx, trash::autopilot::away_secs(), trash::autopilot::on_mains());
            if let Some(text) = run.as_ref().and_then(trash::autopilot::said) {
                s.note(text.clone());
                if cfg.ui.notifications && cfg.ui.notify_cleanups {
                    toast(&app, "mom", &text);
                }
                measure_trash_now(app.clone(), s.clone(), None);
            }
        }
        if mom_core::letter::due(&s.paths, now) {
            mom_core::letter::write(&s.paths, now, &mom_core::system::get(false).checks);
            if cfg.ui.notifications && cfg.ui.notify_letter {
                toast(&app, "mom", "Your weekly letter from mom is here");
            }
            let _ = app.emit("letter", ());
        }
    });
}

#[tauri::command(async)]
fn open_url(url: String, app: AppHandle) -> Res<()> {
    let local = url.starts_with("http://localhost:") || url.starts_with("http://127.0.0.1:");
    let codex_thread = url
        .strip_prefix("codex://threads/")
        .map(|id| !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        .unwrap_or(false);
    let settings_page = url
        .strip_prefix("ms-settings:")
        .map(|page| !page.is_empty() && page.len() <= 48 && page.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(false);
    // Where to get a graphics driver, from Speed; and Windows Security.
    let driver_page = ["https://www.nvidia.com/en-us/drivers/", "https://www.amd.com/en/support/download/drivers.html", "https://www.intel.com/content/www/us/en/support/detect.html"]
        .contains(&url.as_str());
    let security = url == "windowsdefender:" || url == "windowsdefender://network/";
    // The author's other projects, from the About sheet.
    let author = ["https://getmarsell.com", "https://ilisium.xyz"].iter().any(|site| url == *site || url.starts_with(&format!("{site}/")));
    // Where a newer momputer is fetched from.
    let install_page = url == mom_core::updates::GET_IT;
    if !(local || codex_thread || settings_page || author || driver_page || security || install_page || url.starts_with("https://github.com/")) {
        return Err("mom only opens local servers, Codex threads, Windows Settings pages and the project's own pages".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

#[tauri::command(async)]
fn reveal(path: String, app: AppHandle) -> Res<()> {
    // Recycle Bin storage has generated $R names. Its shell view supplies the original names,
    // locations and Restore action; old reports and every reveal caller must use that view.
    if is_recycle_bin_path(&path) {
        return open_recycle_bin(app);
    }
    app.opener().reveal_item_in_dir(path).map_err(err)
}

fn is_recycle_bin_path(path: &str) -> bool {
    path.split(['\\', '/']).any(|part| part.eq_ignore_ascii_case("$Recycle.Bin"))
}

#[tauri::command(async)]
fn open_recycle_bin(app: AppHandle) -> Res<()> {
    #[cfg(windows)]
    {
        let windows = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let explorer = windows.join("explorer.exe").to_string_lossy().into_owned();
        app.opener().open_path("shell:RecycleBinFolder", Some(explorer)).map_err(err)
    }
    #[cfg(not(windows))]
    { let _ = app; Err("The Windows Recycle Bin is only available on Windows.".into()) }
}

#[cfg(test)]
mod reveal_tests {
    #[test]
    fn recycled_paths_use_the_shell_view_without_matching_similar_folder_names() {
        for path in [r"C:\$Recycle.Bin", r"D:\$RECYCLE.BIN\S-1-5-21\$R123.pdf", "C:/$Recycle.Bin/S-1-5-21/$R123.pdf", r"\\?\C:\$Recycle.Bin\S-1-5-21\$R123.pdf"] {
            assert!(super::is_recycle_bin_path(path), "{path}");
        }
        for path in [r"C:\Documents\report.pdf", r"C:\Downloads\$Recycle.Bin notes.txt", r"C:\$Recycle.Bin-old\report.pdf"] {
            assert!(!super::is_recycle_bin_path(path), "{path}");
        }
    }
}

/// BIG MODE: the widget becomes a normal-sized window in the middle of the screen. And back:
/// the small widget by the tray again. Both sizes keep the main window in the taskbar.
/// `smooth`: a stand-in card carries the change (see `make_card`) while the real window moves
/// unseen. Then this returns true, and the widget calls `big_arrived` once its page is laid out
/// for the new size. Off for anyone who asked Windows for less motion. `bg`: the page's colour.
#[tauri::command(async)]
fn set_big(on: bool, smooth: Option<bool>, bg: Option<String>, app: AppHandle, s: State<'_, S>) -> bool {
    let Some(win) = app.get_webview_window("main") else { return false };
    let monitor = win.primary_monitor().ok().flatten();
    // Only on the main screen, where both the widget's spot and the middle are.
    let here = win.current_monitor().ok().flatten().map(|m| m.position().x) == monitor.as_ref().map(|m| m.position().x);
    let smooth = smooth.unwrap_or(false) && here && win.is_visible().unwrap_or(false);
    let bg = bg.unwrap_or_default();
    let mut carried = false;
    if on {
        // As big as fits, up to a comfortable size.
        let (mut w, mut h) = (1280.0_f64, 820.0_f64);
        if let Some(monitor) = &monitor {
            let area = monitor.work_area().size.to_logical::<f64>(monitor.scale_factor());
            w = w.min(area.width - 60.0).max(960.0);
            h = h.min(area.height - 60.0).max(620.0);
        }
        let _ = win.set_always_on_top(false);
        if let Some(monitor) = monitor.as_ref().filter(|_| smooth) {
            // To the middle of the screen.
            let area = monitor.work_area();
            carried = morph(&app, s.inner(), &win, monitor.scale_factor(), &bg, tauri::LogicalSize::new(w, h).to_physical::<u32>(monitor.scale_factor()), |whole| {
                tauri::PhysicalPosition::new(area.position.x + (area.size.width as i32 - whole.width as i32) / 2, area.position.y + (area.size.height as i32 - whole.height as i32) / 2)
            });
        }
        if !carried {
            let _ = win.set_size(tauri::LogicalSize::new(w, h));
            let _ = win.center();
        }
        let _ = win.set_min_size(Some(tauri::LogicalSize::new(960.0, 620.0)));
    } else {
        let _ = win.set_min_size(Some(tauri::LogicalSize::new(360.0, 440.0)));
        if let Some(monitor) = monitor.as_ref().filter(|_| smooth) {
            // Back to her spot by the tray.
            carried = morph(&app, s.inner(), &win, monitor.scale_factor(), &bg, tauri::LogicalSize::new(420.0, 680.0).to_physical::<u32>(monitor.scale_factor()), |whole| widget_spot(monitor, whole));
        }
        if !carried {
            let _ = win.set_size(tauri::LogicalSize::new(420.0, 680.0));
            place_window(&win);
        }
        let _ = win.set_always_on_top(s.cfg().ui.always_on_top);
    }
    carried
}

/// The widget's page is laid out for its new size: wait for the new shape to finish opening, then
/// put the real window back under it.
#[tauri::command(async)]
fn big_arrived(app: AppHandle, s: State<'_, S>) {
    arrive(&app, s.inner());
}

/// The widget's page has drawn itself at its new size, under the stand-in: dissolve the stand-in
/// into it and put it away.
#[tauri::command(async)]
fn card_away(app: AppHandle, s: State<'_, S>) {
    card_gone(&app, s.inner());
}

/// The stand-in card's page says the card is drawn and on the glass.
#[tauri::command(async)]
fn glide_ready(s: State<'_, S>) {
    let (lock, cv) = &s.card_ready;
    *lock.lk() = true;
    cv.notify_all();
}

#[tauri::command(async)]
fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command(async)]
fn set_paused(paused: bool, s: State<'_, S>) {
    s.paused.store(paused, Ordering::Relaxed);
    if !paused {
        s.rescan_now();
    }
}

fn main() {
    let paths = Paths::detect();
    let config = Config::load(&paths);
    let start_hidden = std::env::args().any(|a| a == "--hidden");
    // What was measured last time shows straight away; measuring again is up to the user.
    let saved_trash = trash::load_saved(&paths);
    let shared: S = Arc::new(Shared {
        paths,
        config: Mutex::new(config),
        sampler: Mutex::new(Sampler::new()),
        snapshot: Mutex::new(None),
        trash: Mutex::new(saved_trash),
        trash_busy: AtomicBool::new(false),
        trash_next: Mutex::new(HashSet::new()),
        trash_scale: Mutex::new(HashMap::new()),
        cleaning: trash::Live::default(),
        files_busy: AtomicBool::new(false),
        files_stop: AtomicBool::new(false),
        machines: Mutex::new(Machines::default()),
        machines_busy: AtomicBool::new(false),
        machines_at: Mutex::new(None),
        rooms: Mutex::new(BTreeMap::new()),
        agents: Mutex::new(Vec::new()),
        paused: AtomicBool::new(false),
        wake: (Mutex::new(false), Condvar::new()),
        leftover_since: Mutex::new(HashMap::new()),
        notified: Mutex::new(None),
        last_toast: Mutex::new(0),
        last_alarm: Mutex::new(0),
        melting_since: Mutex::new(None),
        tray_state: Mutex::new((false, String::new())),
        diary: Mutex::new(VecDeque::new()),
        placed: AtomicBool::new(false),
        card_ready: (Mutex::new(false), Condvar::new()),
        morph_until: Mutex::new(None),
        card_up: AtomicBool::new(false),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh,
            stop_groups,
            stop_leftovers,
            keep_group,
            keep_item,
            keep_nothing,
            get_stopped,
            start_again,
            forget_stopped,
            stop_container,
            stop_wsl,
            get_trash,
            measure_trash,
            clean_trash,
            archive_info,
            choose_archive_dir,
            bring_back,
            delete_moved,
            uninstall_app,
            admin_check,
            measure_spot,
            app_version,
            get_update,
            clean_progress,
            stop_clean,
            files_state,
            scan_files,
            stop_files_scan,
            recycle_files,
            get_winsettings,
            fix_setting,
            undo_setting,
            open_program,
            get_health,
            get_speed,
            get_programs,
            get_icons,
            files_places,
            get_browsers,
            fix_registry,
            undo_registry,
            set_files_place,
            add_files_place,
            get_system,
            program_action,
            get_startup,
            set_startup,
            get_rooms,
            room_paths,
            inspect_room,
            add_room,
            remove_room,
            apply_tidy,
            undo_tidy,
            chore_prompt,
            get_rules,
            get_agents,
            check_agents,
            read_doc,
            save_rules,
            reset_rules,
            teach_rules,
            set_integration,
            get_settings,
            set_settings,
            set_autostart,
            get_diary,
            open_url,
            reveal,
            open_recycle_bin,
            hide_window,
            set_big,
            big_arrived,
            card_away,
            glide_ready,
            set_paused,
            // 0.5.0 C2
            get_history,
            undo_history,
            letter_read,
            // 0.5.0 A1
            get_homework,
            get_waiting,
        ])
        .setup(move |app| {
            #[cfg(windows)]
            toasts::register(&shared.paths.state_dir());
            let handle = app.handle().clone();
            let show = MenuItem::with_id(app, "show", "Open momputer", true, None::<&str>)?;
            let clean = MenuItem::with_id(app, "clean", "Clean up leftovers", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit mom", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &clean, &PredefinedMenuItem::separator(app)?, &quit])?;
            TrayIconBuilder::with_id("mom")
                .icon(tauri::image::Image::from_bytes(TRAY_CALM)?)
                .tooltip("momputer")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "clean" => {
                        let s = app.state::<S>().inner().clone();
                        let app = app.clone();
                        std::thread::spawn(move || {
                            let r = stop_where(&s, |g| g.is_leftover());
                            toast(&app, "momputer", &r.message);
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        toggle_main(tray.app_handle());
                    }
                })
                .build(app)?;

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_always_on_top(shared.cfg().ui.always_on_top);
            }
            // The stand-in card for BIG MODE, once mom has settled in.
            let card_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                let app = card_handle.clone();
                let _ = card_handle.run_on_main_thread(move || make_card(&app));
            });
            let (s, skills_handle) = (shared.clone(), handle.clone());
            std::thread::Builder::new().name("mom-skills".into()).spawn(move || skills_watch(skills_handle, s))?;
            let s = shared.clone();
            std::thread::Builder::new()
                .name("mom-watcher".into())
                .spawn(move || scanner(handle, s))?;
            if !start_hidden {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing the widget just tucks it into the tray. Mom never sleeps.
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("momputer failed to start");
}
