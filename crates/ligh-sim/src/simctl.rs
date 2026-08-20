use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use ligh_core::LighError;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct Simctl;

impl Simctl {
    pub fn run(args: &[&str]) -> Result<Output, LighError> {
        let mut command = Command::new("xcrun");
        command.arg("simctl");
        command.args(args);
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(LighError::Io)?;

        if output.status.success() {
            Ok(output)
        } else {
            Err(LighError::Simctl(format!(
                "simctl {} → {}\n{}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    pub fn run_ok(args: &[&str]) -> Result<String, LighError> {
        Ok(String::from_utf8_lossy(&Self::run(args)?.stdout)
            .trim()
            .to_string())
    }

    /// Run a program inside the simulator (no shell — avoids DYLD issues with `/bin/sh -c`).
    pub fn spawn_argv(udid: &str, argv: &[&str]) -> Result<String, LighError> {
        debug!(%udid, prog = ?argv.first(), "simctl spawn");
        let mut command = Command::new("xcrun");
        command.arg("simctl").arg("spawn").arg(udid);
        command.args(argv);
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(LighError::Io)?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if output.status.success() {
            Ok(stdout)
        } else {
            Err(LighError::Simctl(format!(
                "spawn failed: {stderr}\n{stdout}"
            )))
        }
    }

    /// Run a shell script inside the simulator in a **single** spawn (batch ops here).
    pub fn spawn_sh(udid: &str, script: &str) -> Result<String, LighError> {
        Self::spawn_argv(udid, &["/bin/sh", "-c", script])
    }

    /// Spawn with wall-clock timeout — kills the child if simctl blocks during early boot.
    pub fn spawn_timeout(
        udid: &str,
        argv: &[&str],
        timeout: Duration,
    ) -> Result<String, LighError> {
        let mut child = Command::new("xcrun");
        child
            .arg("simctl")
            .arg("spawn")
            .arg(udid)
            .args(argv)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = child.spawn().map_err(LighError::Io)?;

        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait().map_err(LighError::Io)? {
                Some(status) => {
                    let mut stdout = String::new();
                    let mut stderr = String::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = out.read_to_string(&mut stdout);
                    }
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut stderr);
                    }
                    let stdout = stdout.trim().to_string();
                    let stderr = stderr.trim().to_string();
                    if status.success() {
                        return Ok(stdout);
                    }
                    return Err(LighError::Simctl(format!(
                        "spawn failed: {stderr}\n{stdout}"
                    )));
                }
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(LighError::NotReady(format!(
                        "simctl spawn timed out after {}s",
                        timeout.as_secs()
                    )));
                }
                None => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    pub fn spawn_sh_timeout(
        udid: &str,
        script: &str,
        timeout: Duration,
    ) -> Result<String, LighError> {
        Self::spawn_timeout(udid, &["/bin/sh", "-c", script], timeout)
    }

    /// Shut down every booted simulator except `keep` (avoids multi-boot RAM / spawn hangs).
    pub fn shutdown_other_booted(keep: &str) -> Result<(), LighError> {
        let json = Self::run_ok(&["list", "devices", "booted", "-j"])?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        if let Some(map) = value.get("devices").and_then(|d| d.as_object()) {
            for list in map.values() {
                for dev in list.as_array().into_iter().flatten() {
                    let Some(udid) = dev.get("udid").and_then(|u| u.as_str()) else {
                        continue;
                    };
                    if udid != keep {
                        let _ = Self::run(&["shutdown", udid]);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn is_booted(udid: &str) -> Result<bool, LighError> {
        let json = Self::run_ok(&["list", "devices", "booted", "-j"])?;
        Ok(json.contains(udid))
    }

    pub fn booted_count() -> Result<usize, LighError> {
        let json = Self::run_ok(&["list", "devices", "booted", "-j"])?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        let mut n = 0usize;
        if let Some(map) = value.get("devices").and_then(|d| d.as_object()) {
            for list in map.values() {
                for dev in list.as_array().into_iter().flatten() {
                    if dev.get("state").and_then(|s| s.as_str()) == Some("Booted") {
                        n += 1;
                    }
                }
            }
        }
        Ok(n)
    }

    /// Ready = userspace responds to spawn. Do NOT use bootstatus -b (hangs on SpringBoard).
    pub fn wait_ready(udid: &str, timeout: Duration) -> Result<(), LighError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LighError::BootTimeout {
                    udid: udid.to_string(),
                    seconds: timeout.as_secs(),
                });
            }
            // simctl spawn can block ~60s on cold boot; cap per attempt so we can retry
            // on immediate DYLD/launchd failures without waiting the full budget.
            let slice = remaining.min(Duration::from_secs(75));
            match Self::spawn_timeout(udid, &["/bin/echo", "ready"], slice) {
                Ok(_) => return Ok(()),
                Err(LighError::NotReady(_)) | Err(LighError::Simctl(_)) => {
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// SpringBoard is a host process under the sim runtime — don't simctl-spawn for this.
    pub fn wait_springboard(_udid: &str, timeout: Duration) -> Result<(), LighError> {
        let start = Instant::now();
        loop {
            if Self::springboard_up_host() {
                std::thread::sleep(Duration::from_millis(500));
                return Ok(());
            }
            if start.elapsed() >= timeout {
                return Err(LighError::BootTimeout {
                    udid: _udid.to_string(),
                    seconds: timeout.as_secs(),
                });
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    fn springboard_up_host() -> bool {
        Command::new("pgrep")
            .args(["-f", "SpringBoard.app/SpringBoard"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
