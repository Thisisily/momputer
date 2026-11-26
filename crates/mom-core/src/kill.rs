//! Turning things off, carefully.
//!
//! Rules mom never breaks:
//! * She checks each process's start time right before stopping it, so a recycled PID is
//!   never mistaken for the original.
//! * She never stops herself, whoever ran her (the agent and its shell), agents themselves,
//!   or the system's own processes.
//! * Children go first, then the parent, so nothing gets re-spawned by a watcher.
//! * On a Mac she asks first (the signal a program gets to close its files and free its port
//!   on), and only ends what's still there a moment later.

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

#[cfg(unix)]
pub fn terminate(pid: u32, expected_start: u64) -> Result<bool, String> {
    use std::time::{Duration, Instant};
    // PID 0 and negative numbers mean "a whole group of processes" to `kill`. Never.
    let Some(target) = i32::try_from(pid).ok().filter(|p| *p > 1) else {
        return Err("not a process mom may stop".into());
    };
    // Still the one that was asked about? (None where mom can't tell: then it's taken on trust,
    // as it always was off Windows.)
    let same = || crate::procs::started(pid).map(|s| s.is_some_and(|s| expected_start == 0 || s.abs_diff(expected_start) <= 2));
    if same() == Some(false) {
        return Ok(false);
    }
    let send = |signal: i32| -> Result<bool, String> {
        // SAFETY: a signal to one process of ours, by number.
        if unsafe { libc::kill(target, signal) } == 0 {
            return Ok(true);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            Some(libc::EPERM) => Err("not allowed (it may be running as root, or as someone else)".into()),
            _ => Err(format!("couldn't signal it ({})", std::io::Error::last_os_error())),
        }
    };
    let gone_within = |wait: Duration| {
        let until = Instant::now() + wait;
        loop {
            let there = match same() {
                Some(there) => there,
                // SAFETY: signal 0 only asks whether the process is there.
                None => unsafe { libc::kill(target, 0) == 0 },
            };
            if !there {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    // Asked first: a server gets to close what it has open and let go of its port.
    if !send(libc::SIGTERM)? {
        return Ok(false);
    }
    if gone_within(Duration::from_millis(1500)) {
        return Ok(true);
    }
    // It didn't listen (or is busy shutting down). Checked once more, then ended.
    if same() == Some(false) || !send(libc::SIGKILL)? {
        return Ok(true);
    }
    if gone_within(Duration::from_secs(2)) {
        Ok(true)
    } else {
        Err("it's still there after being told to stop".into())
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;

    #[test]
    fn stops_a_real_child_process_and_respects_start_times() {
        let mut child = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let t = crate::procs::Sampler::new().sample();
        let p = t.get(pid).expect("child should be visible").clone();
        assert_eq!(p.session, 1, "it's ours");

        // Wrong start time: mom must refuse to touch it (where she can tell when it started).
        if crate::procs::started(pid).is_some() {
            assert_eq!(terminate(pid, p.start + 1000), Ok(false));
            assert!(child.try_wait().unwrap().is_none(), "still running");
        }
        // Collected as it ends, the way a shell collects what it started.
        let waiter = std::thread::spawn(move || child.wait());
        assert_eq!(terminate(pid, p.start), Ok(true));
        assert!(waiter.join().unwrap().is_ok());
        // Gone already: not an error.
        assert_eq!(terminate(pid, p.start), Ok(false));
    }

    #[test]
    fn a_program_that_ignores_being_asked_is_ended_anyway() {
        let mut child = std::process::Command::new("sh").args(["-c", "trap '' TERM; while :; do sleep 1; done"]).spawn().unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let waiter = std::thread::spawn(move || child.wait());
        assert_eq!(terminate(pid, 0), Ok(true));
        assert!(waiter.join().unwrap().is_ok());
    }

    #[test]
    fn never_signals_a_group_or_the_first_process() {
        assert!(terminate(0, 0).is_err());
        assert!(terminate(1, 0).is_err());
        assert!(terminate(u32::MAX, 0).is_err());
    }
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
