//! Host-path benchmark: stock Simulator.app stack vs LIGH (no Simulator.app).

use std::process::Command;
use std::time::{Duration, Instant};

use ligh_core::{DevicePreset, LighError};
use serde::Serialize;
use tracing::{info, warn};

use crate::device::DeviceManager;
use crate::headless::{ensure_headless, HeadlessBoot};
use crate::measure;
use crate::simctl::Simctl;
use ligh_host::HostSession;

#[derive(Debug, Serialize)]
pub struct BenchReport {
    pub stock_boot_secs: f64,
    pub stock_ram_mb: f64,
    pub stock_procs: usize,
    pub stock_simulator_app_mb: f64,
    pub ligh_boot_secs: f64,
    pub ligh_ram_mb: f64,
    pub ligh_procs: usize,
    pub ligh_simulator_app_mb: f64,
    pub disabled_at_boot: usize,
    pub runtime_disable_ok: usize,
    pub runtime_bootout_ok: usize,
    pub ram_saved_mb: f64,
    pub ram_saved_pct: f64,
    pub host_saved_mb: f64,
}

pub struct Benchmark;

impl Benchmark {
    /// Compare stock (simctl + Simulator.app) vs LIGH host (private boot, no Simulator.app).
    pub fn run(preset: DevicePreset) -> Result<BenchReport, LighError> {
        Simctl::run(&["shutdown", "all"]).ok();
        ensure_headless();
        std::thread::sleep(Duration::from_secs(3));

        info!("bench: stock path (simctl boot + Simulator.app)");
        let stock_dev = DeviceManager::resolve(preset, None, None)?;
        let t0 = Instant::now();
        Simctl::run(&["boot", &stock_dev.udid])?;
        Simctl::wait_ready(&stock_dev.udid, Duration::from_secs(180))?;
        let _ = Simctl::wait_springboard(&stock_dev.udid, Duration::from_secs(60));
        open_simulator_app(&stock_dev.udid)?;
        let stock_boot = t0.elapsed().as_secs_f64();
        std::thread::sleep(Duration::from_secs(15));
        let stock_fp = measure::measure_for_udid(&stock_dev.udid)?;
        let stock_sim_app = measure::simulator_app_mb();

        Simctl::run(&["shutdown", &stock_dev.udid]).ok();
        ensure_headless();
        wait_down(&stock_dev.udid)?;
        Simctl::run(&["shutdown", "all"]).ok();
        std::thread::sleep(Duration::from_secs(4));

        info!("bench: LIGH path (private boot, no Simulator.app)");
        ensure_headless();
        let ligh_dev = DeviceManager::resolve(preset, None, None)?;
        let t1 = Instant::now();
        if HostSession::boot(&ligh_dev.udid).is_err() {
            warn!("private boot failed — simctl headless fallback");
            HeadlessBoot::default().boot(&ligh_dev.udid)?;
        } else {
            Simctl::wait_ready(&ligh_dev.udid, Duration::from_secs(180))?;
            let _ = Simctl::wait_springboard(&ligh_dev.udid, Duration::from_secs(60));
        }
        let ligh_boot = t1.elapsed().as_secs_f64();
        std::thread::sleep(Duration::from_secs(15));
        let ligh_fp = measure::measure_for_udid(&ligh_dev.udid)?;
        let ligh_sim_app = measure::simulator_app_mb();

        let saved = stock_fp.total_mb - ligh_fp.total_mb;
        let pct = if stock_fp.total_mb > 0.0 {
            saved / stock_fp.total_mb * 100.0
        } else {
            0.0
        };
        let host_saved = (stock_sim_app - ligh_sim_app).max(0.0);

        Ok(BenchReport {
            stock_boot_secs: stock_boot,
            stock_ram_mb: stock_fp.total_mb,
            stock_procs: stock_fp.process_count,
            stock_simulator_app_mb: stock_sim_app,
            ligh_boot_secs: ligh_boot,
            ligh_ram_mb: ligh_fp.total_mb,
            ligh_procs: ligh_fp.process_count,
            ligh_simulator_app_mb: ligh_sim_app,
            disabled_at_boot: 0,
            runtime_disable_ok: 0,
            runtime_bootout_ok: 0,
            ram_saved_mb: saved,
            ram_saved_pct: pct,
            host_saved_mb: host_saved,
        })
    }
}

fn open_simulator_app(udid: &str) -> Result<(), LighError> {
    let status = Command::new("open")
        .args(["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid])
        .status()
        .map_err(LighError::Io)?;
    if !status.success() {
        return Err(LighError::Simctl("failed to open Simulator.app".into()));
    }
    for _ in 0..50 {
        if measure::simulator_app_mb() > 5.0 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Ok(())
}

fn wait_down(udid: &str) -> Result<(), LighError> {
    for _ in 0..40 {
        if !Simctl::is_booted(udid)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}
