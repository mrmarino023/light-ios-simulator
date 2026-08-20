use std::path::Path;
use std::time::{Duration, Instant};

use ligh_core::{DevicePreset, FeatureRequirements, LighConfig, LighError, SessionState};
use serde::Serialize;
use tracing::info;

use crate::device::{DeviceManager, SimDevice};
use crate::headless::{HeadlessBoot, ensure_headless};
use crate::measure::{self, FootprintReport};

pub struct SimSupervisor {
    config: LighConfig,
    requirements: FeatureRequirements,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub session: Option<SessionState>,
    pub device: Option<SimDevice>,
    pub booted: bool,
    pub disk_free_mb: u64,
    pub footprint: Option<FootprintReport>,
    pub disabled_at_boot: Option<usize>,
}

impl SimSupervisor {
    pub fn new(config: LighConfig) -> Self {
        Self {
            config,
            requirements: FeatureRequirements::default(),
        }
    }

    pub fn with_requirements(mut self, req: FeatureRequirements) -> Self {
        self.requirements = req;
        self
    }

    pub fn doctor(&self) -> Result<(), LighError> {
        let free = measure::disk_available_mb();
        if free < 2048 {
            return Err(LighError::DiskSpace { available_mb: free });
        }
        crate::simctl::Simctl::run(&["list", "runtimes"])?;
        Ok(())
    }

    pub fn device_create(&self, preset: DevicePreset) -> Result<SessionState, LighError> {
        let device = DeviceManager::create(preset)?;
        info!(udid = %device.udid, "created fresh LIGH simulator");
        self.persist_session(&device, true, preset, None, None)
    }

    pub fn up(
        &self,
        preset: DevicePreset,
        headless: bool,
        explicit_udid: Option<&str>,
    ) -> Result<SessionState, LighError> {
        if headless {
            ensure_headless();
        }

        let session_udid = SessionState::load(&self.config.state_dir)?.map(|s| s.udid);
        let device = DeviceManager::resolve(preset, explicit_udid, session_udid.as_deref())?;

        let boot = HeadlessBoot {
            requirements: self.requirements.clone(),
            runtime_slim: false,
        };
        let report = boot.boot(&device.udid)?;
        info!(
            udid = %device.udid,
            disabled = report.disabled_jobs,
            already = report.already_booted,
            "headless ready"
        );

        self.persist_session(&device, true, preset, None, None)
    }

    pub fn down(&self) -> Result<(), LighError> {
        if let Some(session) = SessionState::load(&self.config.state_dir)? {
            let _ = crate::simctl::Simctl::run(&["shutdown", &session.udid]);
        }
        // Also shut down stray LIGH sims left booted from failed runs.
        let json = crate::simctl::Simctl::run_ok(&["list", "devices", "booted", "-j"]).ok();
        if let Some(json) = json {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(map) = value.get("devices").and_then(|d| d.as_object()) {
                    for list in map.values() {
                        for dev in list.as_array().into_iter().flatten() {
                            let name = dev.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            if name.starts_with("LIGH") {
                                if let Some(udid) = dev.get("udid").and_then(|u| u.as_str()) {
                                    let _ = crate::simctl::Simctl::run(&["shutdown", udid]);
                                }
                            }
                        }
                    }
                }
            }
        }
        SessionState::clear(&self.config.state_dir)
    }

    pub fn status(&self) -> Result<StatusReport, LighError> {
        let session = SessionState::load(&self.config.state_dir)?;
        let device = session
            .as_ref()
            .map(|s| DeviceManager::get(&s.udid))
            .transpose()?;
        let booted = session
            .as_ref()
            .map(|s| crate::simctl::Simctl::is_booted(&s.udid))
            .transpose()?
            .unwrap_or(false);
        let footprint = session
            .as_ref()
            .map(|s| measure::measure_for_udid(&s.udid))
            .transpose()?;

        Ok(StatusReport {
            session,
            device,
            booted,
            disk_free_mb: measure::disk_available_mb(),
            footprint,
            disabled_at_boot: Some(ligh_core::resolve_disabled_jobs(&self.requirements).len()),
        })
    }

    pub fn install_and_launch(
        &self,
        app_path: &Path,
        bundle_id: Option<&str>,
    ) -> Result<SessionState, LighError> {
        let mut session = self.require_session()?;
        let app_path = app_path
            .canonicalize()
            .map_err(|e| LighError::Simctl(format!("invalid app: {e}")))?;

        let t = Instant::now();
        crate::simctl::Simctl::run(&["install", &session.udid, app_path.to_str().unwrap()])?;
        let bundle_id = match bundle_id {
            Some(id) => id.to_string(),
            None => detect_bundle_id(&app_path)?,
        };
        let _ = crate::simctl::Simctl::run(&["terminate", &session.udid, &bundle_id]);
        crate::simctl::Simctl::run(&[
            "launch",
            &session.udid,
            &bundle_id,
            "--terminate-running-process",
        ])?;
        info!(ms = t.elapsed().as_millis(), "app launched");

        session.app_bundle_id = Some(bundle_id);
        session.app_path = Some(app_path);
        session.save(&self.config.state_dir)?;
        Ok(session)
    }

    pub fn relaunch(&self) -> Result<Duration, LighError> {
        let session = self.require_session()?;
        let bundle_id = session
            .app_bundle_id
            .clone()
            .ok_or_else(|| LighError::Simctl("no app — run `ligh run`".into()))?;
        let t = Instant::now();
        if let Some(p) = &session.app_path {
            crate::simctl::Simctl::run(&["install", &session.udid, p.to_str().unwrap()])?;
        }
        let _ = crate::simctl::Simctl::run(&["terminate", &session.udid, &bundle_id]);
        crate::simctl::Simctl::run(&[
            "launch",
            &session.udid,
            &bundle_id,
            "--terminate-running-process",
        ])?;
        Ok(t.elapsed())
    }

    pub fn measure(&self) -> Result<FootprintReport, LighError> {
        let s = self.require_session()?;
        measure::measure_for_udid(&s.udid)
    }

    fn require_session(&self) -> Result<SessionState, LighError> {
        SessionState::load(&self.config.state_dir)?.ok_or(LighError::NoSession)
    }

    fn persist_session(
        &self,
        device: &SimDevice,
        slim_applied: bool,
        preset: DevicePreset,
        app_bundle_id: Option<String>,
        app_path: Option<std::path::PathBuf>,
    ) -> Result<SessionState, LighError> {
        let session = SessionState {
            udid: device.udid.clone(),
            device: preset,
            name: device.name.clone(),
            slim_applied,
            app_bundle_id,
            app_path,
        };
        session.save(&self.config.state_dir)?;
        Ok(session)
    }
}

fn detect_bundle_id(app_path: &Path) -> Result<String, LighError> {
    let output = std::process::Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(app_path.join("Info.plist"))
        .output()
        .map_err(LighError::Io)?;
    if !output.status.success() {
        return Err(LighError::Simctl("pass --bundle-id".into()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
