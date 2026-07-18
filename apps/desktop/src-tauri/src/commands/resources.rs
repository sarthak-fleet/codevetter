//! Live resource usage for CodeVetter itself + any children it spawned
//! (chromiumoxide CDP browser, `gh`, `git`, sandbox tasks…).
//!
//! Frontend polls `get_resource_snapshot` ~every 2s and renders a compact
//! chip in the top nav. CPU / RAM / disk come from sysinfo across all
//! platforms. GPU (Apple Silicon) and per-process network are best-effort
//! macOS shell-outs to `ioreg` and `nettop` with strict 800ms timeouts —
//! they return `None` on any error rather than blocking the UI.

use std::collections::HashSet;
#[cfg(target_os = "macos")]
use std::process::Stdio;
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use std::time::Duration;
use std::time::Instant;

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tauri::State;

const MIN_ELAPSED_MS: u128 = 500;
type NetworkSample = (Instant, Option<(u64, u64)>, u64, u64);

pub struct ResourceState {
    sys: Mutex<System>,
    last_refresh: Mutex<Option<Instant>>,
    last_gpu: Mutex<Option<(Instant, Option<f32>)>>,
    last_net: Mutex<Option<NetworkSample>>,
}

impl ResourceState {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::new().with_processes(
                ProcessRefreshKind::new()
                    .with_cpu()
                    .with_memory()
                    .with_disk_usage(),
            ),
        );
        Self {
            sys: Mutex::new(sys),
            last_refresh: Mutex::new(None),
            last_gpu: Mutex::new(None),
            last_net: Mutex::new(None),
        }
    }
}

impl Default for ResourceState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Clone)]
pub struct ProcessSample {
    pub pid: i32,
    pub name: String,
    pub cpu_percent: f32,
    pub ram_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct ResourceSnapshot {
    pub sampled_at: String,
    pub self_pid: i32,
    /// Total CPU usage across self + children, normalized to 0..100 across
    /// all cores. A single-core fully pegged on an 8-core box reads ~12.5%.
    pub cpu_percent: f32,
    pub cpu_count: u32,
    /// Resident set size in bytes (self + children).
    pub ram_bytes: u64,
    pub disk_read_per_sec: u64,
    pub disk_write_per_sec: u64,
    /// macOS Apple-Silicon GPU utilization 0..100, or `None` if unavailable.
    pub gpu_percent: Option<f32>,
    /// Per-process inbound bytes/sec (self + children, macOS only).
    pub net_in_per_sec: Option<u64>,
    pub net_out_per_sec: Option<u64>,
    pub children: Vec<ProcessSample>,
}

#[tauri::command]
pub async fn get_resource_snapshot(
    state: State<'_, ResourceState>,
) -> Result<ResourceSnapshot, String> {
    let self_pid_raw =
        sysinfo::get_current_pid().map_err(|e| format!("could not resolve current pid: {e}"))?;
    let self_pid_i32 = self_pid_raw.as_u32() as i32;

    // ── Refresh + collect process tree ────────────────────────────
    let now = Instant::now();
    let (snapshot_core, sampled_at) = {
        let mut sys = state
            .sys
            .lock()
            .map_err(|e| format!("sys lock poisoned: {e}"))?;
        let mut last = state
            .last_refresh
            .lock()
            .map_err(|e| format!("last lock poisoned: {e}"))?;

        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new()
                .with_cpu()
                .with_memory()
                .with_disk_usage(),
        );

        let elapsed_ms = last.map(|t| now.duration_since(t).as_millis()).unwrap_or(0);
        let denom_ms = elapsed_ms.max(MIN_ELAPSED_MS) as f64;
        *last = Some(now);

        let pids = collect_process_tree(&sys, self_pid_raw);
        let cpu_count = sys.cpus().len().max(1) as u32;

        let mut total_cpu = 0f32;
        let mut total_ram: u64 = 0;
        let mut total_read: u64 = 0;
        let mut total_write: u64 = 0;
        let mut children: Vec<ProcessSample> = Vec::new();

        for pid in &pids {
            if let Some(proc) = sys.process(*pid) {
                let raw_cpu = proc.cpu_usage();
                let mem = proc.memory();
                let du = proc.disk_usage();
                total_cpu += raw_cpu;
                total_ram += mem;
                total_read += du.read_bytes;
                total_write += du.written_bytes;
                if *pid != self_pid_raw {
                    children.push(ProcessSample {
                        pid: pid.as_u32() as i32,
                        name: proc.name().to_string_lossy().to_string(),
                        cpu_percent: raw_cpu / cpu_count as f32,
                        ram_bytes: mem,
                    });
                }
            }
        }
        // Heaviest first so the popover lists the loudest process at top.
        children.sort_by(|a, b| {
            (b.ram_bytes + (b.cpu_percent as u64 * 1024 * 1024))
                .cmp(&(a.ram_bytes + (a.cpu_percent as u64 * 1024 * 1024)))
        });

        let snapshot = ResourceSnapshot {
            sampled_at: String::new(),
            self_pid: self_pid_i32,
            cpu_percent: (total_cpu / cpu_count as f32).min(100.0),
            cpu_count,
            ram_bytes: total_ram,
            disk_read_per_sec: ((total_read as f64) * 1000.0 / denom_ms) as u64,
            disk_write_per_sec: ((total_write as f64) * 1000.0 / denom_ms) as u64,
            gpu_percent: None,
            net_in_per_sec: None,
            net_out_per_sec: None,
            children,
        };

        (snapshot, chrono::Utc::now().to_rfc3339())
    };

