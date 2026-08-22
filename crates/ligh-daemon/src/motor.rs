//! Unified motor: ready → resolve → ensure_path → fire → settle.
//!
//! This is the architectural spine for app-under-test automation.
//! Overlays are cleared here — never as one-off side effects on type/tap.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    find_id_in_dump, find_label_in_dump, overlay_from_snapshot, CapabilityResult, FaultClass,
    ObserveSnapshot, Overlay, SessionPhase,
};
use ligh_host::{AxDump, HidInput};
use serde_json::json;

use crate::capabilities::{ensure_ready, phase_of, settle_eyes, surface_of};
use crate::DaemonState;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedTarget {
    pub nx: f64,
    pub ny: f64,
    pub name: String,
    pub hittable: bool,
}

fn dump_nodes(dump: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    dump.get("elements")
        .and_then(|e| e.as_array())
        .or_else(|| dump.get("nodes").and_then(|e| e.as_array()))
}

fn node_hittable(n: &serde_json::Value) -> bool {
    n.get("hittable")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn find_node<'a>(
    dump: &'a serde_json::Value,
    label: Option<&str>,
    id: Option<&str>,
) -> Option<&'a serde_json::Value> {
    let nodes = dump_nodes(dump)?;
    if let Some(eid) = id {
        return nodes.iter().find(|n| {
            n.get("identifier").and_then(|v| v.as_str()) == Some(eid)
                || n.get("id").and_then(|v| v.as_str()) == Some(eid)
        });
    }
    if let Some(lab) = label {
        let needle = lab.to_ascii_lowercase();
        return nodes.iter().find(|n| {
            n.get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase().contains(&needle))
                .unwrap_or(false)
                || n.get("identifier")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
        });
    }
    None
}

fn resolve_from_dump(
    dump: &serde_json::Value,
    label: Option<&str>,
    id: Option<&str>,
) -> Option<ResolvedTarget> {
    let (nx, ny) = if let Some(eid) = id {
        find_id_in_dump(dump, eid)?
    } else if let Some(lab) = label {
        find_label_in_dump(dump, lab)?
    } else {
        return None;
    };
    let node = find_node(dump, label, id);
    let name = id.or(label).unwrap_or("?").to_string();
    Some(ResolvedTarget {
        nx,
        ny,
        name,
        hittable: node.map(node_hittable).unwrap_or(true),
    })
}

/// True when overlay likely occludes this target (norm coords).
fn occluded(target: &ResolvedTarget, overlay: Overlay) -> bool {
    match overlay {
        Overlay::None => false,
        Overlay::Transition => true,
        Overlay::Alert | Overlay::Sheet => true,
        // Soft keyboard typically owns the lower ~45% of the screen.
        Overlay::Keyboard => !target.hittable || target.ny > 0.52,
    }
}

fn clear_overlay(
    overlay: Overlay,
    udid: &str,
    w: f64,
    h: f64,
) -> bool {
    match overlay {
        Overlay::None => true,
        Overlay::Transition => {
            std::thread::sleep(Duration::from_millis(120));
            true
        }
        Overlay::Keyboard => {
            let _ = HidInput::key_named(udid, "return");
            std::thread::sleep(Duration::from_millis(60));
            // Resign first responder via upper chrome tap (nav / status band).
            let _ = HidInput::tap(udid, 0.5, 0.10, w, h);
            std::thread::sleep(Duration::from_millis(120));
            true
        }
        Overlay::Alert | Overlay::Sheet => {
            // Escape / home is too destructive for app jobs — report blocked.
            false
        }
    }
}

/// Ensure a clear motor path to `label`/`id`: settle overlays until target is hittable.
pub(crate) fn ensure_path(
    build: &dyn Fn() -> ObserveSnapshot,
    udid: &str,
    w: f64,
    h: f64,
    label: Option<&str>,
    id: Option<&str>,
    timeout: Duration,
) -> Result<(ResolvedTarget, ObserveSnapshot), CapabilityResult> {
    let t0 = Instant::now();
    let mut last_overlay = Overlay::None;
    let mut last_snap = build();
    while t0.elapsed() < timeout {
        let dump = match AxDump::dump(udid) {
            Ok(d) => d,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(40));
                continue;
            }
        };
        last_snap = settle_eyes(build, 200);
        last_overlay = overlay_from_snapshot(&last_snap);
        if let Some(target) = resolve_from_dump(&dump, label, id) {
            if !occluded(&target, last_overlay) {
                return Ok((target, last_snap));
            }
            if !clear_overlay(last_overlay, udid, w, h) {
                return Err(CapabilityResult::fail(
                    FaultClass::Blocked,
                    phase_of(&last_snap),
                    surface_of(&last_snap),
                    "ensure_path",
                    json!({
                        "overlay": last_overlay.as_str(),
                        "target": target.name,
                        "error": "overlay cannot be cleared by motor"
                    }),
                    Some(last_snap),
                ));
            }
        } else if last_overlay.blocks_path() {
            let _ = clear_overlay(last_overlay, udid, w, h);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Err(CapabilityResult::fail(
        if last_overlay.blocks_path() {
            FaultClass::Blocked
        } else {
            FaultClass::TargetMissing
        },
        phase_of(&last_snap),
        surface_of(&last_snap),
        "ensure_path",
        json!({
            "overlay": last_overlay.as_str(),
            "label": label,
            "id": id,
            "error": "timeout waiting for clear path"
        }),
        Some(last_snap),
    ))
}

