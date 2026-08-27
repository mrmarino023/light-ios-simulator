//! Control-plane capabilities: ensure_ready, run_app, wait_label, act-with-settle.
//! Settings helpers remain for demos; app-under-test is the product path.
//! These live in lighd — not in Python gate scripts.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    detect_surface, CapabilityResult, FaultClass, ObserveSnapshot, SessionPhase,
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

fn ax_nodes(snap: &ObserveSnapshot) -> &[serde_json::Value] {
    snap.accessibility_tree.nodes()
}

fn node_label(n: &serde_json::Value) -> Option<&str> {
    n.get("label")
        .and_then(|v| v.as_str())
        .or_else(|| n.get("identifier").and_then(|v| v.as_str()))
}

fn looks_like_springboard(snap: &ObserveSnapshot) -> bool {
    if snap
        .scene
        .as_ref()
        .and_then(|s| s.surface.as_deref())
        == Some("springboard")
    {
        return true;
    }
    let nodes = ax_nodes(snap);
    if detect_surface(&nodes) == "springboard" {
        return true;
    }
    // A real app owns the tree: whatever its layout looks like, this is not the home
    // grid. Without this, any list-of-buttons screen trips the icon-count heuristic.
    if ligh_core::foreground_app_label(&nodes).is_some() {
        return false;
    }
    let has_spotlight = nodes.iter().any(|n| {
        n.get("identifier").and_then(|v| v.as_str()) == Some("spotlight-pill")
            || node_label(n).map(|l| l.eq_ignore_ascii_case("cerca") || l.eq_ignore_ascii_case("search"))
                == Some(true)
    });
    let home_buttons = nodes
        .iter()
        .filter(|n| {
            let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
            role.contains("Button")
                && n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(false)
                && n.get("frame")
                    .and_then(|f| f.get("y"))
                    .and_then(|y| y.as_f64())
                    .map(|y| y < 700.0)
                    .unwrap_or(false)
        })
        .count();
    // Home grid: spotlight + many icons, or dense icon grid without spotlight (slim sims).
    (has_spotlight && home_buttons >= 6) || home_buttons >= 8
}

fn only_app_icon_on_home(snap: &ObserveSnapshot, app_label: &str) -> bool {
    if !looks_like_springboard(snap) {
        return false;
    }
    ax_nodes(snap).iter().any(|n| {
        // The app's own AXApplication node carries the same label as its icon.
        let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
        !role.contains("Application")
            && node_label(n).map(|l| l.eq_ignore_ascii_case(app_label)).unwrap_or(false)
    })
}

fn ax_has_marker(snap: &ObserveSnapshot, wait_label: Option<&str>, wait_id: Option<&str>) -> bool {
    if let Some(id) = wait_id {
        if snap
            .accessibility_tree
            .nodes()
            .iter()
            .any(|n| ligh_core::node_matches_identifier(n, id))
        {
            return true;
        }
    }
    if let Some(lab) = wait_label {
        if snap.accessibility_tree.find_label(lab).is_some() {
            return true;
        }
    }
    false
}

