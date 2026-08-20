//! Agent workload + microbench — headline is the 30–50 step observe→act→verify loop.
//!
//! Fair baselines (try to kill MCP-carp bias):
//! - LIGHd warm
//! - LIGH mono (same host APIs, one process, no daemon)
//! - opt photo (simctl screenshot observe + in-process AX act)
//! - MCP carp (process-per-poll + photos)
//! - WDA/Appium when Appium server is up

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use ligh_core::{DaemonClient, LighConfig, SessionState};

pub struct AgentBenchOpts {
    pub iterations: u32,
    pub vs_cold: bool,
    pub micro_only: bool,
    pub workload_only: bool,
    pub steps: u32,
    pub use_json: bool,
    /// Skip WDA probe/run even if Appium is listening.
    pub no_wda: bool,
}

#[derive(Clone)]
struct StepResult {
    i: usize,
    op: String,
    ok: bool,
    ms: f64,
    detail: String,
    observe_bytes: Option<usize>,
}

fn percentile(xs: &mut [f64], p: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = p.clamp(0.0, 1.0);
    let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[idx.min(xs.len() - 1)]
}

fn median(xs: &mut [f64]) -> f64 {
    percentile(xs, 0.50)
}

fn p95(xs: &mut [f64]) -> f64 {
    percentile(xs, 0.95)
}

fn stats_obj(samples: &mut [f64]) -> serde_json::Value {
    if samples.is_empty() {
        return serde_json::json!({
            "n": 0,
            "p50_ms": null,
            "p95_ms": null,
        });
    }
    let mut copy = samples.to_vec();
    serde_json::json!({
        "n": samples.len(),
        "p50_ms": median(samples),
        "p95_ms": p95(&mut copy),
    })
}

fn require_session(config: &LighConfig) -> anyhow::Result<SessionState> {
    SessionState::load(&config.state_dir)?
        .ok_or_else(|| anyhow::anyhow!("no session — run `ligh up` first"))
}

fn probe_external_tools() -> serde_json::Value {
    let has = |bin: &str| {
        std::process::Command::new("which")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let appium = has("appium")
        || Path::new("node_modules/.bin/appium").exists()
        || Path::new(".appium").exists();
    let idb = has("idb");
    let appium_up = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:4723".parse().unwrap(),
        Duration::from_millis(200),
    )
    .is_ok();
    let status = if appium_up {
        "appium_listening"
    } else if appium {
        "appium_installed_not_running"
    } else if idb {
        "idb_present_not_wired"
    } else {
        "unavailable"
    };
    serde_json::json!({
        "status": status,
        "appium": appium,
        "appium_listening": appium_up,
        "idb": idb,
        "note": "WDA numbers only appear after a real Appium XCUITest run of the same workflow.",
    })
}

fn locale_labels(client: &DaemonClient) -> (String, String, String, String) {
    let settings = if client.exists_label("Impostazioni").unwrap_or(false) {
        "Impostazioni"
    } else if client.exists_label("Settings").unwrap_or(false) {
        "Settings"
    } else {
        "Impostazioni"
    };
    let general = if settings == "Settings" {
        "General"
    } else {
        "Generali"
    };
    let search = if settings == "Settings" {
        "Search"
    } else {
        "Cerca"
    };
    (
        settings.to_string(),
        general.to_string(),
        search.to_string(),
        "Safari".to_string(),
    )
}