/// Tap through the motor pipeline.
pub(crate) fn motor_tap(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(2000), 3);
    if !ready.ok {
        return ready;
    }
    let (udid, w, h) = {
        let st = state.lock().unwrap();
        match st.current_udid() {
            Ok(u) => (u, st.sim_width, st.sim_height),
            Err(e) => {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Dead,
                    None,
                    "act_tap",
                    json!({ "error": e }),
                    ready.observe,
                );
            }
        }
    };
    let (target, _) = match ensure_path(
        build,
        &udid,
        w,
        h,
        label,
        id,
        Duration::from_millis(timeout_ms),
    ) {
        Ok(v) => v,
        Err(e) => {
            return CapabilityResult::fail(
                e.fault,
                e.phase,
                e.surface,
                "act_tap",
                e.detail.unwrap_or(json!({})),
                e.observe,
            );
        }
    };
    if let Err(e) = HidInput::tap(&udid, target.nx, target.ny, w, h) {
        return CapabilityResult::fail(
            FaultClass::MotorRejected,
            SessionPhase::Degraded,
            None,
            "act_tap",
            json!({ "error": e.to_string(), "target": target.name }),
            Some(build()),
        );
    }
    state
        .lock()
        .unwrap()
        .push_action_result(true, "act_tap", json!({ "target": target.name }));
    let snap = settle_eyes(build, settle_ms);
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "act_tap",
        json!({ "target": target.name, "motor": "ensure_path" }),
        Some(snap),
    )
}

/// Type through motor: ready → fire type (keyboard may rise — that is intentional).
/// Clearing the keyboard is the *next* act's ensure_path job.
pub(crate) fn motor_type(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    text: &str,
    settle_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(1500), 2);
    if !ready.ok {
        return ready;
    }
    let udid = match state.lock().unwrap().current_udid() {
        Ok(u) => u,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Dead,
                None,
                "act_type",
                json!({ "error": e }),
                ready.observe,
            );
        }
    };
    if let Err(e) = HidInput::type_text(&udid, text) {
        return CapabilityResult::fail(
            FaultClass::MotorRejected,
            SessionPhase::Degraded,
            ready.surface.clone(),
            "act_type",
            json!({ "error": e.to_string() }),
            ready.observe,
        );
    }
    {
        let mut st = state.lock().unwrap();
        st.push_action_result(true, "act_type", json!({ "text": text }));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        st.sense_buf.push(ligh_core::SenseEvent {
            t: now,
            kind: "typed".into(),
            payload: Some(json!({ "verified": "host_accepted", "text": text })),
        });
    }
    let snap = settle_eyes(build, settle_ms);
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "act_type",
        json!({ "text": text, "verified": "host_accepted", "motor": "type_raises_overlay_ok" }),
        Some(snap),
    )
}

/// Wait until label/id is on a clear path (resolve + settle overlay).
pub(crate) fn motor_wait(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(2000), 3);
    if !ready.ok {
        return ready;
    }
    let (udid, w, h) = {
        let st = state.lock().unwrap();
        match st.current_udid() {
            Ok(u) => (u, st.sim_width, st.sim_height),
            Err(e) => {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Dead,
                    None,
                    "wait",
                    json!({ "error": e }),
                    ready.observe,
                );
            }
        }
    };
    match ensure_path(
        build,
        &udid,
        w,
        h,
        label,
        id,
        Duration::from_millis(timeout_ms),
    ) {
        Ok((target, snap)) => CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "wait",
            json!({ "target": target.name, "motor": "ensure_path" }),
            Some(snap),
        ),
        Err(e) => CapabilityResult::fail(
            e.fault,
            e.phase,
            e.surface,
            "wait",
            e.detail.unwrap_or(json!({})),
            e.observe,
        ),
    }
}
