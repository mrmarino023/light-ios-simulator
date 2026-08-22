//! Control-plane capabilities: ensure_ready, run_app, wait_label, act-with-settle.
//! Settings helpers remain for demos; app-under-test is the product path.
//! These live in lighd — not in Python gate scripts.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    CapabilityResult, FaultClass, ObserveSnapshot, SessionPhase,
};
use ligh_host::{AxDump, HidInput};
use serde_json::json;

use crate::DaemonState;

pub(crate) fn surface_of(snap: &ObserveSnapshot) -> Option<String> {
    snap.scene
        .as_ref()
        .and_then(|s| s.surface.clone())
}

pub(crate) fn phase_of(snap: &ObserveSnapshot) -> SessionPhase {
    snap.phase
        .as_deref()
        .and_then(|p| match p {
            "booting" => Some(SessionPhase::Booting),
            "ax_warming" => Some(SessionPhase::AxWarming),
            "ready" => Some(SessionPhase::Ready),
            "acting" => Some(SessionPhase::Acting),
            "settling" => Some(SessionPhase::Settling),
            "degraded" => Some(SessionPhase::Degraded),
            "dead" => Some(SessionPhase::Dead),
            _ => None,
        })
        .unwrap_or(SessionPhase::Degraded)
}

/// Build one observe + optional settle (reuses daemon observe builder via callback).
pub(crate) fn settle_eyes(
    build: &dyn Fn() -> ObserveSnapshot,
    settle_ms: u64,
) -> ObserveSnapshot {
    let deadline = Instant::now() + Duration::from_millis(settle_ms);
    let mut snap = build();
    while Instant::now() < deadline {
        if snap.is_actionable_eyes() && !snap.eyes_unusable {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
        snap = build();
    }
    snap
}

pub(crate) fn ensure_ready(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
    recover_homes: u32,
) -> CapabilityResult {
    let mut snap = settle_eyes(build, settle_ms.min(1500));
    if snap.is_actionable_eyes() && !snap.eyes_unusable {
        let phase = phase_of(&snap);
        let surface = surface_of(&snap);
        return CapabilityResult::success(
            phase,
            surface,
            "ensure_ready",
            json!({ "recovered": false }),
            Some(snap),
        );
    }

    let udid = match state.lock().unwrap().current_udid() {
        Ok(u) => u,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Dead,
                None,
                "ensure_ready",
                json!({ "error": e }),
                Some(snap),
            );
        }
    };

    for i in 0..recover_homes {
        let _ = HidInput::home(&udid);
        std::thread::sleep(Duration::from_millis(350));
        snap = settle_eyes(build, settle_ms.min(2000));
        if snap.is_actionable_eyes() && !snap.eyes_unusable {
            return CapabilityResult::success(
                phase_of(&snap),
                surface_of(&snap),
                "ensure_ready",
                json!({ "recovered": true, "homes": i + 1 }),
                Some(snap),
            );
        }
    }

    let fault = if snap.ax_quality == "error" {
        FaultClass::Infra
    } else if snap.eyes_unusable {
        FaultClass::EyesUnusable
    } else {
        FaultClass::Timeout
    };
    CapabilityResult::fail(
        fault,
        phase_of(&snap),
        surface_of(&snap),
        "ensure_ready",
        json!({ "ax_quality": snap.ax_quality, "homes": recover_homes }),
        Some(snap),
    )
}