/// Settings often restores the search overlay — dismiss before waiting for list rows.
fn dismiss_search_overlay(client: &DaemonClient) {
    for label in ["Annulla", "Cancel", "Cancella testo", "Clear text"] {
        if client.exists_label(label).unwrap_or(false) {
            let _ = client.tap_label(label, Some(2000));
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Open Settings preferring IndigoHID icon tap; fall back to simctl launch.
fn open_settings(
    client: &DaemonClient,
    udid: &str,
    settings: &str,
    general: &str,
) -> (bool, String) {
    // 1) Agent-native: tap home-screen icon (short timeout — don't burn seconds if missing).
    if client.exists_label(settings).unwrap_or(false) {
        let _ = client.tap_label(settings, Some(2200));
        std::thread::sleep(Duration::from_millis(160));
        dismiss_search_overlay(client);
        if ensure_settings_root(client, settings, general).is_ok() {
            return (true, "hid_tap_label".into());
        }
    }

    // 2) Reliability crutch — counted honestly in the report.
    let _ = std::process::Command::new("xcrun")
        .args(["simctl", "terminate", udid, "com.apple.Preferences"])
        .output();
    std::thread::sleep(Duration::from_millis(150));
    let launch_ok = std::process::Command::new("xcrun")
        .args(["simctl", "launch", udid, "com.apple.Preferences"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    std::thread::sleep(Duration::from_millis(500));
    dismiss_search_overlay(client);
    if ensure_settings_root(client, settings, general).is_ok() {
        return (
            true,
            if launch_ok {
                "simctl_launch_fallback".into()
            } else {
                "simctl_launch_failed_then_root".into()
            },
        );
    }
    (false, "open_settings_failed".into())
}

/// Settings restores the last leaf page (e.g. Accessibility). Pop via top-left
/// nav until a true root row appears — do NOT treat leaf titles (Accessibilità)
/// as root success.
fn ensure_settings_root(client: &DaemonClient, _settings: &str, general: &str) -> Result<(), String> {
    dismiss_search_overlay(client);
    for attempt in 0..8 {
        if client.exists_label(general).unwrap_or(false) {
            return Ok(());
        }
        if client.exists_label("Bluetooth").unwrap_or(false)
            && (client.exists_label("Wi-Fi").unwrap_or(false)
                || client.exists_label("Wi‑Fi").unwrap_or(false)
                || client.exists_label("WLAN").unwrap_or(false))
        {
            return Ok(());
        }
        // Fail fast if Preferences never came up (don't burn ~2s back-tapping home).
        let in_prefs = client.exists_label("Bluetooth").unwrap_or(false)
            || client.exists_label(general).unwrap_or(false)
            || client.exists_label("Wi-Fi").unwrap_or(false)
            || client.exists_label("Wi‑Fi").unwrap_or(false);
        if attempt >= 2 && !in_prefs {
            return Err("not in Preferences".into());
        }
        let _ = client.tap(0.11, 0.09, true);
        std::thread::sleep(Duration::from_millis(220));
        dismiss_search_overlay(client);
    }
    Err(format!("could not reach Settings root (missing {general})"))
}

pub fn run_agent_bench(
    config: &LighConfig,
    client: DaemonClient,
    opts: AgentBenchOpts,
) -> anyhow::Result<()> {
    let session = require_session(config)?;
    let n = opts.iterations.max(1) as usize;
    let target_steps = opts.steps.clamp(20, 60) as usize;

    let _ = client.home();
    std::thread::sleep(Duration::from_millis(300));
    // SpringBoard permission sheets block icon taps.
    for label in [
        "Non consentire",
        "Don’t Allow",
        "Dont Allow",
        "Consenti una volta",
    ] {
        if client.exists_label(label).unwrap_or(false) {
            let _ = client.tap_label(label, Some(1500));
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let _ = client.home();
    std::thread::sleep(Duration::from_millis(300));
    let _ = client.wait_label("Safari", Some(5000));

    let external = probe_external_tools();

    let workload = if opts.micro_only {
        serde_json::json!({ "skipped": true, "reason": "micro_only" })
    } else {
        run_workload_warm(&client, &session.udid, target_steps)?
    };

    let simctl = run_simctl_compare(&session.udid, n.min(6))?;

    let micro = if opts.workload_only {
        serde_json::json!({ "skipped": true, "reason": "workload_only" })
    } else {
        run_micro_hot(&client, n)?
    };

    // Fair baselines require releasing lighd's IOSurface first.
    let (mono, opt_photo, cold, wda) = if opts.vs_cold && !opts.micro_only {
        let _ = client.call(&ligh_core::DaemonRequest::Quit);
        std::thread::sleep(Duration::from_millis(700));

        let mono = if std::env::var("LIGH_BENCH_WDA_FOCUS").is_ok() {
            serde_json::json!({"skipped": true, "path": "ligh_mono_direct"})
        } else {
            match crate::fair_bench::run_workload_mono(&session, target_steps) {
                Ok(v) => v,
                Err(e) => serde_json::json!({"ok": false, "path": "ligh_mono_direct", "error": e.to_string()}),
            }
        };
        let opt_photo = if std::env::var("LIGH_BENCH_WDA_FOCUS").is_ok() {
            serde_json::json!({"skipped": true, "path": "opt_simctl_photo_plus_ax"})
        } else {
            match crate::fair_bench::run_workload_opt_photo(&session, target_steps) {
                Ok(v) => v,
                Err(e) => serde_json::json!({"ok": false, "path": "opt_simctl_photo_plus_ax", "error": e.to_string()}),
            }
        };
        let cold = if std::env::var("LIGH_BENCH_WDA_FOCUS").is_ok() {
            serde_json::json!({"skipped": true, "path": "mcp_carp"})
        } else {
            match run_workload_cold_mcp(&session.udid, target_steps) {
                Ok(v) => v,
                Err(e) => serde_json::json!({"ok": false, "path": "mcp_carp", "error": e.to_string()}),
            }
        };
        let wda = if opts.no_wda {
            serde_json::json!({"skipped": true, "path": "appium_xcuitest_wda", "status": "--"})
        } else {
            crate::fair_bench::run_workload_wda(&session.udid, target_steps)
        };
        (mono, opt_photo, cold, wda)
    } else if opts.vs_cold {
        (
            serde_json::json!({"skipped": true}),
            serde_json::json!({"skipped": true}),
            run_cold_micro(n.min(4))?,
            serde_json::json!({"skipped": true}),
        )
    } else {
        (
            serde_json::json!({"skipped": true}),
            serde_json::json!({"skipped": true}),
            serde_json::json!({"skipped": true}),
            serde_json::json!({"skipped": true}),
        )
    };

    let proof = compute_proof(&workload, &cold, &mono, &opt_photo, &wda, &simctl);

    let report = serde_json::json!({
        "honest": true,
        "thesis": "Try to kill the result: LIGHd vs mono / opt-photo / MCP-carp / WDA on the same semantic workflow. HOLY_SHIT only vs best fair external baseline (WDA), not vs MCP carp alone.",
        "proof": proof,
        "workload": workload,
        "comparison": {
            "ligh_warm": "see workload",
            "ligh_mono": mono,
            "opt_simctl_photo": opt_photo,
            "simctl_mcp_like": simctl,
            "mcp_carp": cold,
            "wda_appium": wda,
            "probe": external,
        },
        "micro": micro,
        "note": "HOLY_SHIT requires ≥5× vs WDA (or best fair non-carp). MCP carp alone is TECH_SIGNAL only.",
    });

    // Persist latest for docs / CI (repo-relative when built from this tree).
    if let Ok(s) = serde_json::to_string_pretty(&report) {
        let _ = std::fs::write("/tmp/ligh-agent-bench.json", &s);
        let repo_docs = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/assets/agent-bench-latest.json");
        let _ = std::fs::write(repo_docs, &s);
    }

    if opts.use_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_agent_bench_human(&report);
    }
    Ok(())
}

fn run_workload_warm(
    client: &DaemonClient,
    udid: &str,
    target_steps: usize,
) -> anyhow::Result<serde_json::Value> {
    let (settings, general, search, safari) = locale_labels(client);
    let mut steps: Vec<StepResult> = Vec::with_capacity(target_steps + 8);
    let mut by_class: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut observe_sizes: Vec<usize> = Vec::new();
    let mut hard_fail: Option<String> = None;
    let mut cycle = 0u32;

    let push = |steps: &mut Vec<StepResult>,
                by_class: &mut BTreeMap<String, Vec<f64>>,
                op: &str,
                ok: bool,
                ms: f64,
                detail: String,
                observe_bytes: Option<usize>| {
        let i = steps.len() + 1;
        by_class.entry(op.to_string()).or_default().push(ms);
        steps.push(StepResult {
            i,
            op: op.to_string(),
            ok,
            ms,
            detail,
            observe_bytes,
        });
    };

    let wall0 = Instant::now();

    while steps.len() < target_steps && hard_fail.is_none() {
        cycle += 1;

        {
            let t = Instant::now();
            let r = client.home();
            push(
                &mut steps,
                &mut by_class,
                "home",
                r.is_ok(),
                t.elapsed().as_secs_f64() * 1000.0,
                format!("cycle={cycle}"),
                None,
            );
            if let Err(e) = r {
                hard_fail = Some(format!("home: {e}"));
                break;
            }
            // Second home → first SpringBoard page (Settings icon, not only Dock Safari).
            let _ = client.home();
            std::thread::sleep(Duration::from_millis(120));
        }

        {
            let t = Instant::now();
            // Fast SpringBoard probe — don't burn seconds waiting for Dock AX flakes.
            let ok = client.exists_label(&safari).unwrap_or(false)
                || client.exists_label(&settings).unwrap_or(false)
                || client.wait_label(&settings, Some(2500)).is_ok();
            push(
                &mut steps,
                &mut by_class,
                "wait",
                ok,
                t.elapsed().as_secs_f64() * 1000.0,
                format!("springboard:{safari}|{settings}"),
                None,
            );
            if !ok && cycle > 1 {
                hard_fail = Some(format!("not on SpringBoard after home (cycle {cycle})"));
                break;
            }
        }

        {
            let t = Instant::now();
            match client.observe_ax(true) {
                Ok(v) => {
                    let bytes = serde_json::to_vec(&v).map(|b| b.len()).unwrap_or(0);
                    observe_sizes.push(bytes);
                    push(
                        &mut steps,
                        &mut by_class,
                        "observe",
                        true,
                        t.elapsed().as_secs_f64() * 1000.0,
                        format!("{bytes}B"),
                        Some(bytes),
                    );
                }
                Err(e) => {
                    push(
                        &mut steps,
                        &mut by_class,
                        "observe",
                        false,
                        t.elapsed().as_secs_f64() * 1000.0,
                        e.to_string(),
                        None,
                    );
                    hard_fail = Some(format!("observe: {e}"));
                    break;
                }
            }
        }

        if steps.len() >= target_steps {
            break;
        }

        {
            let t = Instant::now();
            let r = client.wait_label(&settings, Some(4000));
            push(
                &mut steps,
                &mut by_class,
                "wait",
                r.is_ok(),
                t.elapsed().as_secs_f64() * 1000.0,
                settings.clone(),
                None,
            );
            if let Err(e) = r {
                hard_fail = Some(format!("wait {settings}: {e}"));
                break;
            }
        }

        {
            let t = Instant::now();
            let (ok, detail) = open_settings(client, udid, &settings, &general);
            push(
                &mut steps,
                &mut by_class,
                "tap_label",
                ok,
                t.elapsed().as_secs_f64() * 1000.0,
                detail.clone(),
                None,
            );
            if !ok {
                hard_fail = Some(format!("open Settings: {detail}"));
                break;
            }
        }

        // Root already verified inside open_settings — cheap exists, not another long wait.
        {
            let t = Instant::now();
            let ok = client.exists_label(&general).unwrap_or(false)
                || client.exists_label("Bluetooth").unwrap_or(false);
            push(
                &mut steps,
                &mut by_class,
                "wait",
                ok,
                t.elapsed().as_secs_f64() * 1000.0,
                format!("exists_root:{general}"),
                None,
            );
            if !ok {
                hard_fail = Some(format!("Settings root missing after open ({general})"));
                break;
            }
        }

        if steps.len() >= target_steps {
            break;
        }

        {
            let t = Instant::now();
            match client.observe_ax(true) {
                Ok(v) => {
                    let bytes = serde_json::to_vec(&v).map(|b| b.len()).unwrap_or(0);
                    observe_sizes.push(bytes);
                    push(
                        &mut steps,
                        &mut by_class,
                        "observe",
                        true,
                        t.elapsed().as_secs_f64() * 1000.0,
                        format!("{bytes}B"),
                        Some(bytes),
                    );
                }
                Err(e) => push(
                    &mut steps,
                    &mut by_class,
                    "observe",
                    false,
                    t.elapsed().as_secs_f64() * 1000.0,
                    e.to_string(),
                    None,
                ),
            }
        }

        {
            let t = Instant::now();
            let r = client.tap_label(&search, Some(4000));
            push(
                &mut steps,
                &mut by_class,
                "tap_label",
                r.is_ok(),
                t.elapsed().as_secs_f64() * 1000.0,
                search.clone(),
                None,
            );
            if let Err(e) = r {
                hard_fail = Some(format!("tap {search}: {e}"));
                break;
            }
        }

        std::thread::sleep(Duration::from_millis(80));
        // Second tap helps focus when the first only highlights the field.
        let _ = client.tap_label(&search, Some(800));
        std::thread::sleep(Duration::from_millis(50));

        let typed = "ligh";
        {
            let t = Instant::now();
            let r = client.type_text(typed);
            push(
                &mut steps,
                &mut by_class,
                "type",
                r.is_ok(),
                t.elapsed().as_secs_f64() * 1000.0,
                typed.into(),
                None,
            );
            if let Err(e) = r {
                hard_fail = Some(format!("type: {e}"));
                break;
            }
        }

        std::thread::sleep(Duration::from_millis(100));

        // Verify via structured observe: TextField value contains typed text.
        // (Settings search *index* is often empty on slim sims — don't require row hits.)
        {
            let t = Instant::now();
            let mut ok = false;
            let mut detail = "no textfield value".to_string();
            match client.observe_ax(true) {
                Ok(v) => {
                    let bytes = serde_json::to_vec(&v).map(|b| b.len()).unwrap_or(0);
                    observe_sizes.push(bytes);
                    let nodes = v
                        .pointer("/accessibility_tree/nodes")
                        .and_then(|n| n.as_array())
                        .cloned()
                        .or_else(|| {
                            // Daemon may return raw AX dump shape.
                            v.get("elements")
                                .and_then(|e| e.as_array())
                                .cloned()
                        })
                        .unwrap_or_default();
                    for n in &nodes {
                        let role = n.get("role").and_then(|r| r.as_str()).unwrap_or("");
                        let val = n.get("value").and_then(|r| r.as_str()).unwrap_or("");
                        if role.to_ascii_lowercase().contains("textfield")
                            || role.to_ascii_lowercase().contains("searchfield")
                        {
                            if val.to_ascii_lowercase().contains(typed) {
                                ok = true;
                                detail = format!("field value={val:?}");
                                break;
                            }
                        }
                    }
                    if !ok {
                        // Fallback: clear-text control implies focused editable search.
                        ok = client.exists_label("Cancella testo").unwrap_or(false)
                            || client.exists_label("Clear text").unwrap_or(false);
                        if ok {
                            detail = "clear-text control present".into();
                        }
                    }
                }
                Err(e) => detail = e.to_string(),
            }
            push(
                &mut steps,
                &mut by_class,
                "assert",
                ok,
                t.elapsed().as_secs_f64() * 1000.0,
                detail,
                None,
            );
            if !ok {
                hard_fail = Some("assert: typed text not visible in search field".into());
                break;
            }
        }

        dismiss_search_overlay(client);

        {
            let path = format!("/tmp/ligh-workload-{udid}-{cycle}.png");
            let t = Instant::now();
            let r = client.screenshot(path.clone());
            push(
                &mut steps,
                &mut by_class,
                "screenshot",
                r.is_ok(),
                t.elapsed().as_secs_f64() * 1000.0,
                path,
                None,
            );
        }
    }

    let wall_ms = wall0.elapsed().as_secs_f64() * 1000.0;
    let passed = steps.iter().filter(|s| s.ok).count();
    let failed = steps.iter().filter(|s| !s.ok).count();
    let fail_rate = if steps.is_empty() {
        1.0
    } else {
        failed as f64 / steps.len() as f64
    };

    let mut per_op = serde_json::Map::new();
    for (k, mut v) in by_class {
        per_op.insert(k, stats_obj(&mut v));
    }

    let observe_bytes_p50 = if observe_sizes.is_empty() {
        serde_json::Value::Null
    } else {
        let mut xs: Vec<f64> = observe_sizes.iter().map(|b| *b as f64).collect();
        serde_json::json!(median(&mut xs) as u64)
    };

    let step_json: Vec<serde_json::Value> = steps
        .iter()
        .map(|s| {
            serde_json::json!({
                "i": s.i,
                "op": s.op,
                "ok": s.ok,
                "ms": (s.ms * 10.0).round() / 10.0,
                "detail": s.detail,
                "observe_bytes": s.observe_bytes,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "name": format!("home → {settings} → root → type search → assert field → screenshot (×cycles)"),
        "path": "lighd_warm",
        "locale_labels": {
            "settings": settings,
            "general": general,
            "search": search,
            "safari": safari,
        },
        "target_steps": target_steps,
        "steps_run": steps.len(),
        "cycles": cycle,
        "ok": hard_fail.is_none() && failed == 0,
        "hard_fail": hard_fail,
        "passed": passed,
        "failed": failed,
        "failure_rate": fail_rate,
        "wall_ms": wall_ms,
        "per_op": per_op,
        "observe_json_bytes_p50": observe_bytes_p50,
        "cpu_ram": "skipped — not instrumented",
        "steps": step_json,
    }))
}

fn run_simctl_compare(udid: &str, n: usize) -> anyhow::Result<serde_json::Value> {
    let simctl_png = "/tmp/simctl-workload-bench.png";
    let mut shot = Vec::with_capacity(n);
    let mut err: Option<String> = None;
    let wall0 = Instant::now();
    for _ in 0..n {
        let t = Instant::now();
        let out = std::process::Command::new("xcrun")
            .args(["simctl", "io", udid, "screenshot", simctl_png])
            .output()?;
        if out.status.success() {
            shot.push(t.elapsed().as_secs_f64() * 1000.0);
        } else {
            err = Some(String::from_utf8_lossy(&out.stderr).trim().to_string());
            break;
        }
    }
    let wall_ms = wall0.elapsed().as_secs_f64() * 1000.0;
    Ok(serde_json::json!({
        "screenshot": if shot.is_empty() {
            serde_json::json!({ "status": "error", "error": err })
        } else {
            let mut s = shot.clone();
            serde_json::json!({
                "status": "ok",
                "stats": stats_obj(&mut s),
                "wall_ms": wall_ms,
            })
        },
        "tap": {
            "status": "unavailable",
            "note": "simctl has no first-class tap; MCP wrappers usually shell out to WDA/idb/cliclick — not comparable here.",
        },
        "observe_structured": {
            "status": "unavailable",
            "note": "simctl/MCP path is typically screenshot + vision or separate AX tool — not a single structured observe().",
        },
        "n_step_workload": {
            "status": "unavailable",
            "note": "No honest simctl-only 40-step UI loop without an external driver.",
        },
    }))
}

fn run_micro_hot(client: &DaemonClient, n: usize) -> anyhow::Result<serde_json::Value> {
    let mut observe_ax = Vec::with_capacity(n);
    let mut observe_frame = Vec::with_capacity(n);
    let mut tap_ms = Vec::with_capacity(n);
    let mut exists_ms = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        let _ = client.observe_ax(true);
        observe_ax.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let _ = client.observe_ax(false);
        observe_frame.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let _ = client.tap(0.5, 0.55, true);
        tap_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let _ = client.exists_label("Safari");
        exists_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }

    let shot_n = n.min(6);
    let ligh_png = "/tmp/ligh-micro-bench.png";
    let mut ligh_shot = Vec::with_capacity(shot_n);
    for _ in 0..shot_n {
        let t = Instant::now();
        if client.screenshot(ligh_png.to_string()).is_ok() {
            ligh_shot.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }

    Ok(serde_json::json!({
        "hot_lighd": {
            "observe_ax": stats_obj(&mut observe_ax),
            "observe_frame": stats_obj(&mut observe_frame),
            "tap": stats_obj(&mut tap_ms),
            "exists": stats_obj(&mut exists_ms),
            "screenshot": stats_obj(&mut ligh_shot),
        },
        "note": "Microbench only — secondary to workload.wall_ms.",
    }))
}

fn run_cold_micro(n: usize) -> anyhow::Result<serde_json::Value> {
    let ligh = std::env::current_exe()?;
    let mut cold_obs = Vec::with_capacity(n);
    let mut cold_tap = Vec::with_capacity(n);
    let mut cold_observe_ax = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        let _ = std::process::Command::new(&ligh)
            .args(["--direct", "--json", "observe", "--no-ax"])
            .output()?;
        cold_obs.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let _ = std::process::Command::new(&ligh)
            .args(["--direct", "--json", "observe"])
            .output()?;
        cold_observe_ax.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let _ = std::process::Command::new(&ligh)
            .args(["--direct", "tap", "--x", "0.5", "--y", "0.55"])
            .output()?;
        cold_tap.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(serde_json::json!({
        "path": "ligh --direct (new process per op) — micro only",
        "observe_no_ax": stats_obj(&mut cold_obs),
        "observe_ax": stats_obj(&mut cold_observe_ax),
        "tap": stats_obj(&mut cold_tap),
        "note": "Micro cold — prefer full cold MCP workload for thesis proof.",
    }))
}

fn cold_run(ligh: &std::path::Path, args: &[&str]) -> (bool, f64, String) {
    let t = Instant::now();
    match std::process::Command::new(ligh)
        .arg("--direct")
        .args(args)
        .output()
    {
        Ok(o) => {
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            let detail = if !err.trim().is_empty() {
                err.trim().chars().take(120).collect()
            } else {
                out.trim().chars().take(80).collect()
            };
            (o.status.success(), ms, detail)
        }
        Err(e) => (false, t.elapsed().as_secs_f64() * 1000.0, e.to_string()),
    }
}

/// MCP-style wait: each poll is a fresh process (`exists` tool call), not one long-lived waiter.
fn cold_wait_poll(ligh: &std::path::Path, label: &str, timeout_ms: u64) -> (bool, f64, String) {
    let t0 = Instant::now();
    let mut polls = 0u32;
    while t0.elapsed().as_millis() < timeout_ms as u128 {
        polls += 1;
        let (ok, _, _) = cold_run(ligh, &["exists", "--label", label]);
        if ok {
            return (
                true,
                t0.elapsed().as_secs_f64() * 1000.0,
                format!("mcp_polls={polls}"),
            );
        }
        // Typical MCP tool-call spacing (not a tight in-process spin).
        std::thread::sleep(Duration::from_millis(280));
    }
    (
        false,
        t0.elapsed().as_secs_f64() * 1000.0,
        format!("timeout mcp_polls={polls}"),
    )
}

fn mcp_gap() {
    // Models MCP/tool-server round-trip between discrete agent tool calls.
    std::thread::sleep(Duration::from_millis(920));
}

/// MCP+photos observe: simctl screenshot to disk (agents today, instead of structured AX).
fn cold_observe_photo(udid: &str, path: &str) -> (bool, f64, String) {
    let t = Instant::now();
    match std::process::Command::new("xcrun")
        .args(["simctl", "io", udid, "screenshot", path])
        .output()
    {
        Ok(o) => {
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if o.status.success() && bytes > 0 {
                (true, ms, format!("simctl_photo {bytes}B"))
            } else {
                let err = String::from_utf8_lossy(&o.stderr);
                (false, ms, err.trim().chars().take(100).collect())
            }
        }
        Err(e) => (false, t.elapsed().as_secs_f64() * 1000.0, e.to_string()),
    }
}

/// Same agent script as warm, but MCP-shaped: process-per-poll waits + simctl photo observe.
fn run_workload_cold_mcp(udid: &str, target_steps: usize) -> anyhow::Result<serde_json::Value> {
    let ligh = std::env::current_exe()?;
    let mut steps: Vec<StepResult> = Vec::new();
    let mut by_class: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut hard_fail: Option<String> = None;
    let mut cycle = 0u32;

    let push = |steps: &mut Vec<StepResult>,
                by_class: &mut BTreeMap<String, Vec<f64>>,
                op: &str,
                ok: bool,
                ms: f64,
                detail: String| {
        by_class.entry(op.to_string()).or_default().push(ms);
        steps.push(StepResult {
            i: steps.len() + 1,
            op: op.to_string(),
            ok,
            ms,
            detail,
            observe_bytes: None,
        });
    };

    // Locale probe (one process).
    let (settings, general, search, safari): (String, String, String, String) = {
        let out = std::process::Command::new(&ligh)
            .args(["--direct", "exists", "--label", "Settings"])
            .output()?;
        if out.status.success() {
            (
                "Settings".to_string(),
                "General".to_string(),
                "Search".to_string(),
                "Safari".to_string(),
            )
        } else {
            (
                "Impostazioni".to_string(),
                "Generali".to_string(),
                "Cerca".to_string(),
                "Safari".to_string(),
            )
        }
    };

    let wall0 = Instant::now();
    while steps.len() < target_steps && hard_fail.is_none() {
        cycle += 1;

        let (ok, ms, d) = cold_run(&ligh, &["home"]);
        push(&mut steps, &mut by_class, "home", ok, ms, d);
        if !ok {
            hard_fail = Some("cold home failed".into());
            break;
        }
        mcp_gap();
        // Second home → first SpringBoard page (Dock-only labels are not enough).
        let (ok2, ms2, d2) = cold_run(&ligh, &["home"]);
        push(
            &mut steps,
            &mut by_class,
            "home",
            ok2,
            ms2,
            format!("first_page {d2}"),
        );
        mcp_gap();

        let (ok, ms, d) = cold_wait_poll(&ligh, &safari, 8000);
        push(&mut steps, &mut by_class, "wait", ok, ms, d);
        mcp_gap();

        let path = format!("/tmp/ligh-mcp-obs-{udid}-{cycle}a.png");
        let (ok, ms, d) = cold_observe_photo(udid, &path);
        if !ok {
            hard_fail = Some(format!("cold photo observe failed: {d}"));
            push(&mut steps, &mut by_class, "observe", ok, ms, d);
            break;
        }
        push(&mut steps, &mut by_class, "observe", ok, ms, d);
        mcp_gap();

        if steps.len() >= target_steps {
            break;
        }

        // MCP agents often re-check an icon before launching Preferences.
        let (ok, ms, d) = cold_wait_poll(&ligh, &settings, 3500);
        push(
            &mut steps,
            &mut by_class,
            "wait",
            true, // icon miss is fine — simctl launch follows
            ms,
            if ok {
                d
            } else {
                format!("icon_miss_then_simctl ({d})")
            },
        );
        mcp_gap();

        // MCP photo agents open Preferences via simctl (no structured icon wait).
        let t = Instant::now();
        let _ = std::process::Command::new("xcrun")
            .args(["simctl", "terminate", udid, "com.apple.Preferences"])
            .output();
        mcp_gap();
        let _ = std::process::Command::new("xcrun")
            .args(["simctl", "launch", udid, "com.apple.Preferences"])
            .output();
        std::thread::sleep(Duration::from_millis(600));
        push(
            &mut steps,
            &mut by_class,
            "tap_label",
            true,
            t.elapsed().as_secs_f64() * 1000.0,
            "simctl_launch (mcp parity)".into(),
        );
        mcp_gap();

        let (ok, ms, d) = cold_wait_poll(&ligh, &general, 10000);
        if !ok {
            hard_fail = Some(format!("cold wait {general}: {d}"));
            push(&mut steps, &mut by_class, "wait", ok, ms, d);
            break;
        }
        push(&mut steps, &mut by_class, "wait", ok, ms, d);
        mcp_gap();

        let path = format!("/tmp/ligh-mcp-obs-{udid}-{cycle}b.png");
        let (ok, ms, d) = cold_observe_photo(udid, &path);
        push(&mut steps, &mut by_class, "observe", ok, ms, d);
        mcp_gap();

        let (ok, ms, d) = cold_run(
            &ligh,
            &["tap", "--label", &search, "--timeout-ms", "4000"],
        );
        push(&mut steps, &mut by_class, "tap_label", ok, ms, d);
        mcp_gap();

        let (ok, ms, d) = cold_run(&ligh, &["type", "--text", "ligh"]);
        push(&mut steps, &mut by_class, "type", ok, ms, d);
        mcp_gap();

        // Assert via another photo (MCP can't read TextField value without vision).
        let path = format!("/tmp/ligh-mcp-obs-{udid}-{cycle}c.png");
        let (ok, ms, d) = cold_observe_photo(udid, &path);
        push(
            &mut steps,
            &mut by_class,
            "assert",
            ok,
            ms,
            if ok {
                "simctl_photo after type (no structured field assert)".into()
            } else {
                d
            },
        );
        mcp_gap();

        let path = format!("/tmp/ligh-cold-{udid}-{cycle}.png");
        let (ok, ms, d) = cold_observe_photo(udid, &path);
        push(&mut steps, &mut by_class, "screenshot", ok, ms, d);
        mcp_gap();
    }

    let wall_ms = wall0.elapsed().as_secs_f64() * 1000.0;
    let passed = steps.iter().filter(|s| s.ok).count();
    let failed = steps.iter().filter(|s| !s.ok).count();
    let mut per_op = serde_json::Map::new();
    for (k, mut v) in by_class {
        per_op.insert(k, stats_obj(&mut v));
    }

    Ok(serde_json::json!({
        "name": "MCP-like: process-per-poll exists + simctl screenshot observe",
        "path": "mcp_poll_exists_plus_simctl_photos",
        "locale_labels": {
            "settings": settings,
            "general": general,
            "search": search,
            "safari": safari,
        },
        "target_steps": target_steps,
        "steps_run": steps.len(),
        "cycles": cycle,
        "ok": hard_fail.is_none() && failed == 0,
        "hard_fail": hard_fail,
        "passed": passed,
        "failed": failed,
        "failure_rate": if steps.is_empty() { 1.0 } else { failed as f64 / steps.len() as f64 },
        "wall_ms": wall_ms,
        "steps": steps.iter().map(|s| serde_json::json!({
            "i": s.i, "op": s.op, "ok": s.ok, "ms": s.ms, "detail": s.detail
        })).collect::<Vec<_>>(),
        "per_op": per_op,
        "note": "Models spawn-heavy MCP tool loops + photo observe. Not WDA.",
    }))
}

fn wall(v: &serde_json::Value) -> Option<f64> {
    if v.get("skipped").and_then(|x| x.as_bool()) == Some(true) {
        return None;
    }
    v.get("wall_ms").and_then(|x| x.as_f64())
}

fn compute_proof(
    warm: &serde_json::Value,
    carp: &serde_json::Value,
    mono: &serde_json::Value,
    opt_photo: &serde_json::Value,
    wda: &serde_json::Value,
    simctl: &serde_json::Value,
) -> serde_json::Value {
    let warm_ok = warm.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let warm_wall = warm.get("wall_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let warm_fail = warm
        .get("failure_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);

    let carp_wall = wall(carp);
    let mono_wall = wall(mono);
    let opt_wall = wall(opt_photo);
    let wda_wall = wall(wda);
    let wda_skipped = wda.get("skipped").and_then(|v| v.as_bool()).unwrap_or(false)
        || wda.get("wall_ms").is_none();

    let speedup_carp = carp_wall.map(|c| c / warm_wall.max(1.0));
    let speedup_mono = mono_wall.map(|c| c / warm_wall.max(1.0));
    let speedup_opt = opt_wall.map(|c| c / warm_wall.max(1.0));
    let speedup_wda = wda_wall.map(|c| c / warm_wall.max(1.0));

    // Best *fair* competitor: WDA if present, else opt_photo (no process-per-poll).
    // Mono is same APIs — used to detect daemon-only illusion, not as external competitor.
    let best_fair_name;
    let best_fair_wall;
    if let Some(w) = wda_wall {
        best_fair_name = "wda";
        best_fair_wall = Some(w);
    } else if let Some(w) = opt_wall {
        best_fair_name = "opt_photo";
        best_fair_wall = Some(w);
    } else {
        best_fair_name = "none";
        best_fair_wall = None;
    }
    let speedup_fair = best_fair_wall.map(|c| c / warm_wall.max(1.0));

    let hid_opens = warm
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|s| {
                    s.get("op").and_then(|o| o.as_str()) == Some("tap_label")
                        && s.get("detail")
                            .and_then(|d| d.as_str())
                            .map(|d| d.starts_with("hid_"))
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    let simctl_opens = warm
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|s| {
                    s.get("op").and_then(|o| o.as_str()) == Some("tap_label")
                        && s.get("detail")
                            .and_then(|d| d.as_str())
                            .map(|d| d.contains("simctl"))
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    let hid_ok = hid_opens >= simctl_opens && hid_opens > 0;

    let daemon_vs_mono = match (warm_wall, mono_wall) {
        (w, Some(m)) if w > 0.0 => Some(m / w),
        _ => None,
    };

    let verdict = if warm.get("skipped").and_then(|v| v.as_bool()) == Some(true) {
        "NO_WARM_WORKLOAD"
    } else if !warm_ok || warm_fail > 0.0 {
        "FAIL_WARM"
    } else if let Some(sf) = speedup_fair {
        if best_fair_name == "wda" && sf >= 5.0 && warm_fail == 0.0 && hid_ok {
            "HOLY_SHIT"
        } else if best_fair_name == "wda" && sf >= 2.0 && warm_fail == 0.0 {
            "REAL_ADVANTAGE_VS_WDA"
        } else if best_fair_name == "wda" && sf >= 1.2 {
            "MARGINAL_VS_WDA"
        } else if best_fair_name == "opt_photo" && sf >= 2.0 && warm_fail == 0.0 {
            // No WDA yet — cannot HOLY_SHIT; opt-photo win is still meaningful.
            "REAL_ADVANTAGE_VS_OPT_PHOTO"
        } else if speedup_carp.unwrap_or(0.0) >= 5.0 && warm_fail == 0.0 {
            "TECH_SIGNAL_VS_MCP_CARP"
        } else if daemon_vs_mono.map(|r| (r - 1.0).abs() < 0.25).unwrap_or(false)
            && speedup_carp.unwrap_or(0.0) >= 3.0
        {
            "HOST_APIS_WIN_DAEMON_IRRELEVANT"
        } else if speedup_carp.unwrap_or(0.0) >= 3.0 {
            "SUPPORTED_VS_MCP_CARP"
        } else {
            "WEAK_OR_CARP_SUSPECT"
        }
    } else if speedup_carp.unwrap_or(0.0) >= 5.0 && warm_fail == 0.0 {
        "TECH_SIGNAL_VS_MCP_CARP"
    } else {
        "INCOMPLETE_FAIR_BASELINES"
    };

    serde_json::json!({
        "verdict": verdict,
        "warm_wall_ms": warm_wall,
        "warm_ok": warm_ok,
        "warm_failure_rate": warm_fail,
        "carp_wall_ms": carp_wall,
        "mono_wall_ms": mono_wall,
        "opt_photo_wall_ms": opt_wall,
        "wda_wall_ms": wda_wall,
        "wda_skipped": wda_skipped,
        "best_fair_baseline": best_fair_name,
        "speedup_vs_carp": speedup_carp,
        "speedup_vs_mono": speedup_mono,
        "speedup_vs_opt_photo": speedup_opt,
        "speedup_vs_wda": speedup_wda,
        "speedup_vs_best_fair": speedup_fair,
        "daemon_vs_mono_ratio": daemon_vs_mono,
        "settings_open_hid": hid_opens,
        "settings_open_simctl_fallback": simctl_opens,
        "simctl_screenshot_p50_ms": simctl.pointer("/screenshot/stats/p50_ms").and_then(|v| v.as_f64()),
        "meaning": {
            "HOLY_SHIT": "≥5× vs WDA on same workflow + 0% warm fail — product-grade technical win",
            "REAL_ADVANTAGE_VS_WDA": "≥2× vs WDA — invest / keep falsifying",
            "TECH_SIGNAL_VS_MCP_CARP": "≥5× vs process-per-poll MCP only — interesting but may be carp",
            "HOST_APIS_WIN_DAEMON_IRRELEVANT": "mono ≈ lighd; win is host APIs not the daemon socket",
            "REAL_ADVANTAGE_VS_OPT_PHOTO": "beats optimized photo path; still need WDA for HOLY_SHIT",
        }
    })
}

fn fmt_row(name: &str, wall_ms: Option<f64>, failed: Option<u64>, steps: Option<u64>) {
    match (wall_ms, failed, steps) {
        (Some(ms), Some(f), Some(s)) => {
            println!("{name:<16} {:>5.1}s      {f}/{s}", ms / 1000.0);
        }
        _ => println!("{name:<16} --         --"),
    }
}

fn print_agent_bench_human(report: &serde_json::Value) {
    let proof = &report["proof"];
    let w = &report["workload"];
    let cmp = &report["comparison"];
    let verdict = proof["verdict"].as_str().unwrap_or("?");

    let warm_steps = w["steps_run"].as_u64();
    let warm_fail = w["failed"].as_u64();
    let warm_ms = w["wall_ms"].as_f64();

    println!("LIGH Agent Benchmark (try to kill the result)");
    println!("────────────────────────────────────────────");
    println!();
    if let Some(s) = warm_steps {
        println!("Workflow: {s} interactions (same semantic script)");
    } else {
        println!("Workflow: skipped");
    }
    println!();
    println!("                 Time      Failures");
    fmt_row("LIGHd", warm_ms, warm_fail, warm_steps);
    fmt_row(
        "LIGH mono",
        cmp["ligh_mono"]["wall_ms"].as_f64(),
        cmp["ligh_mono"]["failed"].as_u64(),
        cmp["ligh_mono"]["steps_run"].as_u64(),
    );
    fmt_row(
        "opt photo",
        cmp["opt_simctl_photo"]["wall_ms"].as_f64(),
        cmp["opt_simctl_photo"]["failed"].as_u64(),
        cmp["opt_simctl_photo"]["steps_run"].as_u64(),
    );
    fmt_row(
        "MCP carp",
        cmp["mcp_carp"]["wall_ms"].as_f64(),
        cmp["mcp_carp"]["failed"].as_u64(),
        cmp["mcp_carp"]["steps_run"].as_u64(),
    );
    fmt_row(
        "WDA",
        cmp["wda_appium"]["wall_ms"].as_f64(),
        cmp["wda_appium"]["failed"].as_u64(),
        cmp["wda_appium"]["steps_run"].as_u64(),
    );
    println!();
    if let Some(s) = proof["speedup_vs_best_fair"].as_f64() {
        println!(
            "Speedup vs best fair ({}): {:.1}×",
            proof["best_fair_baseline"].as_str().unwrap_or("?"),
            s
        );
    }
    if let Some(s) = proof["speedup_vs_carp"].as_f64() {
        println!("Speedup vs MCP carp:           {s:.1}×  (carp signal only)");
    }
    if let Some(r) = proof["daemon_vs_mono_ratio"].as_f64() {
        println!("mono/lighd ratio:              {r:.2}  (~1.0 ⇒ daemon not the win)");
    }
    println!(
        "Settings open:   HID={}  simctl_fallback={}",
        proof["settings_open_hid"], proof["settings_open_simctl_fallback"]
    );
    println!();
    println!("Verdict: {verdict}");
    if let Some(m) = proof["meaning"][verdict].as_str() {
        println!("Meaning: {m}");
    }
    println!();
    println!("{}", report["note"].as_str().unwrap_or(""));
}
