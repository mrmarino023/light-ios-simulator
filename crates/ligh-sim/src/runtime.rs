//! Session slim — batched launchctl disable + bootout inside one sim spawn.
//!
//! Faster than SimSlim's disable→reboot→boot loop: kills daemons in-place, no reboot.
//! Pair with `--disabledJob` at boot so they stay dead.

use std::process::{Command, Stdio};
use std::time::Duration;

use ligh_core::{LighError, slim_labels};
use serde::Serialize;
use tracing::info;

const BATCH_SCRIPT: &str = r#"[ -n "$SIMULATOR_ROOT" ] && export DYLD_ROOT_PATH="$SIMULATOR_ROOT"
action=$1; wave=$2; shift 2
n=0
for l in "$@"; do
  { launchctl "$action" "system/$l" 2>/dev/null && echo "ligh-ok $l" || echo "ligh-fail $l"; } &
  n=$((n + 1))
  [ $((n % wave)) -eq 0 ] && wait
done
wait
exit 0"#;

const BATCH_SIZE: usize = 40;
const BATCH_WAVE: &str = "8";

static LABELS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

pub fn managed_labels() -> &'static [String] {
    LABELS.get_or_init(|| slim_labels().iter().map(|s| (*s).to_string()).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSlimReport {
    pub labels_targeted: usize,
    pub disable_ok: usize,
    pub bootout_ok: usize,
}

pub struct RuntimeSlim;

impl RuntimeSlim {
    /// Disable + bootout all managed labels in chunked parallel batches.
    pub fn apply(udid: &str) -> Result<RuntimeSlimReport, LighError> {
        let labels = managed_labels();
        info!(count = labels.len(), "runtime slim starting");

        let disable_ok = run_action_batches(udid, "disable", &labels)?;
        let bootout_ok = run_action_batches(udid, "bootout", &labels)?;

        info!(
            disable_ok,
            bootout_ok,
            targeted = labels.len(),
            "runtime slim complete"
        );

        Ok(RuntimeSlimReport {
            labels_targeted: labels.len(),
            disable_ok,
            bootout_ok,
        })
    }
}

fn run_action_batches(udid: &str, action: &str, labels: &[String]) -> Result<usize, LighError> {
    let mut ok_total = 0usize;
    for chunk in labels.chunks(BATCH_SIZE) {
        let chunk_ok = run_batch(udid, action, chunk)?;
        ok_total += chunk_ok;
    }
    Ok(ok_total)
}

fn run_batch(udid: &str, action: &str, labels: &[String]) -> Result<usize, LighError> {
    let mut cmd = Command::new("xcrun");
    cmd.args(["simctl", "spawn", udid, "/bin/sh", "-c", BATCH_SCRIPT, "ligh-batch"]);
    cmd.arg(action).arg(BATCH_WAVE);
    for label in labels {
        cmd.arg(label);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = cmd.output().map_err(LighError::Io)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ok = stdout.lines().filter(|l| l.starts_with("ligh-ok ")).count();

    if !output.status.success() && ok == 0 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LighError::Simctl(format!(
            "batch {action} failed: {}",
            stderr.trim()
        )));
    }

    // Brief pause so launchd settles between chunks.
    std::thread::sleep(Duration::from_millis(200));
    Ok(ok)
}
