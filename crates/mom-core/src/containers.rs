//! Docker containers, WSL distros and Hyper-V VMs: the heavy stuff agents spin up and forget.
//!
//! Everything here shells out to the official tools with a timeout and no console window,
//! and only when the thing is actually running (asking a stopped Docker Desktop or WSL is
//! slow and can wake it up).

use crate::procs::ProcTable;
use crate::util::{run, CmdOutput};
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub running_for: String,
    pub ports: String,
    /// docker compose project, if any.
    pub project: Option<String>,
    pub project_dir: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Docker {
    pub running: bool,
    pub containers: Vec<Container>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Distro {
    pub name: String,
    /// Docker Desktop's own engine distro. Stopping it stops Docker.
    pub docker: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wsl {
    pub running: Vec<Distro>,
    /// Memory held by the WSL virtual machine (vmmemWSL).
    pub vm_mem: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Vm {
    pub name: String,
    pub state: String,
    pub mem: u64,
    pub uptime: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machines {
    pub docker: Option<Docker>,
    pub wsl: Option<Wsl>,
    pub vms: Vec<Vm>,
}

const QUICK: Duration = Duration::from_secs(6);

fn proc_running(table: &ProcTable, names: &[&str]) -> bool {
    table.procs.values().any(|p| names.contains(&p.lname().as_str()))
}

pub fn docker_running(table: &ProcTable) -> bool {
    proc_running(table, &["com.docker.backend.exe", "dockerd.exe", "docker desktop.exe", "com.docker.service"])
}

/// Everything at once. Only asks tools whose engine is visibly running.
pub fn machines(table: &ProcTable) -> Machines {
    let mut m = Machines::default();
    if docker_running(table) {
        m.docker = Some(docker());
    }
    let vmmem: u64 = table
        .procs
        .values()
        .filter(|p| matches!(p.lname().as_str(), "vmmemwsl" | "vmmem" | "vmmemwsl.exe" | "vmmem.exe"))
        .map(|p| p.mem)
        .sum();
    if vmmem > 0 || proc_running(table, &["wslhost.exe", "wslrelay.exe"]) {
        m.wsl = Some(Wsl {
            running: wsl_running(),
            vm_mem: vmmem,
        });
    }
    if proc_running(table, &["vmwp.exe"]) {
        m.vms = hyperv_vms();
    }
    m
}

pub fn docker() -> Docker {
    match run("docker", &["ps", "--no-trunc", "--format", "{{json .}}"], QUICK) {
        Ok(out) if out.ok() => Docker {
            running: true,
            containers: parse_docker_ps(&out.text()),
            error: None,
        },
        Ok(out) => Docker {
            running: false,
            containers: vec![],
            error: Some(first_line(&out)),
        },
        Err(e) => Docker {
            running: false,
            containers: vec![],
            error: Some(e.to_string()),
        },
    }
}

fn first_line(out: &CmdOutput) -> String {
    let text = if out.timed_out { "timed out".to_string() } else { out.err_text() };
    text.lines().next().unwrap_or("").trim().to_string()
}

pub fn parse_docker_ps(text: &str) -> Vec<Container> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .map(|v| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let labels = s("Labels");
            let label = |key: &str| {
                labels
                    .split(',')
                    .find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(|x| x.to_string()))
            };
            let id = s("ID");
            Container {
                id: id.chars().take(12).collect(),
                name: s("Names"),
                image: s("Image"),
                status: s("Status"),
                running_for: s("RunningFor"),
                ports: s("Ports"),
                project: label("com.docker.compose.project"),
                project_dir: label("com.docker.compose.project.working_dir"),
            }
        })
        .collect()
}

pub fn docker_stop(ids: &[String]) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut args = vec!["stop"];
    args.extend(ids.iter().map(String::as_str));
    let out = run("docker", &args, Duration::from_secs(60))?;
    anyhow::ensure!(out.ok(), "docker stop failed: {}", first_line(&out));
    Ok(())
}

/// One row of `docker system df`: images, containers, local volumes or build cache.
#[derive(Clone, Debug, PartialEq)]
pub struct DiskUse {
    pub kind: String,
    pub size: u64,
    pub reclaimable: u64,
    pub count: u64,
}

/// What Docker is using on disk, by kind.
pub fn docker_df() -> Option<Vec<DiskUse>> {
    let out = run("docker", &["system", "df", "--format", "{{json .}}"], Duration::from_secs(20)).ok()?;
    if !out.ok() {
        return None;
    }
    Some(parse_docker_df(&out.text()))
}

pub fn parse_docker_df(text: &str) -> Vec<DiskUse> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .map(|v| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            DiskUse {
                kind: s("Type"),
                size: parse_size(&s("Size")),
                reclaimable: parse_size(s("Reclaimable").split_whitespace().next().unwrap_or("")),
                count: s("TotalCount").parse().unwrap_or(0),
            }
        })
        .collect()
}

/// Bytes `docker system prune` could give back (dangling images, stopped containers, build cache).
pub fn docker_reclaimable() -> Option<u64> {
    Some(
        docker_df()?
            .iter()
            .filter(|d| !d.kind.eq_ignore_ascii_case("Local Volumes")) // volumes hold data; prune leaves them alone
            .map(|d| d.reclaimable)
            .sum(),
    )
}

