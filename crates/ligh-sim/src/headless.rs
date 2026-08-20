//! Headless boot — `--disabledJob` after UDID (Apple's required arg order) + optional runtime slim.

use std::process::{Command, Stdio};
use std::time::Duration;

use ligh_core::{FeatureRequirements, LighError, resolve_disabled_jobs};
use tracing::{info, warn};

use crate::runtime::RuntimeSlim;
use crate::simctl::Simctl;

pub struct HeadlessBoot {
    pub requirements: FeatureRequirements,
    /// Apply session slim (disable+bootout) after boot. Default true.
    pub runtime_slim: bool,
}

impl Default for HeadlessBoot {
    fn default() -> Self {
        Self {
            requirements: FeatureRequirements::default(),
            runtime_slim: false,
        }
    }
}

impl HeadlessBoot {
    pub fn boot(&self, udid: &str) -> Result<BootReport, LighError> {
        if Simctl::is_booted(udid)? {
            return Ok(BootReport {
                udid: udid.to_string(),
                disabled_jobs: 0,
                runtime_slim: None,
                already_booted: true,
                headless: true,
            });
        }

        let jobs = resolve_disabled_jobs(&self.requirements);
        let applied = boot_with_jobs(udid, &jobs)?;
        Simctl::wait_ready(udid, Duration::from_secs(120))?;

        let runtime_slim = if self.runtime_slim {
            Some(RuntimeSlim::apply(udid)?)
        } else {
            None
        };

        // Let memory reclaim after bootouts.
        std::thread::sleep(Duration::from_secs(3));

        Ok(BootReport {
            udid: udid.to_string(),
            disabled_jobs: applied,
            runtime_slim,
            already_booted: false,
            headless: true,
        })
    }
}

/// Boot with `--disabledJob` flags. UDID must precede flags (simctl requirement).
fn boot_with_jobs(udid: &str, jobs: &[String]) -> Result<usize, LighError> {
    if jobs.is_empty() {
        Simctl::run(&["boot", udid])?;
        return Ok(0);
    }

    if try_boot(udid, jobs).is_ok() {
        info!(count = jobs.len(), "boot with disabledJob profile");
        return Ok(jobs.len());
    }

    warn!("disabledJob boot rejected — plain boot (runtime slim still applies)");
    Simctl::run(&["boot", udid])?;
    Ok(0)
}

fn try_boot(udid: &str, jobs: &[String]) -> Result<(), LighError> {
    let mut cmd = Command::new("xcrun");
    cmd.arg("simctl").arg("boot").arg(udid);
    for label in jobs {
        cmd.arg(format!("--disabledJob={label}"));
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(LighError::Io)?;
    if output.status.success() || Simctl::is_booted(udid)? {
        Ok(())
    } else {
        Err(LighError::Simctl(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct BootReport {
    pub udid: String,
    pub disabled_jobs: usize,
    pub runtime_slim: Option<crate::runtime::RuntimeSlimReport>,
    pub already_booted: bool,
    pub headless: bool,
}

pub fn ensure_headless() {
    let _ = Command::new("osascript")
        .args([
            "-e",
            "if application \"Simulator\" is running then tell application \"Simulator\" to quit",
        ])
        .status();
}
