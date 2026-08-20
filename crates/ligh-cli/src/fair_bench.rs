//! Fair baselines that try to kill MCP-carp bias in `ligh bench agent`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ligh_core::SessionState;
use ligh_host::{AxDump, HidInput};

#[derive(Clone)]
struct Step {
    i: usize,
    op: String,
    ok: bool,
    ms: f64,
    detail: String,
}

fn median(xs: &mut [f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs[xs.len() / 2]
}

fn p95(xs: &mut [f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((xs.len() as f64 - 1.0) * 0.95).round() as usize;
    xs[idx.min(xs.len() - 1)]
}

fn stats(samples: &mut [f64]) -> serde_json::Value {
    if samples.is_empty() {
        return serde_json::json!({ "n": 0, "p50_ms": null, "p95_ms": null });
    }
    let mut copy = samples.to_vec();
    serde_json::json!({
        "n": samples.len(),
        "p50_ms": median(samples),
        "p95_ms": p95(&mut copy),
    })
}

struct DirectHost {
    udid: String,
    w: f64,
    h: f64,
}

impl DirectHost {
    fn new(session: &SessionState) -> Self {
        let (w, h) = session.device.logical_size();
        Self {
            udid: session.udid.clone(),
            w,
            h,
        }
    }

    fn home(&self) -> anyhow::Result<()> {
        HidInput::home(&self.udid).map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn exists(&self, label: &str) -> bool {
        AxDump::exists_label(&self.udid, label).unwrap_or(false)
    }
    fn wait(&self, label: &str, timeout_ms: u64) -> anyhow::Result<()> {
        AxDump::wait_label(&self.udid, label, Duration::from_millis(timeout_ms))
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn tap_label(&self, label: &str, timeout_ms: u64) -> anyhow::Result<()> {
        let (x, y, _) = AxDump::wait_label(&self.udid, label, Duration::from_millis(timeout_ms))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        HidInput::tap(&self.udid, x, y, self.w, self.h).map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn tap_norm(&self, x: f64, y: f64) -> anyhow::Result<()> {
        HidInput::tap(&self.udid, x, y, self.w, self.h).map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn type_text(&self, text: &str) -> anyhow::Result<()> {
        HidInput::type_text(&self.udid, text).map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn dump(&self) -> anyhow::Result<(serde_json::Value, usize)> {
        let d = AxDump::dump(&self.udid).map_err(|e| anyhow::anyhow!("{e}"))?;
        let n = serde_json::to_vec(&d).map(|b| b.len()).unwrap_or(0);
        Ok((d, n))
    }
    fn shot_simctl(&self, path: &str) -> anyhow::Result<()> {
        let o = std::process::Command::new("xcrun")
            .args(["simctl", "io", &self.udid, "screenshot", path])
            .output()?;
        if o.status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("{}", String::from_utf8_lossy(&o.stderr)))
        }
    }
}

fn dismiss(host: &DirectHost) {
    for label in ["Annulla", "Cancel", "Cancella testo", "Clear text"] {
        if host.exists(label) {
            let _ = host.tap_label(label, 1500);
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn ensure_root(host: &DirectHost, general: &str) -> Result<(), String> {
    dismiss(host);
    for attempt in 0..8 {
        if host.exists(general)
            || (host.exists("Bluetooth")
                && (host.exists("Wi-Fi") || host.exists("Wi‑Fi") || host.exists("WLAN")))
        {
            return Ok(());
        }
        let in_prefs = host.exists("Bluetooth")
            || host.exists(general)
            || host.exists("Wi-Fi")
            || host.exists("Wi‑Fi");
        if attempt >= 2 && !in_prefs {
            return Err("not in Preferences".into());
        }
        let _ = host.tap_norm(0.11, 0.09);
        std::thread::sleep(Duration::from_millis(220));
        dismiss(host);
    }
    Err(format!("missing {general}"))
}

fn open_settings(host: &DirectHost, settings: &str, general: &str) -> (bool, String) {
    if host.exists(settings) {
        let _ = host.tap_label(settings, 2200);
        std::thread::sleep(Duration::from_millis(220));
        dismiss(host);
        if ensure_root(host, general).is_ok() {
            return (true, "hid_tap_label".into());
        }
    }
    let udid = &host.udid;
    let _ = std::process::Command::new("xcrun")
        .args(["simctl", "terminate", udid, "com.apple.Preferences"])
        .output();
    std::thread::sleep(Duration::from_millis(150));
    let ok = std::process::Command::new("xcrun")
        .args(["simctl", "launch", udid, "com.apple.Preferences"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    std::thread::sleep(Duration::from_millis(500));
    dismiss(host);
    if ensure_root(host, general).is_ok() {
        return (
            true,
            if ok {
                "simctl_launch_fallback".into()
            } else {
                "simctl_launch_failed_then_root".into()
            },
        );
    }
    (false, "open_settings_failed".into())
}

fn labels(host: &DirectHost) -> (String, String, String, String) {
    if host.exists("Settings") {
        (
            "Settings".into(),
            "General".into(),
            "Search".into(),
            "Safari".into(),
        )
    } else {
        (
            "Impostazioni".into(),
            "Generali".into(),
            "Cerca".into(),
            "Safari".into(),
        )
    }
}

fn finish(
    name: &str,
    path: &str,
    note: &str,
    steps: &[Step],
    by_class: BTreeMap<String, Vec<f64>>,
    hard_fail: Option<String>,
    wall_ms: f64,
    cycle: u32,
    target_steps: usize,
) -> serde_json::Value {
    let passed = steps.iter().filter(|s| s.ok).count();
    let failed = steps.iter().filter(|s| !s.ok).count();
    let mut per_op = serde_json::Map::new();
    for (k, mut v) in by_class {
        per_op.insert(k, stats(&mut v));
    }
    serde_json::json!({
        "name": name,
        "path": path,
        "ok": hard_fail.is_none() && failed == 0,
        "hard_fail": hard_fail,
        "wall_ms": wall_ms,
        "target_steps": target_steps,
        "steps_run": steps.len(),
        "cycles": cycle,
        "passed": passed,
        "failed": failed,
        "failure_rate": if steps.is_empty() { 1.0 } else { failed as f64 / steps.len() as f64 },
        "per_op": per_op,
        "steps": steps.iter().map(|s| serde_json::json!({
            "i": s.i, "op": s.op, "ok": s.ok, "ms": s.ms, "detail": s.detail
        })).collect::<Vec<_>>(),
        "note": note,
    })
}

/// Same script as warm LIGHd, but one process / direct host APIs (no Unix socket).
pub fn run_workload_mono(
    session: &SessionState,
    target_steps: usize,
) -> anyhow::Result<serde_json::Value> {
    let host = DirectHost::new(session);
    let (settings, general, search, safari) = labels(&host);
    let mut steps = Vec::new();
    let mut by_class: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut hard_fail = None;
    let mut cycle = 0u32;
    let push = |steps: &mut Vec<Step>,
                by_class: &mut BTreeMap<String, Vec<f64>>,
                op: &str,
                ok: bool,
                ms: f64,
                detail: String| {
        by_class.entry(op.to_string()).or_default().push(ms);
        steps.push(Step {
            i: steps.len() + 1,
            op: op.to_string(),
            ok,
            ms,
            detail,
        });
    };
    let wall0 = Instant::now();
    while steps.len() < target_steps && hard_fail.is_none() {
        cycle += 1;
        let t = Instant::now();
        let r = host.home();
        push(
            &mut steps,
            &mut by_class,
            "home",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            format!("cycle={cycle}"),
        );
        if r.is_err() {
            hard_fail = Some("home".into());
            break;
        }
        let _ = host.home();
        std::thread::sleep(Duration::from_millis(200));

        let t = Instant::now();
        let ok = host.exists(&safari)
            || host.exists(&settings)
            || host.wait(&settings, 2500).is_ok();
        push(
            &mut steps,
            &mut by_class,
            "wait",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            format!("springboard:{safari}|{settings}"),
        );
        if !ok && cycle > 1 {
            hard_fail = Some("springboard".into());
            break;
        }

        let t = Instant::now();
        match host.dump() {
            Ok((_, n)) => push(
                &mut steps,
                &mut by_class,
                "observe",
                true,
                t.elapsed().as_secs_f64() * 1000.0,
                format!("{n}B"),
            ),
            Err(e) => {
                push(
                    &mut steps,
                    &mut by_class,
                    "observe",
                    false,
                    t.elapsed().as_secs_f64() * 1000.0,
                    e.to_string(),
                );
                hard_fail = Some("observe".into());
                break;
            }
        }
        if steps.len() >= target_steps {
            break;
        }

        let t = Instant::now();
        let r = host.wait(&settings, 4000);
        push(
            &mut steps,
            &mut by_class,
            "wait",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            settings.clone(),
        );
        if r.is_err() {
            hard_fail = Some(format!("wait {settings}"));
            break;
        }

        let t = Instant::now();
        let (ok, detail) = open_settings(&host, &settings, &general);
        push(
            &mut steps,
            &mut by_class,
            "tap_label",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            detail,
        );
        if !ok {
            hard_fail = Some("open Settings".into());
            break;
        }

        let t = Instant::now();
        let ok = host.exists(&general) || host.exists("Bluetooth");
        push(
            &mut steps,
            &mut by_class,
            "wait",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            format!("exists_root:{general}"),
        );
        if !ok {
            hard_fail = Some("settings root".into());
            break;
        }

        let t = Instant::now();
        if let Ok((_, n)) = host.dump() {
            push(
                &mut steps,
                &mut by_class,
                "observe",
                true,
                t.elapsed().as_secs_f64() * 1000.0,
                format!("{n}B"),
            );
        }

        let t = Instant::now();
        let r = host.tap_label(&search, 4000);
        push(
            &mut steps,
            &mut by_class,
            "tap_label",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            search.clone(),
        );
        if r.is_err() {
            hard_fail = Some(format!("tap {search}"));
            break;
        }
        std::thread::sleep(Duration::from_millis(120));
        let _ = host.tap_label(&search, 1000);
        std::thread::sleep(Duration::from_millis(80));

        let t = Instant::now();
        let r = host.type_text("ligh");
        push(
            &mut steps,
            &mut by_class,
            "type",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            "ligh".into(),
        );
        if r.is_err() {
            hard_fail = Some("type".into());
            break;
        }
        std::thread::sleep(Duration::from_millis(140));

        let t = Instant::now();
        let ok = host.exists("Cancella testo")
            || host.exists("Clear text")
            || host
                .dump()
                .ok()
                .map(|(v, _)| v.to_string().to_ascii_lowercase().contains("ligh"))
                .unwrap_or(false);
        push(
            &mut steps,
            &mut by_class,
            "assert",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            "field_or_clear".into(),
        );
        if !ok {
            hard_fail = Some("assert".into());
            break;
        }
        dismiss(&host);

        let path = format!("/tmp/ligh-mono-{}-{cycle}.png", host.udid);
        let t = Instant::now();
        let r = host.shot_simctl(&path);
        push(
            &mut steps,
            &mut by_class,
            "screenshot",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            path,
        );
    }
    Ok(finish(
        "LIGH mono — same host APIs, one process, no lighd socket",
        "ligh_mono_direct",
        "Kills 'daemon magic' claim if ≈ LIGHd wall time.",
        &steps,
        by_class,
        hard_fail,
        wall0.elapsed().as_secs_f64() * 1000.0,
        cycle,
        target_steps,
    ))
}

/// Optimized non-carp photo path: simctl screenshot for observe, in-process AX for act/wait.
pub fn run_workload_opt_photo(
    session: &SessionState,
    target_steps: usize,
) -> anyhow::Result<serde_json::Value> {
    let host = DirectHost::new(session);
    let (settings, general, search, safari) = labels(&host);
    let mut steps = Vec::new();
    let mut by_class: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut hard_fail = None;
    let mut cycle = 0u32;
    let push = |steps: &mut Vec<Step>,
                by_class: &mut BTreeMap<String, Vec<f64>>,
                op: &str,
                ok: bool,
                ms: f64,
                detail: String| {
        by_class.entry(op.to_string()).or_default().push(ms);
        steps.push(Step {
            i: steps.len() + 1,
            op: op.to_string(),
            ok,
            ms,
            detail,
        });
    };
    let wall0 = Instant::now();
    while steps.len() < target_steps && hard_fail.is_none() {
        cycle += 1;
        let t = Instant::now();
        let r = host.home();
        push(
            &mut steps,
            &mut by_class,
            "home",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            format!("cycle={cycle}"),
        );
        if r.is_err() {
            hard_fail = Some("home".into());
            break;
        }
        let _ = host.home();
        std::thread::sleep(Duration::from_millis(200));

        let t = Instant::now();
        let ok = host.exists(&safari) || host.exists(&settings) || host.wait(&settings, 2500).is_ok();
        push(
            &mut steps,
            &mut by_class,
            "wait",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            "springboard".into(),
        );

        // Observe = simctl photo (optimized: single process, but still file screenshot).
        let path = format!("/tmp/ligh-opt-{}-{cycle}a.png", host.udid);
        let t = Instant::now();
        let r = host.shot_simctl(&path);
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        push(
            &mut steps,
            &mut by_class,
            "observe",
            r.is_ok() && bytes > 0,
            t.elapsed().as_secs_f64() * 1000.0,
            format!("simctl_photo {bytes}B"),
        );
        if r.is_err() {
            hard_fail = Some("photo observe".into());
            break;
        }
        if steps.len() >= target_steps {
            break;
        }

        let t = Instant::now();
        let r = host.wait(&settings, 4000);
        push(
            &mut steps,
            &mut by_class,
            "wait",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            settings.clone(),
        );
        if r.is_err() {
            hard_fail = Some(format!("wait {settings}"));
            break;
        }

        let t = Instant::now();
        let (ok, detail) = open_settings(&host, &settings, &general);
        push(
            &mut steps,
            &mut by_class,
            "tap_label",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            detail,
        );
        if !ok {
            hard_fail = Some("open Settings".into());
            break;
        }

        let t = Instant::now();
        let ok = host.exists(&general) || host.exists("Bluetooth");
        push(
            &mut steps,
            &mut by_class,
            "wait",
            ok,
            t.elapsed().as_secs_f64() * 1000.0,
            format!("exists_root:{general}"),
        );

        let path = format!("/tmp/ligh-opt-{}-{cycle}b.png", host.udid);
        let t = Instant::now();
        let r = host.shot_simctl(&path);
        push(
            &mut steps,
            &mut by_class,
            "observe",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            "simctl_photo".into(),
        );

        let t = Instant::now();
        let r = host.tap_label(&search, 4000);
        push(
            &mut steps,
            &mut by_class,
            "tap_label",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            search.clone(),
        );
        if r.is_err() {
            hard_fail = Some(format!("tap {search}"));
            break;
        }
        std::thread::sleep(Duration::from_millis(120));
        let _ = host.tap_label(&search, 1000);

        let t = Instant::now();
        let r = host.type_text("ligh");
        push(
            &mut steps,
            &mut by_class,
            "type",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            "ligh".into(),
        );
        std::thread::sleep(Duration::from_millis(140));

        let path = format!("/tmp/ligh-opt-{}-{cycle}c.png", host.udid);
        let t = Instant::now();
        let r = host.shot_simctl(&path);
        push(
            &mut steps,
            &mut by_class,
            "assert",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            "simctl_photo after type (no structured field)".into(),
        );

        let path = format!("/tmp/ligh-opt-{}-{cycle}.png", host.udid);
        let t = Instant::now();
        let r = host.shot_simctl(&path);
        push(
            &mut steps,
            &mut by_class,
            "screenshot",
            r.is_ok(),
            t.elapsed().as_secs_f64() * 1000.0,
            path,
        );
    }
    Ok(finish(
        "Optimized photo path — simctl screenshot observe + in-process AX act (no process-per-poll)",
        "opt_simctl_photo_plus_ax",
        "Fairer than MCP carp: isolates screenshot-file tax without spawn-per-tool.",
        &steps,
        by_class,
        hard_fail,
        wall0.elapsed().as_secs_f64() * 1000.0,
        cycle,
        target_steps,
    ))
}

/// Run scripts/wda-agent-bench.py against a listening Appium server.
pub fn run_workload_wda(udid: &str, target_steps: usize) -> serde_json::Value {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/wda-agent-bench.py");
    if !script.exists() {
        return serde_json::json!({
            "ok": false,
            "path": "appium_xcuitest_wda",
            "skipped": true,
            "error": "scripts/wda-agent-bench.py missing",
        });
    }
    let listening = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:4723".parse().unwrap(),
        Duration::from_millis(300),
    )
    .is_ok();
    if !listening {
        return serde_json::json!({
            "ok": false,
            "path": "appium_xcuitest_wda",
            "skipped": true,
            "status": "--",
            "error": "Appium not listening on 127.0.0.1:4723 — start: APPIUM_HOME=.appium ./node_modules/.bin/appium",
        });
    }
    let t0 = Instant::now();
    match std::process::Command::new("python3")
        .arg(&script)
        .env("UDID", udid)
        .env("STEPS", target_steps.to_string())
        .env("APPIUM_URL", "http://127.0.0.1:4723")
        .output()
    {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                v
            } else {
                serde_json::json!({
                    "ok": false,
                    "path": "appium_xcuitest_wda",
                    "wall_ms": t0.elapsed().as_secs_f64() * 1000.0,
                    "error": format!("bad json stdout={} stderr={}", stdout.chars().take(200).collect::<String>(), stderr.chars().take(200).collect::<String>()),
                })
            }
        }
        Err(e) => serde_json::json!({
            "ok": false,
            "path": "appium_xcuitest_wda",
            "error": e.to_string(),
        }),
    }
}