fn locale_settings_labels(snap: &ObserveSnapshot) -> (String, String, String) {
    let labs: Vec<String> = snap
        .actionable_topk
        .iter()
        .filter_map(|n| n.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let settings = if labs.iter().any(|l| l == "Settings") {
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
    (settings.into(), general.into(), search.into())
}

pub(crate) fn open_settings(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms, 4);
    if !ready.ok {
        return ready;
    }
    let snap0 = ready.observe.clone().unwrap_or_else(build);
    let (settings, general, _search) = locale_settings_labels(&snap0);

    let (udid, w, h) = {
        let st = state.lock().unwrap();
        match st.current_udid() {
            Ok(u) => (u, st.sim_width, st.sim_height),
            Err(e) => {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Dead,
                    None,
                    "open_settings",
                    json!({ "error": e }),
                    Some(snap0),
                );
            }
        }
    };

    // Prefer icon tap; fallback Preferences launch.
    let mut path = "hid_tap";
    match AxDump::wait_label(&udid, &settings, Duration::from_millis(3500)) {
        Ok((x, y, _)) => {
            if HidInput::tap(&udid, x, y, w, h).is_err() {
                path = "tap_failed_fallback_launch";
                let _ = std::process::Command::new("xcrun")
                    .args(["simctl", "launch", &udid, "com.apple.Preferences"])
                    .output();
            }
        }
        Err(_) => {
            path = "simctl_launch";
            let _ = std::process::Command::new("xcrun")
                .args(["simctl", "launch", &udid, "com.apple.Preferences"])
                .output();
        }
    }
    std::thread::sleep(Duration::from_millis(400));

    // Pop toward root if needed.
    for _ in 0..6 {
        let snap = settle_eyes(build, settle_ms.min(2000));
        let surface = surface_of(&snap).unwrap_or_default();
        let labs: Vec<_> = snap
            .actionable_topk
            .iter()
            .filter_map(|n| n.get("label").and_then(|v| v.as_str()))
            .collect();
        if surface == "settings"
            && (labs.contains(&general.as_str())
                || labs.iter().any(|l| l.eq_ignore_ascii_case("bluetooth")))
        {
            return CapabilityResult::success(
                phase_of(&snap),
                Some(surface),
                "open_settings",
                json!({ "path": path, "settings_label": settings }),
                Some(snap),
            );
        }
        // Back chrome tap
        let _ = HidInput::tap(&udid, 0.11, 0.09, w, h);
        std::thread::sleep(Duration::from_millis(220));
        for cancel in ["Annulla", "Cancel"] {
            if let Ok((x, y, _)) = AxDump::wait_label(&udid, cancel, Duration::from_millis(400)) {
                let _ = HidInput::tap(&udid, x, y, w, h);
            }
        }
    }

    let snap = settle_eyes(build, settle_ms.min(2000));
    let surface = surface_of(&snap);
    if surface.as_deref() == Some("settings") {
        return CapabilityResult::success(
            phase_of(&snap),
            surface,
            "open_settings",
            json!({ "path": path, "soft": true }),
            Some(snap),
        );
    }
    CapabilityResult::fail(
        FaultClass::WrongSurface,
        phase_of(&snap),
        surface,
        "open_settings",
        json!({ "path": path, "expected": "settings" }),
        Some(snap),
    )
}

pub(crate) fn settings_search(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    query: &str,
    settle_ms: u64,
) -> CapabilityResult {
    let opened = open_settings(build, state, settle_ms);
    if !opened.ok {
        return opened;
    }
    let snap0 = opened.observe.clone().unwrap_or_else(build);
    let (_settings, _general, search) = locale_settings_labels(&snap0);

    let (udid, w, h) = {
        let st = state.lock().unwrap();
        match st.current_udid() {
            Ok(u) => (u, st.sim_width, st.sim_height),
            Err(e) => {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Dead,
                    None,
                    "settings_search",
                    json!({ "error": e }),
                    opened.observe,
                );
            }
        }
    };

    match AxDump::wait_label(&udid, &search, Duration::from_millis(5000)) {
        Ok((x, y, _)) => {
            if let Err(e) = HidInput::tap(&udid, x, y, w, h) {
                return CapabilityResult::fail(
                    FaultClass::MotorRejected,
                    SessionPhase::Degraded,
                    surface_of(&snap0),
                    "settings_search",
                    json!({ "error": e.to_string(), "step": "tap_search" }),
                    Some(snap0),
                );
            }
        }
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::TargetMissing,
                phase_of(&snap0),
                surface_of(&snap0),
                "settings_search",
                json!({ "error": e.to_string(), "label": search }),
                Some(snap0),
            );
        }
    }
    std::thread::sleep(Duration::from_millis(250));
    if let Err(e) = HidInput::type_text(&udid, query) {
        return CapabilityResult::fail(
            FaultClass::MotorRejected,
            SessionPhase::Degraded,
            surface_of(&snap0),
            "settings_search",
            json!({ "error": e.to_string(), "step": "type" }),
            Some(snap0),
        );
    }
    {
        let mut st = state.lock().unwrap();
        st.push_action_result(
            true,
            "typed",
            json!({ "verified": "host_accepted", "text": query }),
        );
        // Also push typed sensation for agents
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        st.sense_buf.push(ligh_core::SenseEvent {
            t: now,
            kind: "typed".into(),
            payload: Some(json!({ "verified": "host_accepted", "text": query })),
        });
    }
    std::thread::sleep(Duration::from_millis(400));
    let snap = settle_eyes(build, settle_ms);
    let labs: Vec<String> = snap
        .actionable_topk
        .iter()
        .filter_map(|n| n.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let q = query.to_ascii_lowercase();
    let hit = labs.iter().any(|l| l.to_ascii_lowercase().contains(&q));
    if hit && surface_of(&snap).as_deref() == Some("settings") {
        return CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "settings_search",
            json!({ "query": query, "hit": true }),
            Some(snap),
        );
    }
    if hit {
        return CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "settings_search",
            json!({ "query": query, "hit": true, "surface_soft": true }),
            Some(snap),
        );
    }
    CapabilityResult::fail(
        FaultClass::TargetMissing,
        phase_of(&snap),
        surface_of(&snap),
        "settings_search",
        json!({ "query": query, "labels": labs }),
        Some(snap),
    )
}

