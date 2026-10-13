// momputer: a mom for you, your computer and your AIs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mom_core::containers::{self, Machines};
use mom_core::kill::{self, KillReport};
use mom_core::procs::Sampler;
use mom_core::scan::{Group, Status};
use mom_core::util::{human_bytes, now_secs};
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

fn place_window(win: &tauri::WebviewWindow) {
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let area = monitor.work_area();
        if let Ok(size) = win.outer_size() {
            let margin = (16.0 * monitor.scale_factor()) as i32;
            let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
            let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
            let _ = win.set_position(tauri::PhysicalPosition::new(x.max(area.position.x), y.max(area.position.y)));
        }
    }
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
    mom_core::restart::remember(&s.paths, &table, &chosen);
    let report = kill::stop_groups(&table, &chosen);
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
    if s.trash_busy.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let cfg = s.cfg();
        let table = s.sampler.lk().sample();
        let ctx = trash::TrashCtx::new(&s.paths, &cfg, &table);
        let ids: Vec<String> = match only {
            Some(id) => vec![id],
            None => trash::spot_ids().into_iter().map(String::from).collect(),
        };
        for id in ids {
            if let Some(spot) = trash::measure(&id, &ctx) {
                let mut list = s.trash.lk();
                match list.iter_mut().find(|x| x.id == spot.id) {
                    Some(x) => *x = spot,
                    None => list.push(spot),
                }
            }
            emit_trash(&app, &s);
        }
        trash::save(&s.paths, &s.trash.lk());
        s.trash_busy.store(false, Ordering::SeqCst);
        emit_trash(&app, &s);
    });
}

#[tauri::command(async)]
fn measure_trash(app: AppHandle, s: State<'_, S>) {
    measure_trash_now(app, s.inner().clone(), None);
}

/// Measures one spot again (the refresh button on its sheet).
#[tauri::command(async)]
fn measure_spot(id: String, app: AppHandle, s: State<'_, S>) {
    measure_trash_now(app, s.inner().clone(), Some(id));
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
    // Then the rest as administrator.
    let mut args = format!("trash clean {id} --yes --elevated");
    if let Some(p) = pick {
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
    FilesState { report: mom_core::files::load_saved(&s.paths), busy: s.files_busy.load(Ordering::Relaxed) }
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
        let r = mom_core::files::scan(&s.paths, &opts, &s.files_stop, &progress);
        mom_core::files::save(&s.paths, &r);
        s.files_busy.store(false, Ordering::SeqCst);
        let _ = app.emit("files", FilesState { report: Some(r), busy: false });
    });
}

#[tauri::command(async)]
fn stop_files_scan(s: State<'_, S>) {
    s.files_stop.store(true, Ordering::SeqCst);
}

/// Sends what's ticked to the Recycle Bin, never the last copy of anything (see files::recycle).
#[tauri::command(async)]
fn recycle_files(paths: Vec<String>, s: State<'_, S>) -> mom_core::files::RecycleReport {
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let r = mom_core::files::recycle(&s.paths, &files);
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
    s.note(format!("Put back a Windows setting: {}.", f.title));
    Ok(format!("Done: {}.{after}", f.title.trim_end_matches('.')))
}

#[tauri::command(async)]
fn undo_setting(id: String, s: State<'_, S>) -> Res<String> {
    mom_core::winsettings::undo(&s.paths, &id).map_err(err)
}

/// Opens one of Windows' own settings programs (the ones the checkup points to), Event Viewer, or
/// the graphics card maker's app.
#[tauri::command(async)]
fn open_program(name: String) -> Res<()> {
    const ALLOWED: &[&str] = &["SystemPropertiesPerformance.exe", "SystemPropertiesAdvanced.exe", "UserAccountControlSettings.exe", "cttune.exe", "eventvwr.exe", "dfrgui.exe", "devmgmt.msc"];
    if let Some(app) = mom_core::speed::gpu_app_path(&name) {
        return mom_core::elevate::open(&app, "");
    }
    if !ALLOWED.contains(&name.as_str()) {
        return Err("mom only opens Windows' own settings programs".into());
    }
    let windows = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    mom_core::elevate::open(&windows.join("System32").join(&name), "")
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
    mom_core::system::fix_registry(&s.paths, &id)
}

/// Puts back what the last registry fix of that kind removed.
#[tauri::command(async)]
fn undo_registry(id: String, s: State<'_, S>) -> Res<String> {
    mom_core::system::undo_registry(&s.paths, &id)
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
        mom_core::programs::act(&table, &id, &action)
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
    mom_core::startup::set(&s.paths, &id, on)
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
}

#[tauri::command(async)]
fn get_rules(s: State<'_, S>) -> RulesState {
    RulesState {
        text: rules::current(&s.paths),
        customized: rules::is_customized(&s.paths),
        targets: rules::targets(&s.paths),
        integrations: hooks::integrations(&s.paths),
        skills: mom_core::skills::list(&s.paths),
        skills_install: mom_core::skills::INSTALL,
        skills_page: mom_core::skills::PAGE,
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
fn set_settings(config: Config, app: AppHandle, s: State<'_, S>) -> Res<()> {
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
    app.opener().reveal_item_in_dir(path).map_err(err)
}

/// BIG MODE: the widget becomes a normal-sized window, in the taskbar and in the middle of the
/// screen. And back: the small widget by the tray again.
#[tauri::command(async)]
fn set_big(on: bool, app: AppHandle, s: State<'_, S>) {
    let Some(win) = app.get_webview_window("main") else { return };
    if on {
        // As big as fits, up to a comfortable size.
        let (mut w, mut h) = (1280.0_f64, 820.0_f64);
        if let Ok(Some(monitor)) = win.primary_monitor() {
            let area = monitor.work_area().size.to_logical::<f64>(monitor.scale_factor());
            w = w.min(area.width - 60.0).max(960.0);
            h = h.min(area.height - 60.0).max(620.0);
        }
        let _ = win.set_always_on_top(false);
        let _ = win.set_skip_taskbar(false);
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
        let _ = win.set_min_size(Some(tauri::LogicalSize::new(960.0, 620.0)));
        let _ = win.center();
    } else {
        let _ = win.set_min_size(Some(tauri::LogicalSize::new(360.0, 440.0)));
        let _ = win.set_size(tauri::LogicalSize::new(420.0, 680.0));
        let _ = win.set_skip_taskbar(true);
        let _ = win.set_always_on_top(s.cfg().ui.always_on_top);
        place_window(&win);
    }
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
        cleaning: trash::Live::default(),
        files_busy: AtomicBool::new(false),
        files_stop: AtomicBool::new(false),
        machines: Mutex::new(Machines::default()),
        machines_busy: AtomicBool::new(false),
        machines_at: Mutex::new(None),
        rooms: Mutex::new(BTreeMap::new()),
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
            hide_window,
            set_big,
            set_paused,
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
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing the widget just tucks it into the tray. Mom never sleeps.
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("momputer failed to start");
}
