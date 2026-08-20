use std::process::Command;

use ligh_core::LighError;
use serde::Serialize;

use crate::device::DeviceManager;
use crate::simctl::Simctl;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessFootprint {
    pub pid: u32,
    pub name: String,
    pub mb: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FootprintReport {
    pub udid: String,
    pub data_path: String,
    pub total_mb: f64,
    pub process_count: usize,
    pub top: Vec<ProcessFootprint>,
}

/// RAM for this sim: UDID in cmdline + launchd_sim tree + CoreSimulator when sole booted device
/// + Simulator.app / SimRender host processes when present.
pub fn measure_for_udid(udid: &str) -> Result<FootprintReport, LighError> {
    let device = DeviceManager::get(udid)?;
    let data_path = device
        .data_path
        .clone()
        .unwrap_or_else(|| format!("CoreSimulator/Devices/{udid}"));

    let booted_count = Simctl::booted_count()?;
    let single_sim = booted_count == 1;

    let output = Command::new("ps")
        .args(["-ax", "-o", "pid=,rss=,command="])
        .output()
        .map_err(LighError::Io)?;

    let mut processes = Vec::new();
    let mut total_kb = 0u64;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split_whitespace();
        let Some(pid_str) = parts.next() else { continue };
        let Some(rss_str) = parts.next() else { continue };
        let cmd: String = parts.collect::<Vec<_>>().join(" ");

        let udid_hit = cmd.contains(udid) || cmd.contains(&data_path);
        let launchd_hit = cmd.contains("launchd_sim");
        let host_gui = cmd.contains("Simulator.app")
            || cmd.contains("SimRenderServer")
            || cmd.contains("SimulatorTrampoline");
        let core_hit = single_sim
            && (cmd.contains("CoreSimulator")
                || cmd.contains("SimLaunchHost")
                || host_gui);

        if !(udid_hit || launchd_hit || core_hit) {
            continue;
        }

        let Ok(pid) = pid_str.parse::<u32>() else { continue };
        let Ok(rss) = rss_str.parse::<u64>() else { continue };
        total_kb += rss;
        processes.push(ProcessFootprint {
            pid,
            name: cmd.chars().take(72).collect(),
            mb: rss as f64 / 1024.0,
        });
    }

    processes.sort_by(|a, b| b.mb.partial_cmp(&a.mb).unwrap_or(std::cmp::Ordering::Equal));

    Ok(FootprintReport {
        udid: udid.to_string(),
        data_path,
        total_mb: total_kb as f64 / 1024.0,
        process_count: processes.len(),
        top: processes.into_iter().take(12).collect(),
    })
}

/// RSS of Simulator.app (+ helper) only — the host GUI LIGH replaces.
pub fn simulator_app_mb() -> f64 {
    let Ok(output) = Command::new("ps")
        .args(["-ax", "-o", "rss=,command="])
        .output()
    else {
        return 0.0;
    };
    let mut total_kb = 0u64;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split_whitespace();
        let Some(rss_str) = parts.next() else { continue };
        let cmd: String = parts.collect::<Vec<_>>().join(" ");
        if cmd.contains("Simulator.app/Contents/MacOS/Simulator")
            || cmd.contains("SimRenderServer")
            || cmd.contains("SimulatorTrampoline")
        {
            if let Ok(rss) = rss_str.parse::<u64>() {
                total_kb += rss;
            }
        }
    }
    total_kb as f64 / 1024.0
}

pub fn disk_available_mb() -> u64 {
    Command::new("df")
        .args(["-k", "/"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .nth(1)
                .and_then(|l| l.split_whitespace().nth(3))
                .and_then(|k| k.parse::<u64>().ok())
        })
        .map(|k| k / 1024)
        .unwrap_or(0)
}
