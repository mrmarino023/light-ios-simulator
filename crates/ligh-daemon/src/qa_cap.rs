//! QA-layer capabilities: perceive, attempt, find, dismiss.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    build_feel, build_perceive, evaluate_attempt, feel_agent_view, overlay_from_snapshot,
    parse_expectation, CapabilityResult, Expectation, FaultClass, ObserveSnapshot, Overlay,
    PerceiveView, SessionPhase,
};
use ligh_host::{AxDump, HidInput};
use serde_json::json;

use crate::capabilities::{ensure_ready, phase_of, settle_eyes, surface_of};
use crate::ux_cap::{ux_persist_attempt, ux_persist_perceive};
use crate::DaemonState;

pub(crate) fn perceive_from_snap(snap: &ObserveSnapshot) -> PerceiveView {
    build_perceive(snap, &snap.events)
}

pub(crate) fn cap_perceive(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
    workspace: Option<&std::path::Path>,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms, 4);
    if !ready.ok {
        return CapabilityResult::fail(
            ready.fault,
            ready.phase,
            ready.surface,
            "perceive",
            json!({ "recovered": false, "reason": "ensure_ready failed" }),
            ready.observe,
        );
    }
    let snap = ready
        .observe
        .clone()
        .unwrap_or_else(|| settle_eyes(build, settle_ms));
    let view = perceive_from_snap(&snap);
    let feel = build_feel(&view, &snap, None, Some(settle_ms));
    ux_persist_perceive(workspace, &view);
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "perceive",
        json!({
            "perceive": view,
            "feel": feel_agent_view(&feel),
        }),
        Some(snap),
    )
}

pub(crate) fn cap_attempt(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    intent: &str,
    label: Option<&str>,
    id: Option<&str>,
    text: Option<&str>,
    key: Option<&str>,
    expect: Option<&Expectation>,
    settle_ms: u64,
    timeout_ms: u64,
    workspace: Option<&std::path::Path>,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(2000), 3);
    if !ready.ok {
        return ready;
    }
    let pre = ready
        .observe
        .clone()
        .unwrap_or_else(|| settle_eyes(build, settle_ms));

    let motor = match intent {
        "tap" => crate::motor::motor_tap(build, state, label, id, settle_ms, timeout_ms, None, None),
        "type" => {
            let t = text.unwrap_or("");
            crate::motor::motor_type(build, state, t, label, id, settle_ms, timeout_ms, ligh_core::MotorTypeStrategy::FocusHid)
        }
        "key" => cap_key(build, state, key.unwrap_or("return"), settle_ms),
        other => {
            return CapabilityResult::fail(
                FaultClass::Model,
                phase_of(&pre),
                surface_of(&pre),
                "attempt",
                json!({ "error": format!("unknown intent: {other}"), "allowed": ["tap","type","key"] }),
                Some(pre),
            );
        }
    };

    let motor_ok = motor.ok;
    let post = if motor_ok {
        motor
            .observe
            .clone()
            .unwrap_or_else(|| settle_eyes(build, settle_ms))
    } else {
        // Motor effect detector can miss overlay opens; re-settle before verdict.
        settle_eyes(build, settle_ms.max(2500))
    };

    // Re-attach events on post snapshot for delta (caller should have enriched via attach_sense).
    let verdict = evaluate_attempt(
        intent,
        motor_ok,
        &pre,
        &post,
        &post.events,
        expect,
        id,
        label,
    );
    let pre_view = build_perceive(&pre, &pre.events);
    ux_persist_attempt(workspace, &pre_view, &verdict, label, id, text);
    let feel_after = build_feel(
        &verdict.perceive_after,
        &post,
        Some(pre_view.location.fingerprint.as_str()),
        Some(settle_ms),
    );

    let fault = if verdict.intent_met {
        FaultClass::Ok
    } else if !motor_ok {
        motor.fault
    } else {
        FaultClass::Model
    };

    if verdict.intent_met {
        CapabilityResult::success(
            phase_of(&post),
            surface_of(&post),
            "attempt",
            json!({
                "verdict": verdict,
                "feel": feel_agent_view(&feel_after),
                "motor_detail": motor.detail,
            }),
            Some(post),
        )
    } else {
        CapabilityResult::fail(
            fault,
            phase_of(&post),
            surface_of(&post),
            "attempt",
            json!({
                "verdict": verdict,
                "feel": feel_agent_view(&feel_after),
                "motor_detail": motor.detail,
                "motor_fault": motor.fault.as_str(),
            }),
            Some(post),
        )
    }
}

fn cap_key(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    key: &str,
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
                "attempt",
                json!({ "error": e }),
                ready.observe,
            );
        }
    };
    if let Err(e) = HidInput::key_named(&udid, key) {
        return CapabilityResult::fail(
            FaultClass::MotorRejected,
            phase_of(&ready.observe.as_ref().unwrap_or(&build())),
            ready.surface.clone(),
            "attempt",
            json!({ "error": e.to_string(), "key": key }),
            ready.observe,
        );
    }
    state.lock().unwrap().push_action_result(
        true,
        "attempt_key",
        json!({ "key": key }),
    );
    let snap = settle_eyes(build, settle_ms);
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "attempt",
        json!({ "key": key, "motor": "key" }),
        Some(snap),
    )
}

