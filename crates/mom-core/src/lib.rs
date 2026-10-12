//! mom-core: the brains of momputer.
//!
//! Finds every runtime on the machine (node, bun, python, headless browsers, databases,
//! containers, VMs), works out which agent started each one, and spots what was left behind.
//! Also takes out the trash (temp folders and caches), gives the whole machine a checkup,
//! grades messy folders, and teaches agents the house rules.

pub mod agents;
pub mod archive;
pub mod browsers;
pub mod classify;
pub mod codex;
pub mod config;
pub mod elevate;
pub mod containers;
pub mod files;
pub mod health;
pub mod hooks;
pub mod icons;
pub mod installed;
pub mod kill;
pub mod paths;
pub mod ports;
pub mod procs;
pub mod programs;
pub mod restart;
pub mod rules;
pub mod scan;
pub mod speed;
pub mod startup;
pub mod system;
pub mod tidy;
pub mod trash;
pub mod util;
pub mod voice;
pub mod winsettings;

use agents::AgentKind;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub use config::Config;
pub use paths::Paths;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub total_mem: u64,
    pub used_mem: u64,
    pub cpus: usize,
    pub processes: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCount {
    pub agent: Option<AgentKind>,
    pub label: String,
    pub leftovers: usize,
    pub mem: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub leftovers: usize,
    pub leftover_mem: u64,
    /// Share of the machine's CPU the leftovers are burning right now.
    pub leftover_cpu: f32,
    /// Whole machine CPU use right now.
    pub machine_cpu: f32,
    pub forgotten: usize,
    pub running: usize,
    pub runtime_mem: u64,
    pub helpers: usize,
    pub helper_mem: u64,
    pub yours: usize,
    pub sessions: usize,
    pub containers: usize,
    /// Leftovers per culprit, worst first.
    pub by_agent: Vec<AgentCount>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub taken_at: u64,
    pub machine: Machine,
    pub sessions: Vec<scan::Session>,
    pub groups: Vec<scan::Group>,
    pub machines: containers::Machines,
    /// What Codex is working on, from its own records (threads and scheduled automations).
    pub codex: codex::CodexState,
    pub summary: Summary,
    pub mood: voice::Mood,
}

pub fn summarize(groups: &[scan::Group], sessions: &[scan::Session], machines: &containers::Machines) -> Summary {
    use scan::Status;
    let mut s = Summary {
        sessions: sessions.len(),
        containers: machines.docker.as_ref().map(|d| d.containers.len()).unwrap_or(0),
        ..Default::default()
    };
    let mut by: BTreeMap<String, AgentCount> = BTreeMap::new();
    for g in groups {
        s.runtime_mem += g.mem;
        if g.is_leftover() {
            s.leftovers += 1;
            s.leftover_mem += g.mem;
            s.leftover_cpu += g.cpu;
            let label = g.owner.agent.map(|a| a.label().to_string()).unwrap_or_else(|| "Nobody".into());
            let e = by.entry(label.clone()).or_insert(AgentCount { agent: g.owner.agent, label, leftovers: 0, mem: 0 });
            e.leftovers += 1;
            e.mem += g.mem;
            continue;
        }
        match g.status {
            Status::Forgotten if !g.kept => s.forgotten += 1,
            Status::Running | Status::Forgotten => s.running += 1,
            Status::Helper => {
                s.helpers += 1;
                s.helper_mem += g.mem;
            }
            Status::Yours => s.yours += 1,
            _ => {}
        }
    }
    let mut v: Vec<AgentCount> = by.into_values().collect();
    v.sort_by(|a, b| b.mem.cmp(&a.mem));
    s.by_agent = v;
    s
}

pub fn build_snapshot(
    table: &procs::ProcTable,
    listeners: &HashMap<u32, Vec<ports::Listener>>,
    cfg: &Config,
    machines: containers::Machines,
) -> Snapshot {
    let a = scan::analyze(table, listeners, cfg);
    let mut summary = summarize(&a.groups, &a.sessions, &machines);
    summary.machine_cpu = table.procs.values().filter(|p| p.pid != 0).map(|p| p.cpu).sum::<f32>().min(100.0);
    let mood = voice::mood(summary.leftovers, summary.leftover_mem, summary.forgotten);
    Snapshot {
        taken_at: table.taken_at,
        machine: Machine {
            total_mem: table.total_mem,
            used_mem: table.used_mem,
            cpus: table.cpus,
            processes: table.procs.len(),
        },
        sessions: a.sessions,
        groups: a.groups,
        machines,
        codex: codex::CodexState::default(),
        summary,
        mood,
    }
}

/// Adds Codex's own view (threads, automations) and says which thread each Codex process serves.
pub fn attach_codex(snap: &mut Snapshot, state: codex::CodexState) {
    for g in snap.groups.iter_mut().filter(|g| g.owner.agent == Some(AgentKind::Codex)) {
        let Some(cwd) = g.cwd.as_deref().map(util::norm_path) else { continue };
        let matches: Vec<&codex::Thread> = state
            .threads
            .iter()
            .filter(|t| {
                t.cwd
                    .as_deref()
                    .map(|d| {
                        let d = util::norm_path(d);
                        cwd == d || cwd.starts_with(&format!("{d}\\"))
                    })
                    .unwrap_or(false)
            })
            .collect();
        // Only claim it when exactly one awake thread works in that folder.
        if let [t] = matches.as_slice() {
            let label = if t.name.is_empty() { "a Codex thread".to_string() } else { format!("\"{}\"", t.name) };
            g.why = format!("{} Probably for {label}.", g.why);
        }
    }
    snap.codex = state;
}

/// One-shot: look at the machine right now. Takes two quick looks so CPU numbers mean something.
pub fn snapshot_now(cfg: &Config, with_machines: bool) -> (procs::ProcTable, Snapshot) {
    let mut sampler = procs::Sampler::new();
    let _ = sampler.sample();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let table = sampler.sample();
    let listeners = ports::listeners();
    let machines = if with_machines { containers::machines(&table) } else { Default::default() };
    let mut snap = build_snapshot(&table, &listeners, cfg, machines);
    attach_codex(&mut snap, codex::state(&Paths::detect().codex_dir()));
    (table, snap)
}