    // ── macOS-only: GPU + network (best-effort, off the hot path) ─
    let mut snapshot = snapshot_core;
    snapshot.sampled_at = sampled_at;

    #[cfg(target_os = "macos")]
    {
        snapshot.gpu_percent = sample_gpu_macos(&state.last_gpu);
        let pids_for_net = std::iter::once(self_pid_i32 as u32)
            .chain(snapshot.children.iter().map(|c| c.pid as u32))
            .collect::<HashSet<_>>();
        if let Some((rx, tx)) = sample_net_macos(&state.last_net, &pids_for_net) {
            snapshot.net_in_per_sec = Some(rx);
            snapshot.net_out_per_sec = Some(tx);
        }
    }

    Ok(snapshot)
}

fn collect_process_tree(sys: &System, root: Pid) -> Vec<Pid> {
    let mut out = vec![root];
    let mut seen: HashSet<Pid> = HashSet::new();
    seen.insert(root);
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for (pid, proc) in sys.processes() {
            if proc.parent() == Some(parent) && seen.insert(*pid) {
                out.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────
// macOS GPU sampling via `ioreg`
// ─────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn sample_gpu_macos(cache: &Mutex<Option<(Instant, Option<f32>)>>) -> Option<f32> {
    // Cache for 1.5s so back-to-back chip refreshes don't fork ioreg every tick.
    if let Ok(guard) = cache.lock() {
        if let Some((when, value)) = *guard {
            if when.elapsed() < Duration::from_millis(1500) {
                return value;
            }
        }
    }

    let raw = run_with_timeout(
        "ioreg",
        &["-r", "-d", "1", "-c", "AGXAccelerator", "-w", "0"],
        800,
    )?;
    let value = parse_ioreg_gpu(&raw);
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), value));
    }
    value
}