/// Trust gate: confirm we are *inside* the expected app, not looking at its SpringBoard icon.
///
/// Never accept `scene.bundle_id` alone — that lies while AX is still SpringBoard.
pub(crate) fn confirm_app_ready(
    snap: &ObserveSnapshot,
    bundle_id: &str,
    app_label: &str,
    wait_label: Option<&str>,
    wait_id: Option<&str>,
) -> CapabilityResult {
    // Entry markers prove in-app chrome — same matching as motor ensure_path.
    // Surface heuristics lie while AX attaches; trust markers when motor would.
    if wait_label.is_some() || wait_id.is_some() {
        if ax_has_marker(snap, wait_label, wait_id) {
            return CapabilityResult::success(
                phase_of(snap),
                surface_of(snap),
                "app_ready",
                json!({
                    "app_ready": true,
                    "via": "entry_marker",
                    "bundle_id": bundle_id,
                }),
                Some(snap.clone()),
            );
        }
    }

    if looks_like_springboard(snap) || only_app_icon_on_home(snap, app_label) {
        return CapabilityResult::fail(
            FaultClass::AppNotForeground,
            phase_of(snap),
            surface_of(snap),
            "app_ready",
            json!({
                "reason": "app_not_foreground",
                "bundle_id": bundle_id,
                "app_label": app_label,
                "surface": surface_of(snap),
                "detail": "AX tree looks like SpringBoard / home grid — icon ≠ foreground",
            }),
            Some(snap.clone()),
        );
    }

    if wait_label.is_some() || wait_id.is_some() {
        return CapabilityResult::fail(
            FaultClass::AppNotForeground,
            phase_of(snap),
            surface_of(snap),
            "app_ready",
            json!({
                "reason": "entry_marker_missing",
                "bundle_id": bundle_id,
                "wait_label": wait_label,
                "wait_id": wait_id,
            }),
            Some(snap.clone()),
        );
    }

    // No markers: require non-springboard surface and settled AX with actionable content.
    let surface = surface_of(snap).unwrap_or_else(|| "unknown".into());
    if surface == "springboard" || surface == "transition" {
        return CapabilityResult::fail(
            FaultClass::AppNotForeground,
            phase_of(snap),
            Some(surface),
            "app_ready",
            json!({
                "reason": "app_not_foreground",
                "bundle_id": bundle_id,
                "surface": surface_of(snap),
            }),
            Some(snap.clone()),
        );
    }

    let actionable = snap.actionable_topk.len();
    if actionable == 0 || !snap.settled {
        return CapabilityResult::fail(
            FaultClass::AppNotForeground,
            phase_of(snap),
            surface_of(snap),
            "app_ready",
            json!({
                "reason": "app_not_ready",
                "bundle_id": bundle_id,
                "actionable": actionable,
                "settled": snap.settled,
            }),
            Some(snap.clone()),
        );
    }

    CapabilityResult::success(
        phase_of(snap),
        surface_of(snap),
        "app_ready",
        json!({
            "app_ready": true,
            "via": "ax_surface",
            "bundle_id": bundle_id,
            "surface": surface,
            "actionable": actionable,
        }),
        Some(snap.clone()),
    )
}

