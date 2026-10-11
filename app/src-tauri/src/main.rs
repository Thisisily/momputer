// momputer: a mom for your AI.
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
            "{who} left {} thing{} running: {}{more} ({}). Open momputer to turn {} off.",
            ripe.len(),
            if ripe.len() == 1 { "" } else { "s" },
            what.join(", "),
            human_bytes(mem),
            if ripe.len() == 1 { "it" } else { "them" }
        );
        toast(app, "Somebody left the lights on", &body);
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
            "Leftovers have been burning {:.0}% of your CPU for a while ({} things). Open momputer and press Clean up.",
            snap.summary.leftover_cpu, snap.summary.leftovers
        );
        toast(app, "Your computer is melting", &body);
        s.note(format!("Meltdown alarm: leftovers at {:.0}% CPU.", snap.summary.leftover_cpu));
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
}

#[tauri::command(async)]
fn get_trash(s: State<'_, S>) -> TrashState {
    TrashState { spots: s.trash.lk().clone(), busy: s.trash_busy.load(Ordering::Relaxed) }
}

fn emit_trash(app: &AppHandle, s: &Shared) {
    let _ = app.emit(
        "trash",
        TrashState { spots: s.trash.lk().clone(), busy: s.trash_busy.load(Ordering::Relaxed) },
    );
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
        s.trash_busy.store(false, Ordering::SeqCst);
        emit_trash(&app, &s);
    });
}

#[tauri::command(async)]
fn measure_trash(app: AppHandle, s: State<'_, S>) {
    measure_trash_now(app, s.inner().clone(), None);
}

#[tauri::command]
async fn clean_trash(id: String, pick: Option<trash::Pick>, app: AppHandle, s: State<'_, S>) -> Res<trash::CleanReport> {
    let s = s.inner().clone();
    let s2 = s.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let cfg = s.cfg();
        let table = s.sampler.lk().sample();
        let ctx = trash::TrashCtx::new(&s.paths, &cfg, &table);
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
        let report = tidy::inspect(&PathBuf::from(&path), &cfg.tidy).map_err(err)?;
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
}

#[tauri::command(async)]
fn get_rules(s: State<'_, S>) -> RulesState {
    RulesState {
        text: rules::current(&s.paths),
        customized: rules::is_customized(&s.paths),
        targets: rules::targets(&s.paths),
        integrations: hooks::integrations(&s.paths),
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
    if !(local || codex_thread || settings_page || url.starts_with("https://github.com/")) {
        return Err("mom only opens local servers, Codex threads, Windows Settings pages and the project page".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

#[tauri::command(async)]
fn reveal(path: String, app: AppHandle) -> Res<()> {
    app.opener().reveal_item_in_dir(path).map_err(err)
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
    let shared: S = Arc::new(Shared {
        paths,
        config: Mutex::new(config),
        sampler: Mutex::new(Sampler::new()),
        snapshot: Mutex::new(None),
        trash: Mutex::new(Vec::new()),
        trash_busy: AtomicBool::new(false),
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
            stop_container,
            stop_wsl,
            get_trash,
            measure_trash,
            clean_trash,
            get_health,
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