pub(crate) fn assert_surface(
    build: &dyn Fn() -> ObserveSnapshot,
    want: &str,
    settle_ms: u64,
) -> CapabilityResult {
    let snap = settle_eyes(build, settle_ms);
    let got = surface_of(&snap).unwrap_or_else(|| "unknown".into());
    if snap.eyes_unusable {
        return CapabilityResult::fail(
            FaultClass::EyesUnusable,
            phase_of(&snap),
            Some(got),
            "assert_surface",
            json!({ "expected": want }),
            Some(snap),
        );
    }
    if got == want {
        CapabilityResult::success(
            phase_of(&snap),
            Some(got),
            "assert_surface",
            json!({ "expected": want }),
            Some(snap),
        )
    } else {
        CapabilityResult::fail(
            FaultClass::WrongSurface,
            phase_of(&snap),
            Some(got.clone()),
            "assert_surface",
            json!({ "expected": want, "got": got }),
            Some(snap),
        )
    }
}

pub(crate) fn act_tap(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    crate::motor::motor_tap(build, state, label, id, settle_ms, timeout_ms)
}

fn detect_bundle_id(app_path: &std::path::Path) -> Option<String> {
    let plist = app_path.join("Info.plist");
    std::process::Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            }
        })
}

/// Product path: install Debug `.app` → launch → settle → optional wait chrome.
/// Set `install=false` to relaunch only (gates should install once, then relaunch).
pub(crate) fn run_app(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    app: &str,
    bundle_id: Option<&str>,
    wait_label: Option<&str>,
    wait_id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
    install: bool,
) -> CapabilityResult {
    let app_path = match std::path::Path::new(app).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Dead,
                None,
                "run_app",
                json!({ "error": format!("invalid app: {e}"), "app": app }),
                None,
            );
        }
    };
    let udid = match state.lock().unwrap().current_udid() {
        Ok(u) => u,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Dead,
                None,
                "run_app",
                json!({ "error": e }),
                None,
            );
        }
    };
    let bid = match bundle_id.map(|s| s.to_string()).or_else(|| detect_bundle_id(&app_path)) {
        Some(b) => b,
        None => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Degraded,
                None,
                "run_app",
                json!({ "error": "could not detect CFBundleIdentifier" }),
                None,
            );
        }
    };

    let launch_once = |force_install: bool| -> Result<(), String> {
        if force_install {
            ligh_sim::Simctl::run(&[
                "install",
                &udid,
                app_path.to_str().unwrap_or(""),
            ])
            .map_err(|e| e.to_string())?;
        }
        let _ = ligh_sim::Simctl::run(&["terminate", &udid, &bid]);
        std::thread::sleep(Duration::from_millis(150));
        ligh_sim::Simctl::run(&["launch", &udid, &bid, "--terminate-running-process"])
            .map_err(|e| e.to_string())?;
        // Let AX attach to the new process before first dump.
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    };

    if let Err(e) = launch_once(install) {
        if !install {
            if let Err(e2) = launch_once(true) {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Degraded,
                    None,
                    "run_app",
                    json!({ "error": e2, "step": "launch_retry", "prior": e }),
                    None,
                );
            }
        } else {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Degraded,
                None,
                "run_app",
                json!({ "error": e, "step": "launch", "bundle_id": bid }),
                None,
            );
        }
    }
    if let Ok(cfg) = ligh_core::LighConfig::load() {
        if let Ok(Some(mut s)) = ligh_core::SessionState::load(&cfg.state_dir) {
            s.app_bundle_id = Some(bid.clone());
            s.app_path = Some(app_path.clone());
            let _ = s.save(&cfg.state_dir);
        }
    }

    let chrome_wait = |attempt: &str| -> CapabilityResult {
        let ready = ensure_ready(build, state, settle_ms, 6);
        if !ready.ok {
            return CapabilityResult::fail(
                ready.fault,
                ready.phase,
                ready.surface.clone(),
                "run_app",
                json!({
                    "bundle_id": bid,
                    "step": "ensure_ready",
                    "attempt": attempt,
                    "install": install,
                    "detail": ready.detail,
                }),
                ready.observe,
            );
        }
        if wait_label.is_none() && wait_id.is_none() {
            return CapabilityResult::success(
                ready.phase,
                ready.surface.clone(),
                "run_app",
                json!({ "bundle_id": bid, "install": install, "attempt": attempt }),
                ready.observe,
            );
        }
        let mut r = crate::motor::motor_wait(
            build,
            state,
            wait_label,
            wait_id,
            settle_ms,
            timeout_ms,
        );
        if let Some(detail) = r.detail.as_mut() {
            if let Some(obj) = detail.as_object_mut() {
                obj.insert("bundle_id".into(), json!(bid));
                obj.insert("attempt".into(), json!(attempt));
                obj.insert("install".into(), json!(install));
            }
        }
        r.capability = Some("run_app".into());
        r
    };

    let mut out = chrome_wait("1");
    if !out.ok {
        // Hard recovery: relaunch process, re-warm AX, wait again.
        let _ = launch_once(false);
        out = chrome_wait("2");
    }
    if !out.ok {
        let _ = launch_once(true);
        out = chrome_wait("3");
    }
    if !out.ok {
        return CapabilityResult::fail(
            out.fault,
            out.phase,
            out.surface,
            "run_app",
            json!({
                "step": "wait_chrome",
                "wait_label": wait_label,
                "wait_id": wait_id,
                "detail": out.detail,
                "install": install,
            }),
            out.observe,
        );
    }
    out
}