fn app_label_from_path(app: &str) -> String {
    std::path::Path::new(app)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app")
        .to_string()
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
    crate::motor::motor_tap(build, state, label, id, settle_ms, timeout_ms, None, None)
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
    launch_args: Option<&[String]>,
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

    // Relaunch-only: skip terminate/launch if app is foreground and wait chrome is already up.
    if !install {
        let snap = build();
        let front = snap
            .scene
            .as_ref()
            .and_then(|s| s.bundle_id.as_deref());
        if front == Some(bid.as_str()) {
            let chrome_ready = match (wait_id.as_ref(), wait_label.as_ref()) {
                (Some(eid), _) => crate::motor::target_onscreen_udid(&udid, None, Some(eid)),
                (None, Some(lab)) => crate::motor::target_onscreen_udid(&udid, Some(lab), None),
                (None, None) => true,
            };
            if chrome_ready {
                let app_label = app_label_from_path(app_path.to_str().unwrap_or(app));
                let snap = build();
                let trust = confirm_app_ready(
                    &snap,
                    &bid,
                    &app_label,
                    wait_label,
                    wait_id,
                );
                if trust.ok {
                    return CapabilityResult::success(
                        trust.phase,
                        trust.surface.clone(),
                        "run_app",
                        json!({
                            "bundle_id": bid,
                            "install": false,
                            "skipped_relaunch": true,
                            "app_ready": true,
                        }),
                        trust.observe.or(Some(snap)),
                    );
                }
            }
        }
    }

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
        let mut launch_cmd = vec![
            "launch".to_string(),
            udid.clone(),
            bid.clone(),
            "--terminate-running-process".to_string(),
        ];
        if let Some(extra) = launch_args {
            launch_cmd.extend(extra.iter().cloned());
        }
        let launch_refs: Vec<&str> = launch_cmd.iter().map(|s| s.as_str()).collect();
        ligh_sim::Simctl::run(&launch_refs).map_err(|e| e.to_string())?;
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
            s.begin_launch(bid.clone(), Some(app_path.clone()));
            let _ = s.save(&cfg.state_dir);
            let mut daemon = state.lock().unwrap();
            daemon.session_id = s.session_id.clone();
            daemon.boot_epoch = s.boot_epoch;
            daemon.launch_epoch = s.launch_epoch;
            daemon.expected_bundle_id = Some(bid.clone());
            daemon.screen_epoch = daemon.screen_epoch.saturating_add(1).max(1);
            daemon.stability_streak = 0;
            daemon.last_screen_fingerprint = None;
        }
    }

    // Foreground settle: simctl launch often leaves AX on the home grid while scene.bundle_id lies.
    let app_label = app_label_from_path(app_path.to_str().unwrap_or(app));
    for fg in 1..=10u32 {
        let wait_ms = settle_ms.min(1200) + u64::from(fg) * 400;
        let snap = settle_eyes(build, wait_ms);
        let trust = confirm_app_ready(&snap, &bid, &app_label, None, None);
        if trust.ok {
            break;
        }
        // Give AX time to attach after launch_once before forcing another relaunch.
        if fg <= 2 {
            continue;
        }
        let icon_on_home = only_app_icon_on_home(&snap, &app_label)
            || ax_nodes(&snap).iter().any(|n| {
                node_label(n)
                    .map(|l| l.eq_ignore_ascii_case(&app_label))
                    .unwrap_or(false)
            });
        if icon_on_home || (looks_like_springboard(&snap) && fg >= 3) {
            if icon_on_home {
                let _ = crate::motor::motor_tap(
                    build,
                    state,
                    Some(app_label.as_str()),
                    None,
                    settle_ms.min(2000),
                    timeout_ms.min(8000),
                    None,
                    None,
                );
                std::thread::sleep(Duration::from_millis(1200));
            } else if looks_like_springboard(&snap) {
                // Relaunch without terminate — killing the process mid-attach keeps AX on SpringBoard.
                let _ = ligh_sim::Simctl::run(&["launch", &udid, &bid]);
                std::thread::sleep(Duration::from_millis(900 + u64::from(fg) * 150));
            }
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
        let app_label = app_label_from_path(app_path.to_str().unwrap_or(app));

        // Optional chrome markers: wait first, then trust-gate on the settled tree.
        let snap_after_chrome = if wait_label.is_some() || wait_id.is_some() {
            let snap0 = build();
            if looks_like_springboard(&snap0) || only_app_icon_on_home(&snap0, &app_label) {
                let _ = crate::motor::motor_tap(
                    build,
                    state,
                    Some(app_label.as_str()),
                    None,
                    settle_ms.min(2000),
                    timeout_ms.min(8000),
                    None,
                    None,
                );
                std::thread::sleep(Duration::from_millis(900));
            }
            let mut r = crate::motor::motor_wait(
                build,
                state,
                wait_label,
                wait_id,
                settle_ms,
                timeout_ms,
            );
            if !r.ok {
                if let Some(detail) = r.detail.as_mut() {
                    if let Some(obj) = detail.as_object_mut() {
                        obj.insert("bundle_id".into(), json!(bid));
                        obj.insert("attempt".into(), json!(attempt));
                        obj.insert("install".into(), json!(install));
                    }
                }
                r.capability = Some("run_app".into());
                return r;
            }
            let snap = r.observe.unwrap_or_else(|| build());
            // Motor ensure_path proved entry chrome on live AX dump — snapshot surface may lag.
            return CapabilityResult::success(
                phase_of(&snap),
                surface_of(&snap),
                "run_app",
                json!({
                    "bundle_id": bid,
                    "install": install,
                    "attempt": attempt,
                    "app_ready": true,
                    "via": "motor_chrome",
                    "wait_label": wait_label,
                    "wait_id": wait_id,
                }),
                Some(snap),
            );
        } else {
            ready.observe.clone().unwrap_or_else(|| build())
        };

        let trust = confirm_app_ready(
            &snap_after_chrome,
            &bid,
            &app_label,
            wait_label,
            wait_id,
        );
        if !trust.ok {
            return CapabilityResult::fail(
                trust.fault,
                trust.phase,
                trust.surface.clone(),
                "run_app",
                json!({
                    "bundle_id": bid,
                    "step": "app_ready",
                    "attempt": attempt,
                    "install": install,
                    "detail": trust.detail,
                }),
                trust.observe.or(Some(snap_after_chrome)),
            );
        }

        CapabilityResult::success(
            phase_of(&snap_after_chrome),
            surface_of(&snap_after_chrome),
            "run_app",
            json!({
                "bundle_id": bid,
                "install": install,
                "attempt": attempt,
                "app_ready": true,
                "wait_label": wait_label,
                "wait_id": wait_id,
            }),
            Some(snap_after_chrome),
        )
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
    crate::motor::motor_type(
        build,
        state,
        text,
        None,
        None,
        settle_ms,
        12_000,
        ligh_core::MotorTypeStrategy::FocusHid,
    )
}

fn step_labels(step: &serde_json::Value) -> Vec<String> {
    if let Some(arr) = step.get("labels").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }
    step.get("label")
        .and_then(|v| v.as_str())
        .map(|s| vec![s.to_string()])
        .unwrap_or_default()
}