#[cfg(target_os = "macos")]
fn parse_ioreg_gpu(raw: &str) -> Option<f32> {
    // ioreg prints lines like:
    //   "Device Utilization %"=23
    for line in raw.lines() {
        if let Some(eq) = line.find("\"Device Utilization %\"=") {
            let rest = &line[eq + "\"Device Utilization %\"=".len()..];
            let n: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = n.parse::<f32>() {
                return Some(v);
            }
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────
// macOS network sampling via `nettop`
// ─────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn sample_net_macos(
    cache: &Mutex<Option<NetworkSample>>,
    pids: &HashSet<u32>,
) -> Option<(u64, u64)> {
    // `nettop -P` is slow (~400-700ms cold). Re-sample at most every 2s and
    // compute a rate from the cumulative byte counters.
    let prev = cache.lock().ok().and_then(|g| *g);
    if let Some((when, _, _, _)) = prev {
        if when.elapsed() < Duration::from_millis(1500) {
            if let Some((_, rate, _, _)) = prev {
                return rate;
            }
        }
    }

    let raw = run_with_timeout(
        "nettop",
        &["-P", "-L", "1", "-J", "bytes_in,bytes_out", "-x"],
        800,
    )?;
    let (cum_rx, cum_tx) = parse_nettop(&raw, pids);
    let now = Instant::now();
    let rate = match prev {
        Some((then, _, prev_rx, prev_tx)) => {
            let elapsed_ms = now.duration_since(then).as_millis().max(MIN_ELAPSED_MS);
            let drx = cum_rx.saturating_sub(prev_rx);
            let dtx = cum_tx.saturating_sub(prev_tx);
            Some((
                ((drx as f64) * 1000.0 / elapsed_ms as f64) as u64,
                ((dtx as f64) * 1000.0 / elapsed_ms as f64) as u64,
            ))
        }
        None => Some((0, 0)),
    };
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((now, rate, cum_rx, cum_tx));
    }
    rate
}

#[cfg(target_os = "macos")]
fn parse_nettop(raw: &str, pids: &HashSet<u32>) -> (u64, u64) {
    // nettop -x emits CSV like:
    //   time,,bytes_in,bytes_out
    //   00:00:00.000000,Chrome.10242,12345,6789
    // The pid is appended to the process name after a dot.
    let mut rx = 0u64;
    let mut tx = 0u64;
    for line in raw.lines().skip(1) {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 4 {
            continue;
        }
        let name = cols[1];
        let pid: u32 = match name.rsplit_once('.') {
            Some((_, pid_str)) => match pid_str.trim().parse() {
                Ok(p) => p,
                Err(_) => continue,
            },
            None => continue,
        };
        if !pids.contains(&pid) {
            continue;
        }
        rx += cols[2].trim().parse::<u64>().unwrap_or(0);
        tx += cols[3].trim().parse::<u64>().unwrap_or(0);
    }
    (rx, tx)
}

#[cfg(target_os = "macos")]
fn run_with_timeout(bin: &str, args: &[&str], timeout_ms: u64) -> Option<String> {
    use std::io::Read;
    let mut child = std::process::Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(_) => return None,
        }
    }
    let mut out = String::new();
    child.stdout?.read_to_string(&mut out).ok()?;
    Some(out)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn parses_ioreg_device_utilization() {
        let raw = r#"
        | | |   "PerformanceStatistics" = {
        | | |     "Device Utilization %"=37
        | | |     "GPU Activity(%)"=37
        | | |   }
        "#;
        assert_eq!(super::parse_ioreg_gpu(raw), Some(37.0));
    }

    #[test]
    fn ioreg_missing_field_returns_none() {
        let raw = "| | | random stuff that does not include the marker";
        assert_eq!(super::parse_ioreg_gpu(raw), None);
    }

    #[test]
    fn nettop_filters_to_tracked_pids() {
        let raw = "time,,bytes_in,bytes_out\n\
                   ts,FooApp.123,1000,500\n\
                   ts,Other.456,9999,9999\n\
                   ts,Bar.789,200,100\n";
        let mut pids = HashSet::new();
        pids.insert(123u32);
        pids.insert(789u32);
        let (rx, tx) = parse_nettop(raw, &pids);
        assert_eq!(rx, 1200);
        assert_eq!(tx, 600);
    }

    #[test]
    fn nettop_skips_malformed_rows() {
        let raw = "header,header,header,header\n\
                   bogus\n\
                   ts,NoPid,1,2\n\
                   ts,Good.42,7,3\n";
        let mut pids = HashSet::new();
        pids.insert(42u32);
        let (rx, tx) = parse_nettop(raw, &pids);
        assert_eq!(rx, 7);
        assert_eq!(tx, 3);
    }
}