/// "1.2GB" -> bytes.
pub fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let idx = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: f64 = num.trim().parse().unwrap_or(0.0);
    let mult = match unit.trim().to_ascii_uppercase().as_str() {
        "KB" | "K" | "KIB" => 1024.0,
        "MB" | "M" | "MIB" => 1024.0 * 1024.0,
        "GB" | "G" | "GIB" => 1024.0 * 1024.0 * 1024.0,
        "TB" | "T" | "TIB" => 1024.0_f64.powi(4),
        _ => 1.0,
    };
    (n * mult) as u64
}

pub fn docker_prune() -> anyhow::Result<String> {
    let out = run("docker", &["system", "prune", "-f"], Duration::from_secs(300))?;
    anyhow::ensure!(out.ok(), "docker system prune failed: {}", first_line(&out));
    Ok(out.text().lines().last().unwrap_or("").to_string())
}

pub fn wsl_running() -> Vec<Distro> {
    let Ok(out) = run("wsl.exe", &["--list", "--running", "--quiet"], QUICK) else {
        return vec![];
    };
    if !out.ok() {
        return vec![];
    }
    parse_wsl_list(&out.text())
}

pub fn parse_wsl_list(text: &str) -> Vec<Distro> {
    text.lines()
        .map(|l| l.trim().trim_matches('\0').trim_start_matches('*').trim())
        .filter(|l| !l.is_empty() && !l.contains(' '))
        .map(|name| Distro {
            name: name.to_string(),
            docker: name.to_ascii_lowercase().starts_with("docker-desktop"),
        })
        .collect()
}

pub fn wsl_terminate(name: &str) -> anyhow::Result<()> {
    let out = run("wsl.exe", &["--terminate", name], Duration::from_secs(30))?;
    anyhow::ensure!(out.ok(), "wsl --terminate failed: {}", first_line(&out));
    Ok(())
}

pub fn wsl_shutdown() -> anyhow::Result<()> {
    let out = run("wsl.exe", &["--shutdown"], Duration::from_secs(60))?;
    anyhow::ensure!(out.ok(), "wsl --shutdown failed: {}", first_line(&out));
    Ok(())
}

/// Set once Get-VM has failed. Without Hyper-V admin rights it always will (WSL alone is enough
/// to make it look worth asking), and each try costs a PowerShell start-up.
static HYPERV_DENIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Running Hyper-V VMs. Needs Hyper-V admin rights; returns nothing if we don't have them.
pub fn hyperv_vms() -> Vec<Vm> {
    use std::sync::atomic::Ordering;
    if HYPERV_DENIED.load(Ordering::Relaxed) {
        return vec![];
    }
    let script = "Get-VM | Where-Object State -eq 'Running' | ForEach-Object { [pscustomobject]@{ n=$_.Name; s=[string]$_.State; m=[int64]$_.MemoryAssigned; u=[string]$_.Uptime } } | ConvertTo-Json -Compress";
    let Ok(out) = run(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
        Duration::from_secs(15),
    ) else {
        HYPERV_DENIED.store(true, Ordering::Relaxed);
        return vec![];
    };
    if !out.ok() {
        HYPERV_DENIED.store(true, Ordering::Relaxed);
        return vec![];
    }
    let text = out.text();
    let v: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let items = match v {
        serde_json::Value::Array(a) => a,
        other => vec![other],
    };
    items
        .iter()
        .map(|i| Vm {
            name: i.get("n").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
            state: i.get("s").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            mem: i.get("m").and_then(|x| x.as_u64()).unwrap_or(0),
            uptime: i
                .get("u")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

pub fn hyperv_stop(name: &str) -> anyhow::Result<()> {
    let safe: String = name.chars().filter(|c| *c != '\'' && *c != '`').collect();
    let script = format!("Stop-VM -Name '{safe}'");
    let out = run(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
        Duration::from_secs(120),
    )?;
    anyhow::ensure!(out.ok(), "Stop-VM failed: {}", first_line(&out));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_ps_json_lines() {
        let text = r#"{"Command":"\"docker-entrypoint.s…\"","CreatedAt":"2026-10-07 01:22:11 +0200 CEST","ID":"3f1a2b3c4d5e6f7a8b9c","Image":"postgres:16","Labels":"com.docker.compose.project=supabase,com.docker.compose.project.working_dir=C:\\Workspace\\app\\supabase","Names":"supabase_db","Ports":"0.0.0.0:54322->5432/tcp","RunningFor":"3 days ago","Status":"Up 3 days"}
{"ID":"aaaaaaaaaaaaaaaa","Image":"redis","Labels":"","Names":"cache","Ports":"","RunningFor":"2 hours ago","Status":"Up 2 hours"}"#;
        let c = parse_docker_ps(text);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].id, "3f1a2b3c4d5e");
        assert_eq!(c[0].project.as_deref(), Some("supabase"));
        assert_eq!(c[0].project_dir.as_deref(), Some(r"C:\Workspace\app\supabase"));
        assert_eq!(c[1].project, None);
    }

    #[test]
    fn parses_wsl_list() {
        let d = parse_wsl_list("Ubuntu\r\ndocker-desktop\r\n\r\n");
        assert_eq!(d.len(), 2);
        assert!(!d[0].docker && d[1].docker);
        assert!(parse_wsl_list("There are no running distributions.").is_empty());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1.5GB"), 1_610_612_736);
        assert_eq!(parse_size("512MB"), 536_870_912);
        assert_eq!(parse_size("0B"), 0);
    }
}