/// Universal motor step — same ops for app_job, app_goal, and any app (Debug .app or system).
pub(crate) fn run_motor_step(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    step: &serde_json::Value,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let op = step.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "launch" => {
            let bid = step
                .get("bundle_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if bid.is_empty() {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Ready,
                    None,
                    "launch",
                    json!({ "error": "bundle_id required" }),
                    None,
                );
            }
            crate::motor::motor_launch(build, state, bid, settle_ms)
        }
        "wait" => {
            let labels = step_labels(step);
            if !labels.is_empty() {
                let mut last = None;
                for lab in &labels {
                    let r = crate::motor::motor_wait(
                        build,
                        state,
                        Some(lab.as_str()),
                        None,
                        settle_ms,
                        timeout_ms,
                    );
                    if r.ok {
                        return r;
                    }
                    last = Some(r);
                }
                return last.unwrap_or_else(|| {
                    CapabilityResult::fail(
                        FaultClass::TargetMissing,
                        SessionPhase::Ready,
                        None,
                        "wait",
                        json!({ "labels": labels }),
                        None,
                    )
                });
            }
            crate::motor::motor_wait(
                build,
                state,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                settle_ms,
                timeout_ms,
            )
        }
        "tap" => {
            let until_id = step
                .get("until_id")
                .or_else(|| step.get("until"))
                .and_then(|v| v.as_str());
            let until_label = step.get("until_label").and_then(|v| v.as_str());
            let labels = step_labels(step);
            if !labels.is_empty() {
                let mut last = None;
                for lab in &labels {
                    let r = crate::motor::motor_tap(
                        build,
                        state,
                        Some(lab.as_str()),
                        None,
                        settle_ms,
                        timeout_ms,
                        until_id,
                        until_label,
                    );
                    if r.ok {
                        return r;
                    }
                    last = Some(r);
                }
                return last.unwrap_or_else(|| {
                    CapabilityResult::fail(
                        FaultClass::TargetMissing,
                        SessionPhase::Ready,
                        None,
                        "tap",
                        json!({ "labels": labels }),
                        None,
                    )
                });
            }
            crate::motor::motor_tap(
                build,
                state,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                settle_ms,
                timeout_ms,
                until_id,
                until_label,
            )
        }
        "type" => {
            let text = step.get("text").and_then(|v| v.as_str()).unwrap_or("");
            crate::motor::motor_type(
                build,
                state,
                text,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                settle_ms,
                timeout_ms,
                ligh_core::MotorTypeStrategy::FocusHid,
            )
        }
        "key" => {
            let name = step
                .get("name")
                .or_else(|| step.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("return");
            crate::motor::motor_key(build, state, name, settle_ms)
        }
        "scroll_until" => {
            let max_swipes = step
                .get("max_swipes")
                .and_then(|v| v.as_u64())
                .unwrap_or(8) as u32;
            crate::motor::motor_scroll_until(
                build,
                state,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                max_swipes,
                timeout_ms,
            )
        }
        "dismiss_overlay" => crate::motor::motor_dismiss_overlay(build, state, settle_ms),
        "explore" => {
            let max_probes = step
                .get("max_probes")
                .and_then(|v| v.as_u64())
                .unwrap_or(4) as u32;
            let max_swipes = step
                .get("max_swipes")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as u32;
            crate::motor::motor_explore(
                build,
                state,
                step.get("label").and_then(|v| v.as_str()),
                step.get("id").and_then(|v| v.as_str()),
                max_probes,
                max_swipes,
                settle_ms,
                timeout_ms,
            )
        }
        _ => CapabilityResult::fail(
            FaultClass::Infra,
            SessionPhase::Ready,
            None,
            op,
            json!({ "error": format!("unknown op {op}") }),
            None,
        ),
    }
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
    launch_args: Option<&[String]>,
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
        launch_args,
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
        let r = run_motor_step(build, state, step, settle_ms, timeout_ms);
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

