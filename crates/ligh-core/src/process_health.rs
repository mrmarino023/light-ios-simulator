//! Process health — distinguish crash loops from “no chrome” / SpringBoard.
//!
//! Agent paradise: surface `app_crashed` / `app_not_running` with a pointer to
//! DiagnosticReports. Do **not** invent Swift root causes; attach enough signal
//! that the agent opens `.ips` / symbols on purpose.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::control::FaultClass;

/// Freshness window for linking a DiagnosticReports crash to the current session.
pub const CRASH_RECENT_SECS: u64 = 180;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProcessHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// Guest process present in `launchctl list` for this bundle.
    #[serde(default)]
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    /// A matching `.ips` exists within [`CRASH_RECENT_SECS`].
    #[serde(default)]
    pub crashed_recently: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_report_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_exception: Option<String>,
    /// One-line agent hint (never a root-cause claim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ProcessHealth {
    pub fn classify_fault(&self) -> Option<FaultClass> {
        if self.running {
            return None;
        }
        if self.crashed_recently {
            return Some(FaultClass::AppCrashed);
        }
        // Only assert not-running when we actually looked up a bundle.
        if self.bundle_id.is_some() {
            return Some(FaultClass::AppNotRunning);
        }
        None
    }
}

/// Map process health → fault for discover / app_ready (never `discover_no_chrome`).
pub fn fault_from_process_health(health: &ProcessHealth) -> Option<FaultClass> {
    health.classify_fault()
}

/// Probe guest launchctl + recent DiagnosticReports for `bundle_id` / `app_label`.
pub fn probe_process_health(
    udid: &str,
    bundle_id: Option<&str>,
    app_label: Option<&str>,
) -> ProcessHealth {
    let Some(bid) = bundle_id.filter(|s| !s.is_empty()) else {
        return ProcessHealth::default();
    };
    let (running, pid) = guest_app_running(udid, bid);
    let label = app_label
        .filter(|s| !s.is_empty() && s.to_ascii_lowercase() != "app")
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // org.joinmastodon.app → try "Mastodon" from penultimate; else last.
            let parts: Vec<&str> = bid.split('.').collect();
            if parts.len() >= 2 {
                let penultimate = parts[parts.len() - 2];
                if penultimate != "joinmastodon" && penultimate.len() > 2 {
                    // capitalize-ish: joinmastodon stays special-cased below
                    let mut c = penultimate.chars();
                    match c.next() {
                        Some(f) => format!("{}{}", f.to_uppercase(), c.as_str()),
                        None => penultimate.to_string(),
                    }
                } else if bid.contains("mastodon") {
                    "Mastodon".into()
                } else {
                    parts.last().unwrap_or(&bid).to_string()
                }
            } else {
                bid.to_string()
            }
        });
    let crash = recent_crash_report(&label, bid, CRASH_RECENT_SECS);
    let mut health = ProcessHealth {
        bundle_id: Some(bid.to_string()),
        running,
        pid,
        crashed_recently: crash.is_some(),
        crash_report_path: crash.as_ref().map(|c| c.path.display().to_string()),
        crash_signal: crash.as_ref().and_then(|c| c.signal.clone()),
        crash_exception: crash.as_ref().and_then(|c| c.exception.clone()),
        hint: None,
    };
    if !running && health.crashed_recently {
        health.hint = Some(format!(
            "app_crashed: process not in launchctl; open {} (atos / symbols) — not discover_no_chrome",
            health
                .crash_report_path
                .as_deref()
                .unwrap_or("DiagnosticReports")
        ));
    } else if !running {
        health.hint = Some(
            "app_not_running: expected bundle absent from sim launchctl — relaunch before discover"
                .into(),
        );
    }
    health
}

fn guest_app_running(udid: &str, bundle_id: &str) -> (bool, Option<i32>) {
    let out = Command::new("xcrun")
        .args(["simctl", "spawn", udid, "launchctl", "list"])
        .output();
    let Ok(out) = out else {
        return (false, None);
    };
    if !out.status.success() {
        return (false, None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!("UIKitApplication:{bundle_id}");
    for line in text.lines() {
        if !line.contains(&needle) && !line.contains(bundle_id) {
            continue;
        }
        // Prefer UIKitApplication lines for the exact bid.
        if line.contains(&needle) || line.contains(bundle_id) {
            let pid = line
                .split_whitespace()
                .next()
                .and_then(|p| p.parse::<i32>().ok())
                .filter(|&p| p > 0);
            if pid.is_some() || line.contains(&needle) {
                return (pid.is_some(), pid);
            }
        }
    }
    (false, None)
}

#[derive(Debug)]
struct CrashHit {
    path: PathBuf,
    signal: Option<String>,
    exception: Option<String>,
}

fn recent_crash_report(app_label: &str, bundle_id: &str, within_secs: u64) -> Option<CrashHit> {
    let home = dirs_home()?;
    let dir = home.join("Library/Logs/DiagnosticReports");
    if !dir.is_dir() {
        return None;
    }
    let prefix = format!("{app_label}-");
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(within_secs))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let Ok(entries) = fs::read_dir(&dir) else {
        return None;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".ips") {
            continue;
        }
        let name_match = name.starts_with(&prefix)
            || name.to_ascii_lowercase().contains(&app_label.to_ascii_lowercase());
        let meta = ent.metadata().ok()?;
        let modified = meta.modified().ok()?;
        if modified < cutoff {
            continue;
        }
        if !name_match {
            // Fall back: IPS header may carry bundleID for this process.
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let header = raw.lines().next().unwrap_or("");
            if !header.contains(bundle_id) && !raw.contains(bundle_id) {
                continue;
            }
        }
        if best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            best = Some((modified, path));
        }
    }
    let (_, path) = best?;
    let (signal, exception) = parse_ips_signals(&path);
    Some(CrashHit {
        path,
        signal,
        exception,
    })
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn parse_ips_signals(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(raw) = fs::read_to_string(path) else {
        return (None, None);
    };
    // Modern IPS: header line + JSON body.
    let body = raw.split_once('\n').map(|(_, b)| b).unwrap_or(&raw);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return (None, None);
    };
    let signal = v
        .pointer("/exception/signal")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            v.pointer("/termination/indicator")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        });
    let exception = v
        .pointer("/exception/type")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    (signal, exception)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crashed_recently_is_app_crashed_not_discover() {
        let h = ProcessHealth {
            bundle_id: Some("org.joinmastodon.app".into()),
            running: false,
            crashed_recently: true,
            crash_report_path: Some("/tmp/Mastodon.ips".into()),
            hint: Some("app_crashed: ...".into()),
            ..Default::default()
        };
        assert_eq!(h.classify_fault(), Some(FaultClass::AppCrashed));
    }

    #[test]
    fn not_running_without_crash_is_app_not_running() {
        let h = ProcessHealth {
            bundle_id: Some("com.test".into()),
            running: false,
            crashed_recently: false,
            ..Default::default()
        };
        assert_eq!(h.classify_fault(), Some(FaultClass::AppNotRunning));
    }

    #[test]
    fn running_has_no_fault() {
        let h = ProcessHealth {
            bundle_id: Some("com.test".into()),
            running: true,
            pid: Some(42),
            ..Default::default()
        };
        assert_eq!(h.classify_fault(), None);
    }
}