pub(crate) fn cap_find(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    scroll: bool,
    settle_ms: u64,
    timeout_ms: u64,
    max_swipes: u32,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms, 3);
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
                "find",
                json!({ "error": e }),
                ready.observe,
            );
        }
    };

    let exists = |udid: &str| -> bool {
        if let Some(eid) = id {
            if AxDump::exists_id(udid, eid).unwrap_or(false) {
                return true;
            }
        }
        if let Some(lab) = label {
            if AxDump::exists_label(udid, lab).unwrap_or(false) {
                return true;
            }
        }
        false
    };

    if exists(&udid) {
        let snap = settle_eyes(build, settle_ms);
        let view = perceive_from_snap(&snap);
        return CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "find",
            json!({ "found": true, "swipes": 0, "perceive": view }),
            Some(snap),
        );
    }

    if !scroll {
        let snap = settle_eyes(build, settle_ms);
        return CapabilityResult::fail(
            FaultClass::TargetMissing,
            phase_of(&snap),
            surface_of(&snap),
            "find",
            json!({ "found": false, "scrolled": false, "label": label, "id": id }),
            Some(snap),
        );
    }

    let (w, h) = {
        let st = state.lock().unwrap();
        (st.sim_width, st.sim_height)
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut swipes = 0u32;
    while Instant::now() < deadline && swipes < max_swipes {
        if exists(&udid) {
            let snap = settle_eyes(build, settle_ms);
            let view = perceive_from_snap(&snap);
            return CapabilityResult::success(
                phase_of(&snap),
                surface_of(&snap),
                "find",
                json!({ "found": true, "swipes": swipes, "perceive": view }),
                Some(snap),
            );
        }
        if let Err(e) = HidInput::swipe(&udid, 0.5, 0.72, 0.5, 0.28, w, h) {
            return CapabilityResult::fail(
                FaultClass::MotorRejected,
                SessionPhase::Degraded,
                None,
                "find",
                json!({ "error": e.to_string(), "swipes": swipes }),
                Some(build()),
            );
        }
        swipes += 1;
        std::thread::sleep(Duration::from_millis(280));
    }

    let snap = settle_eyes(build, settle_ms);
    CapabilityResult::fail(
        FaultClass::TargetMissing,
        phase_of(&snap),
        surface_of(&snap),
        "find",
        json!({ "found": false, "swipes": swipes, "label": label, "id": id }),
        Some(snap),
    )
}

pub(crate) fn cap_dismiss(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms, 3);
    if !ready.ok {
        return ready;
    }
    let snap0 = ready
        .observe
        .clone()
        .unwrap_or_else(|| settle_eyes(build, settle_ms));
    let overlay = overlay_from_snapshot(&snap0);
    if overlay == Overlay::None {
        let view = perceive_from_snap(&snap0);
        return CapabilityResult::success(
            phase_of(&snap0),
            surface_of(&snap0),
            "dismiss",
            json!({ "dismissed": false, "reason": "no overlay", "perceive": view }),
            Some(snap0),
        );
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
                    "dismiss",
                    json!({ "error": e }),
                    Some(snap0),
                );
            }
        }
    };

    let mut strategy = overlay.as_str();
    let cleared = match overlay {
        Overlay::None => true,
        Overlay::Keyboard => {
            let _ = HidInput::key_named(&udid, "return");
            std::thread::sleep(Duration::from_millis(60));
            let _ = HidInput::tap(&udid, 0.5, 0.10, w, h);
            std::thread::sleep(Duration::from_millis(150));
            true
        }
        Overlay::Transition => {
            std::thread::sleep(Duration::from_millis(200));
            true
        }
        Overlay::Alert => {
            strategy = "tap_ok";
            for lab in ["OK", "Ok", "Allow", "Consenti", "Fine", "Done"] {
                if AxDump::exists_label(&udid, lab).unwrap_or(false) {
                    if let Ok((x, y, _)) =
                        AxDump::wait_label(&udid, lab, Duration::from_millis(800))
                    {
                        let _ = HidInput::tap(&udid, x, y, w, h);
                        std::thread::sleep(Duration::from_millis(200));
                        break;
                    }
                }
            }
            true
        }
        Overlay::Sheet => {
            strategy = "swipe_down";
            let _ = HidInput::swipe(&udid, 0.5, 0.35, 0.5, 0.85, w, h);
            std::thread::sleep(Duration::from_millis(250));
            true
        }
        Overlay::SystemSurface => {
            let role = snap0.system_surface.as_ref().map(|s| s.role);
            let policy = ligh_core::policy_for_overlay(overlay, role);
            if policy.auto_dismiss {
                strategy = "swipe_down";
                let _ = HidInput::swipe(&udid, 0.5, 0.35, 0.5, 0.85, w, h);
                std::thread::sleep(Duration::from_millis(250));
                true
            } else {
                // Auth / permission — agent must interact inside; never swipe away.
                strategy = "refuse_system_surface";
                false
            }
        }
    };

    let snap = settle_eyes(build, settle_ms);
    let after = overlay_from_snapshot(&snap);
    let view = perceive_from_snap(&snap);
    let dismissed = after == Overlay::None;

    if dismissed {
        CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "dismiss",
            json!({
                "dismissed": true,
                "was": overlay.as_str(),
                "strategy": strategy,
                "cleared_attempt": cleared,
                "perceive": view,
            }),
            Some(snap),
        )
    } else {
        CapabilityResult::fail(
            FaultClass::Blocked,
            phase_of(&snap),
            surface_of(&snap),
            "dismiss",
            json!({
                "dismissed": false,
                "was": overlay.as_str(),
                "still": after.as_str(),
                "strategy": strategy,
                "perceive": view,
            }),
            Some(snap),
        )
    }
}