/// Goal-driven job: optional run_app, setup steps, then postconditions (wait_id / wait_label).
pub(crate) fn app_goal(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    app: Option<&str>,
    bundle_id: Option<&str>,
    setup: &[serde_json::Value],
    postconditions: &[serde_json::Value],
    settle_ms: u64,
    timeout_ms: u64,
    install: bool,
    launch_args: Option<&[String]>,
) -> CapabilityResult {
    let mut last = if let Some(app_path) = app {
        let home_id = postconditions
            .first()
            .and_then(|p| p.get("wait_id").and_then(|v| v.as_str()))
            .or_else(|| {
                setup.iter().find_map(|s| {
                    (s.get("op").and_then(|v| v.as_str()) == Some("wait"))
                        .then(|| s.get("id").and_then(|v| v.as_str()))
                        .flatten()
                })
            });
        let home_label = postconditions
            .first()
            .and_then(|p| p.get("wait_label").and_then(|v| v.as_str()))
            .or_else(|| {
                setup.iter().find_map(|s| {
                    (s.get("op").and_then(|v| v.as_str()) == Some("wait"))
                        .then(|| s.get("label").and_then(|v| v.as_str()))
                        .flatten()
                })
            });
        let launched = run_app(
            build,
            state,
            app_path,
            bundle_id,
            home_label,
            home_id,
            settle_ms,
            timeout_ms,
            install,
            launch_args,
        );
        if !launched.ok {
            return CapabilityResult::fail(
                launched.fault,
                launched.phase,
                launched.surface,
                "app_goal",
                json!({ "phase": "run_app", "detail": launched.detail }),
                launched.observe,
            );
        }
        launched
    } else {
        let ready = ensure_ready(build, state, settle_ms, 3);
        if !ready.ok {
            return CapabilityResult::fail(
                ready.fault,
                ready.phase,
                ready.surface,
                "app_goal",
                json!({ "phase": "ready", "detail": ready.detail }),
                ready.observe,
            );
        }
        ready
    };

    for (i, step) in setup.iter().enumerate() {
        let op = step.get("op").and_then(|v| v.as_str()).unwrap_or("");
        let r = run_motor_step(build, state, step, settle_ms, timeout_ms);
        if !r.ok {
            return CapabilityResult::fail(
                r.fault,
                r.phase,
                r.surface,
                "app_goal",
                json!({ "phase": "setup", "step": i, "op": op, "detail": r.detail }),
                r.observe,
            );
        }
        last = r;
    }

    for (i, post) in postconditions.iter().enumerate() {
        let wait_id = post.get("wait_id").and_then(|v| v.as_str());
        let wait_label = post.get("wait_label").and_then(|v| v.as_str());
        let post_timeout = post
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(timeout_ms);
        let max_swipes = post
            .get("max_swipes")
            .and_then(|v| v.as_u64())
            .unwrap_or(12) as u32;
        let r = crate::motor::motor_reach(
            build,
            state,
            wait_label,
            wait_id,
            max_swipes,
            settle_ms,
            post_timeout,
        );
        if !r.ok {
            return CapabilityResult::fail(
                r.fault,
                r.phase,
                r.surface,
                "app_goal",
                json!({
                    "phase": "postcondition",
                    "index": i,
                    "wait_id": wait_id,
                    "wait_label": wait_label,
                    "detail": r.detail,
                }),
                r.observe,
            );
        }
        last = r;
    }

    CapabilityResult::success(
        last.phase,
        last.surface.clone(),
        "app_goal",
        json!({
            "postconditions": postconditions.len(),
            "setup_steps": setup.len(),
            "motor": "reach",
        }),
        last.observe,
    )
}
