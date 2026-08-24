//! Physical motor cascade — DevDriver fast path, WDA fallback, effect verify.
//!
//! Order (Ennio-style): in-app semantic activate → in-app tap → WDA tap → WDA label.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::LighError;
use ligh_host::{AxDump, PhysicalUi};
use serde_json::json;

use crate::device_hub::DeviceHub;
use crate::wda::WdaArms;
use crate::{build_observe_once, DaemonState};

const EFFECT_POLL_MS: u64 = 40;
const EFFECT_BUDGET_MS: u64 = 480;

pub(crate) fn wait_effect_change(
    state: &Arc<Mutex<DaemonState>>,
    before_sig: &str,
    budget_ms: u64,
) -> Option<String> {
    if before_sig.is_empty() {
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(EFFECT_POLL_MS));
        let after = build_observe_once(state, true)
            .screen_sig
            .unwrap_or_default();
        if !after.is_empty() && after != before_sig {
            return Some(after);
        }
    }
    None
}

fn ensure_wda(arms: &WdaArms, hub: &DeviceHub) -> Result<(), LighError> {
    crate::wda::load_wda_dotenv();
    let udid = std::env::var("LIGH_WDA_UDID").unwrap_or_default();
    if udid.is_empty() {
        return Err(LighError::NotReady(
            "WDA fallback needs LIGH_WDA_UDID (~/.ligh/wda.env)".into(),
        ));
    }
    let bundle = hub.bundle_id_hint().or_else(|| std::env::var("LIGH_WDA_BUNDLE").ok());
    arms.ensure(&udid, bundle.as_deref())
}

pub(crate) fn tap_label(
    state: &Arc<Mutex<DaemonState>>,
    hub: &DeviceHub,
    arms: &WdaArms,
    udid: &str,
    label: &str,
    id: Option<&str>,
    w: f64,
    h: f64,
    timeout_ms: u64,
) -> Result<serde_json::Value, LighError> {
    let before_sig = build_observe_once(state, true)
        .screen_sig
        .unwrap_or_default();
    let t0 = Instant::now();

    if hub.active() {
        let activated = hub.activate_label(label).is_ok() || hub.press_label(label).is_ok();
        if activated {
            if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS) {
                return Ok(json!({
                    "label": label,
                    "id": id,
                    "motor": "devdriver_activate",
                    "effect": "ok",
                    "before_sig": before_sig,
                    "after_sig": after,
                    "waited_ms": t0.elapsed().as_secs_f64() * 1000.0,
                }));
            }
        }
    }

    let timeout = Duration::from_millis(timeout_ms);
    let resolved = if let Some(eid) = id {
        AxDump::wait_id(udid, eid, timeout)
            .or_else(|_| AxDump::wait_label(udid, label, timeout))
    } else {
        AxDump::wait_label(udid, label, timeout)
    };

    if let Ok((nx, ny, waited)) = resolved {
        if hub.active() && hub.tap(nx, ny, w, h).is_ok() {
            if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS) {
                return Ok(json!({
                    "label": label,
                    "id": id,
                    "x": nx,
                    "y": ny,
                    "motor": "devdriver_tap",
                    "effect": "ok",
                    "before_sig": before_sig,
                    "after_sig": after,
                    "waited_ms": waited.as_secs_f64() * 1000.0,
                }));
            }
        }

        if ensure_wda(arms, hub).is_ok() && arms.tap_norm(nx, ny).is_ok() {
            if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS + 200) {
                return Ok(json!({
                    "label": label,
                    "id": id,
                    "x": nx,
                    "y": ny,
                    "motor": "wda_tap",
                    "effect": "ok",
                    "before_sig": before_sig,
                    "after_sig": after,
                    "waited_ms": waited.as_secs_f64() * 1000.0,
                }));
            }
        }
    }

    if ensure_wda(arms, hub).is_ok() && arms.click_label(label).is_ok() {
        if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS + 200) {
            return Ok(json!({
                "label": label,
                "id": id,
                "motor": "wda_label",
                "effect": "ok",
                "before_sig": before_sig,
                "after_sig": after,
                "waited_ms": t0.elapsed().as_secs_f64() * 1000.0,
            }));
        }
    }

    Err(LighError::NotReady(
        "physical tap cascade exhausted — no UI effect (devdriver + WDA)".into(),
    ))
}

pub(crate) fn tap_coord(
    state: &Arc<Mutex<DaemonState>>,
    hub: &DeviceHub,
    arms: &WdaArms,
    nx: f64,
    ny: f64,
    w: f64,
    h: f64,
) -> Result<serde_json::Value, LighError> {
    let before_sig = build_observe_once(state, true)
        .screen_sig
        .unwrap_or_default();

    if hub.active() && hub.tap(nx, ny, w, h).is_ok() {
        if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS) {
            return Ok(json!({
                "x": nx,
                "y": ny,
                "motor": "devdriver_tap",
                "effect": "ok",
                "before_sig": before_sig,
                "after_sig": after,
            }));
        }
    }

    if ensure_wda(arms, hub).is_ok() && arms.tap_norm(nx, ny).is_ok() {
        if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS + 200) {
            return Ok(json!({
                "x": nx,
                "y": ny,
                "motor": "wda_tap",
                "effect": "ok",
                "before_sig": before_sig,
                "after_sig": after,
            }));
        }
    }

    Err(LighError::NotReady(
        "physical coordinate tap had no UI effect".into(),
    ))
}

pub(crate) fn swipe(
    state: &Arc<Mutex<DaemonState>>,
    hub: &DeviceHub,
    arms: &WdaArms,
    fnx: f64,
    fny: f64,
    tnx: f64,
    tny: f64,
    w: f64,
    h: f64,
) -> Result<serde_json::Value, LighError> {
    let before_sig = build_observe_once(state, true)
        .screen_sig
        .unwrap_or_default();

    if hub.active()
        && hub
            .swipe(fnx, fny, tnx, tny, w, h)
            .is_ok()
    {
        if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS) {
            return Ok(json!({
                "motor": "devdriver_swipe",
                "effect": "ok",
                "before_sig": before_sig,
                "after_sig": after,
            }));
        }
    }

    if ensure_wda(arms, hub).is_ok()
        && arms
            .swipe_norm(fnx, fny, tnx, tny, 320.0)
            .is_ok()
    {
        if let Some(after) = wait_effect_change(state, &before_sig, EFFECT_BUDGET_MS + 200) {
            return Ok(json!({
                "motor": "wda_swipe",
                "effect": "ok",
                "before_sig": before_sig,
                "after_sig": after,
            }));
        }
    }

    if before_sig.is_empty() {
        return Ok(json!({ "motor": "swipe", "effect": "unknown" }));
    }

    Err(LighError::NotReady(
        "physical swipe had no UI effect".into(),
    ))
}