fn wait_label_inner(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: &str,
    settle_ms: u64,
    timeout_ms: u64,
    extra: Option<serde_json::Value>,
) -> CapabilityResult {
    let mut r = crate::motor::motor_wait(
        build,
        state,
        Some(label),
        None,
        settle_ms,
        timeout_ms,
    );
    if let Some(ex) = extra {
        if let Some(detail) = r.detail.as_mut() {
            if let (Some(a), Some(b)) = (detail.as_object_mut(), ex.as_object()) {
                for (k, v) in b {
                    a.insert(k.clone(), v.clone());
                }
            }
        }
    }
    if r.capability.as_deref() == Some("wait") {
        r.capability = Some("wait_label".into());
    }
    r
}

/// Settle → wait until AX label exists (developer app chrome).
pub(crate) fn wait_label(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: &str,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    wait_label_inner(build, state, label, settle_ms, timeout_ms, None)
}

pub(crate) fn act_type(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    text: &str,
    settle_ms: u64,
) -> CapabilityResult {
    crate::motor::motor_type(build, state, text, settle_ms)
}

/// First-class app job: run-app then a sequence of motor steps (one capability).
/// Steps: `{"op":"wait"|"tap","id"|"label":"..."}` or `{"op":"type","text":"..."}`.
pub(crate) fn app_job(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    app: &str,
    bundle_id: Option<&str>,
    steps: &[serde_json::Value],
    settle_ms: u64,
    timeout_ms: u64,
    install: bool,
) -> CapabilityResult {
    let home_label = steps.first().and_then(|s| {
        if s.get("op").and_then(|v| v.as_str()) == Some("wait") {
            s.get("label").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            None
        }
    });
    let home_id = steps.first().and_then(|s| {
        if s.get("op").and_then(|v| v.as_str()) == Some("wait") {
            s.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            None
        }
    });
    let launched = run_app(
        build,
        state,
        app,
        bundle_id,
        home_label.as_deref(),
        home_id.as_deref(),
        settle_ms,
        timeout_ms,
        install,
    );
    if !launched.ok {
        return CapabilityResult::fail(
            launched.fault,
            launched.phase,
            launched.surface,
            "app_job",
            json!({ "step": 0, "op": "run_app", "detail": launched.detail }),
            launched.observe,
        );
    }

    let start = if home_label.is_some() || home_id.is_some() {
        1
    } else {
        0
    };
    let mut last = launched;
    for (i, step) in steps.iter().enumerate().skip(start) {
        let op = step.get("op").and_then(|v| v.as_str()).unwrap_or("");
        let r = match op {
            "wait" => crate::motor::motor_wait(
                build,
                state,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                settle_ms,
                timeout_ms,
            ),
            "tap" => crate::motor::motor_tap(
                build,
                state,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                settle_ms,
                timeout_ms,
            ),
            "type" => {
                let text = step.get("text").and_then(|v| v.as_str()).unwrap_or("");
                crate::motor::motor_type(build, state, text, settle_ms)
            }
            _ => {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Ready,
                    last.surface.clone(),
                    "app_job",
                    json!({ "step": i, "error": format!("unknown op {op}") }),
                    last.observe,
                );
            }
        };
        if !r.ok {
            return CapabilityResult::fail(
                r.fault,
                r.phase,
                r.surface,
                "app_job",
                json!({ "step": i, "op": op, "detail": r.detail }),
                r.observe,
            );
        }
        last = r;
    }
    CapabilityResult::success(
        last.phase,
        last.surface.clone(),
        "app_job",
        json!({
            "steps": steps.len(),
            "motor": "ensure_path",
            "detail": last.detail,
        }),
        last.observe,
    )
}
