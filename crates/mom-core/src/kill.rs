//! Turning things off, carefully.
//!
//! Rules mom never breaks:
//! * She checks each process's start time right before stopping it, so a recycled PID is
//!   never mistaken for the original.
//! * She never stops herself, whoever ran her (the agent and its shell), agents themselves,
//!   or Windows' own processes.
//! * Children go first, then the parent, so nothing gets re-spawned by a watcher.

use crate::agents;
use crate::classify;
use crate::procs::ProcTable;
use crate::scan::Group;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillReport {
    pub stopped: Vec<u32>,
    pub already_gone: Vec<u32>,
    pub failed: Vec<KillFailure>,
    pub freed_mem: u64,
    pub groups: u32,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillFailure {
    pub pid: u32,
    pub name: String,
    pub reason: String,
}

/// Processes mom refuses to stop no matter what she's asked.
pub(crate) fn untouchable(table: &ProcTable) -> HashSet<u32> {
    let mut s = table.self_and_ancestors();
    for p in table.procs.values() {
        if agents::agent_process(p).is_some() || classify::is_critical(p) || p.pid <= 4 {
            s.insert(p.pid);
        }
    }
    s
}

/// Stops each group: its root and every descendant, deepest first.
pub fn stop_groups(table: &ProcTable, groups: &[&Group]) -> KillReport {
    let protected = untouchable(table);
    let mut report = KillReport::default();
    let mut done: HashSet<u32> = HashSet::new();
    for g in groups {
        let Some(root) = table.get(g.root_pid) else {
            report.already_gone.push(g.root_pid);
            continue;
        };
        if root.start.abs_diff(g.root_start) > 2 {
            // That PID belongs to someone else now. The original is long gone.
            report.already_gone.push(g.root_pid);
            continue;
        }
        if protected.contains(&g.root_pid) {
            // Not a group mom is allowed to stop at all (an agent, Windows, or herself).
            report.failed.push(KillFailure {
                pid: g.root_pid,
                name: root.name.clone(),
                reason: "protected (agent, system process, or whoever asked)".into(),
            });
            continue;
        }
        report.groups += 1;
        let mut order = vec![g.root_pid];
        order.extend(table.descendants_until(g.root_pid, |p| protected.contains(&p.pid)));
        for pid in order.into_iter().rev() {
            if !done.insert(pid) {
                continue;
            }
            let Some(p) = table.get(pid) else { continue };
            if protected.contains(&pid) {
                report.failed.push(KillFailure {
                    pid,
                    name: p.name.clone(),
                    reason: "protected (agent, system process, or whoever asked)".into(),
                });
                continue;
            }
            match terminate(pid, p.start) {
                Ok(true) => {
                    report.stopped.push(pid);
                    report.freed_mem += p.mem;
                }
                Ok(false) => report.already_gone.push(pid),
                Err(reason) => report.failed.push(KillFailure { pid, name: p.name.clone(), reason }),
            }
        }
    }
    report.message = crate::voice::stopped(report.groups as usize, report.freed_mem);
    if !report.failed.is_empty() {
        report.message.push_str(&format!(" ({} wouldn't budge.)", report.failed.len()));
    }
    report
}

/// Ok(true): stopped. Ok(false): it was already gone (or the PID was recycled). Err: couldn't.
#[cfg(windows)]
pub fn terminate(pid: u32, expected_start: u64) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };
    const SYNCHRONIZE: u32 = 0x0010_0000;
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return match GetLastError() {
                87 => Ok(false), // ERROR_INVALID_PARAMETER: no such process
                5 => Err("access denied (it may be running as admin)".into()),
                e => Err(format!("couldn't open it (error {e})")),
            };
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if expected_start != 0 && GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) != 0 {
            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            let start = (ticks / 10_000_000).saturating_sub(11_644_473_600);
            if start.abs_diff(expected_start) > 2 {
                CloseHandle(h);
                return Ok(false);
            }
        }
        let ok = TerminateProcess(h, 1) != 0;
        let err = GetLastError();
        if ok {
            WaitForSingleObject(h, 3000);
        }
        CloseHandle(h);
        if ok {
            Ok(true)
        } else if err == 5 {
            // Already exiting processes report access denied.
            Ok(false)
        } else {
            Err(format!("TerminateProcess failed (error {err})"))
        }
    }
}

#[cfg(not(windows))]
pub fn terminate(pid: u32, _expected_start: u64) -> Result<bool, String> {
    let out = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(out.status.success())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::procs::Sampler;

    #[test]
    fn stops_a_real_child_process_and_respects_start_times() {
        // A harmless child that would otherwise wait for a minute.
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 60 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let t = Sampler::new().sample();
        let p = t.get(pid).expect("child should be visible").clone();

        // Wrong start time: mom must refuse to touch it.
        assert_eq!(terminate(pid, p.start + 1000), Ok(false));
        assert!(child.try_wait().unwrap().is_none(), "still running");

        assert_eq!(terminate(pid, p.start), Ok(true));
        assert!(child.wait().is_ok());
    }

    #[test]
    fn never_stops_herself() {
        let t = Sampler::new().sample();
        let me = t.get(std::process::id()).unwrap().clone();
        let g = crate::scan::Group {
            id: "x".into(),
            root_pid: me.pid,
            root_start: me.start,
            name: "me".into(),
            kind: crate::classify::Kind::Tool,
            category: crate::classify::Category::Script,
            project: None,
            cwd: None,
            command: String::new(),
            pids: vec![me.pid],
            ports: vec![],
            mem: 0,
            cpu: 0.0,
            started: me.start,
            age: 0,
            owner: crate::scan::Owner {
                kind: crate::scan::OwnerKind::Nobody,
                agent: None,
                label: String::new(),
                session: None,
                role: crate::agents::Role::Unknown,
                alive: false,
                session_alive: false,
                agent_pid: None,
                guessed: false,
            },
            status: crate::scan::Status::Orphaned,
            dupes: 0,
            signature: String::new(),
            kept: false,
            why: String::new(),
        };
        let r = stop_groups(&t, &[&g]);
        assert!(r.stopped.is_empty());
        assert_eq!(r.failed.len(), 1);
    }
}
