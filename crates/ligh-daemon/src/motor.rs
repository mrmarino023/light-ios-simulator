//! Unified motor: ready → resolve → ensure_path → fire → settle.
//!
//! This is the architectural spine for app-under-test automation.
//! Overlays are cleared here — never as one-off side effects on type/tap.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    build_actionable_topk, find_hittable_label_in_dump, find_id_in_dump,
    find_label_in_dump, find_onscreen_id_in_dump, is_editable_role, overlay_from_snapshot,
    rank_candidates, ActionOutcome, CapabilityResult, FaultClass, MotorTypeStrategy,
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

/// Heuristic: SwiftUI sheet presented (overlay detection misses some sheets).
fn snap_on_sheet(snap: &ObserveSnapshot) -> bool {
    if overlay_from_snapshot(snap) == Overlay::Sheet
        || overlay_from_snapshot(snap) == Overlay::SystemSurface
    {
        return true;
    }
    if let Some(scene) = &snap.scene {
        if scene
            .screen_title
            .as_deref()
            .is_some_and(|t| t.to_ascii_lowercase().contains("sheet"))
        {
            return true;
        }
        if !scene.sheets.is_empty() {
            return true;
        }
    }
    let nodes = snap.accessibility_tree.nodes();
    let has_sheet_title = nodes.iter().any(|n| {
        n.get("identifier")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == "SheetTitle")
    });
    let has_sheet_btn = nodes.iter().any(|n| {
        n.get("identifier")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == "ConfirmAction" || id == "CancelSheet")
    });
    has_sheet_title && has_sheet_btn
}

fn id_still_actionable(snap: &ObserveSnapshot, id: &str) -> bool {
    snap.accessibility_tree.nodes().iter().any(|n| {
        ligh_core::node_matches_identifier(n, id) && ligh_core::node_viewport_hittable(n)
    })
}

/// True when a tap likely changed UI (not just HID ack).
pub(crate) fn tap_effect_observed(
    before: &ObserveSnapshot,
    after: &ObserveSnapshot,
    id: Option<&str>,
) -> bool {
    if overlay_from_snapshot(before) != overlay_from_snapshot(after) {
        return true;
    }
    let bt = before
        .scene
        .as_ref()
        .and_then(|s| s.screen_title.as_deref())
        .unwrap_or("");
    let at = after
        .scene
        .as_ref()
        .and_then(|s| s.screen_title.as_deref())
        .unwrap_or("");
    if bt != at {
        return true;
    }
    if snap_on_sheet(before) && !snap_on_sheet(after) {
        return true;
    }
    if let Some(eid) = id {
        if id_still_actionable(before, eid) && !id_still_actionable(after, eid) {
            return true;
        }
        let was_focused = target_focused_editable(before, None, Some(eid));
        let now_focused = target_focused_editable(after, None, Some(eid));
        if !was_focused && now_focused {
            return true;
        }
        let before_ids: std::collections::HashSet<_> = before
            .accessibility_tree
            .nodes()
            .iter()
            .filter_map(|n| n.get("identifier").and_then(|v| v.as_str()))
            .collect();
        let after_ids: std::collections::HashSet<_> = after
            .accessibility_tree
            .nodes()
            .iter()
            .filter_map(|n| n.get("identifier").and_then(|v| v.as_str()))
            .collect();
        if before_ids != after_ids {
            return true;
        }
    }
    after.events.len() > before.events.len()
}

fn node_hittable(n: &serde_json::Value) -> bool {
    n.get("hittable")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn node_matches_target(n: &serde_json::Value, label: Option<&str>, id: Option<&str>) -> bool {
    if let Some(eid) = id {
        if n.get("identifier").and_then(|v| v.as_str()) == Some(eid)
            || n.get("id").and_then(|v| v.as_str()) == Some(eid)
        {
            return true;
        }
    }
    if let Some(lab) = label {
        let needle = lab.to_ascii_lowercase();
        if n.get("label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase().contains(&needle))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn node_editable(n: &serde_json::Value) -> bool {
    if node_is_keyboard(n) {
        return false;
    }
    n.get("traits")
        .and_then(|v| v.as_str())
        .is_some_and(|t| t.contains("editable"))
        || n.get("role")
            .and_then(|v| v.as_str())
            .is_some_and(|r| r.contains("TextField") || r.contains("TextArea"))
}

fn node_is_keyboard(n: &serde_json::Value) -> bool {
    n.get("role")
        .and_then(|v| v.as_str())
        .is_some_and(|r| r.contains("Keyboard"))
        || n.get("identifier")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id.to_ascii_lowercase().contains("keyboard"))
}

fn snap_node_sources<'a>(
    snap: &'a ObserveSnapshot,
) -> impl Iterator<Item = &'a serde_json::Value> {
    snap.accessibility_tree
        .nodes()
        .iter()
        .chain(snap.actionable_topk.iter())
}

fn target_focused_editable(
    snap: &ObserveSnapshot,
    label: Option<&str>,
    id: Option<&str>,
) -> bool {
    snap_node_sources(snap).any(|n| {
        node_matches_target(n, label, id)
            && n.get("focused").and_then(|v| v.as_bool()) == Some(true)
            && node_editable(n)
    })
}

fn any_focused_editable(snap: &ObserveSnapshot) -> bool {
    snap_node_sources(snap).any(|n| {
        n.get("focused").and_then(|v| v.as_bool()) == Some(true) && node_editable(n)
    })
}

fn focus_gained(
    before: &ObserveSnapshot,
    after: &ObserveSnapshot,
    label: Option<&str>,
    id: Option<&str>,
) -> bool {
    !target_focused_editable(before, label, id) && target_focused_editable(after, label, id)
}

pub(crate) fn target_onscreen_udid(
    udid: &str,
    label: Option<&str>,
    id: Option<&str>,
) -> bool {
    let Ok(dump) = AxDump::dump(udid) else {
        return false;
    };
    if let Some(eid) = id {
        return find_onscreen_id_in_dump(&dump, eid).is_some();
    }
    if let Some(lab) = label {
        return find_hittable_label_in_dump(&dump, lab).is_some()
            || find_label_in_dump(&dump, lab).is_some();
    }
    false
}

fn find_node<'a>(
    dump: &'a serde_json::Value,
    label: Option<&str>,
    id: Option<&str>,
) -> Option<&'a serde_json::Value> {
    let nodes = dump_nodes(dump)?;
    let mut hits: Vec<&serde_json::Value> = if let Some(eid) = id {
        nodes
            .iter()
            .filter(|n| {
                n.get("identifier").and_then(|v| v.as_str()) == Some(eid)
                    || n.get("id").and_then(|v| v.as_str()) == Some(eid)
            })
            .collect()
    } else if let Some(lab) = label {
        let needle = lab.to_ascii_lowercase();
        nodes
            .iter()
            .filter(|n| {
                n.get("label")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
                    || n.get("identifier")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_ascii_lowercase().contains(&needle))
                        .unwrap_or(false)
            })
            .collect()
    } else {
        Vec::new()
    };
    if hits.is_empty() {
        return None;
    }
    hits.sort_by_key(|n| {
        let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let editable = node_editable(n) || is_editable_role(role);
        let focused = n.get("focused").and_then(|v| v.as_bool()).unwrap_or(false);
        (u8::from(!editable), u8::from(!focused))
    });
    Some(hits[0])
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
        Overlay::Alert | Overlay::Sheet | Overlay::SystemSurface => !target.hittable,
        // Soft keyboard typically owns the lower ~45% of the screen.
        Overlay::Keyboard => !target.hittable || target.ny > 0.52,
    }
}

fn slim_topk(snap: &ObserveSnapshot, k: usize) -> Vec<serde_json::Value> {
    if !snap.actionable_topk.is_empty() {
        return snap.actionable_topk.iter().take(k).cloned().collect();
    }
    build_actionable_topk(snap.accessibility_tree.nodes(), k)
}

fn fault_evidence(
    snap: &ObserveSnapshot,
    udid: &str,
    label: Option<&str>,
    id: Option<&str>,
    overlay: Overlay,
    error: &str,
) -> serde_json::Value {
    let mut detail = json!({
        "overlay": overlay.as_str(),
        "label": label.unwrap_or(""),
        "id": id.unwrap_or(""),
        "error": error,
        "wanted": { "id": id, "label": label },
    });
    if let Ok(dump) = AxDump::dump(udid) {
        if let Some(nodes) = dump_nodes(&dump) {
            detail["candidates"] = json!(rank_candidates(nodes, id, label, 8));
        }
    }
    detail["actionable_topk"] = json!(slim_topk(snap, 15));
    if let Some(scene) = &snap.scene {
        detail["scene"] = json!(scene);
    }
    detail
}

pub(crate) fn attach_probes(detail: &mut serde_json::Value, probes: &[crate::cognition::ProbeEntry]) {
    if !probes.is_empty() {
        detail["probes_tried"] = json!(probes);
        detail["suggestion"] = json!("Host tried probes — re-observe or fix a11y; see probes_tried");
    }
}

fn try_dismiss_modal(udid: &str, w: f64, h: f64) -> bool {
    let Ok(dump) = AxDump::dump(udid) else {
        return false;
    };
    const NEEDLES: &[&str] = &[
        "CancelSheet", "Cancel", "Close", "Dismiss", "Not Now", "Skip", "Annulla", "Chiudi",
    ];
    for needle in NEEDLES {
        if let Some((nx, ny)) = find_label_in_dump(&dump, needle) {
            let _ = HidInput::tap(udid, nx, ny, w, h);
            std::thread::sleep(Duration::from_millis(220));
            return true;
        }
        if let Some((nx, ny)) = find_id_in_dump(&dump, needle) {
            let _ = AxDump::press_id(udid, needle).or_else(|_| HidInput::tap(udid, nx, ny, w, h));
            std::thread::sleep(Duration::from_millis(220));
            return true;
        }
    }
    let _ = HidInput::swipe(udid, 0.5, 0.32, 0.5, 0.78, w, h);
    std::thread::sleep(Duration::from_millis(280));
    true
}

fn surface_role(snap: &ObserveSnapshot) -> Option<ligh_core::SystemSurfaceRole> {
    snap.system_surface.as_ref().map(|s| s.role)
}

fn clear_overlay(
    overlay: Overlay,
    udid: &str,
    w: f64,
    h: f64,
    system_role: Option<ligh_core::SystemSurfaceRole>,
) -> bool {
    match overlay {
        Overlay::None => true,
        Overlay::Transition => {
            std::thread::sleep(Duration::from_millis(120));
            true
        }
        Overlay::Keyboard => {
            let _ = HidInput::key_named(udid, "return");
            std::thread::sleep(Duration::from_millis(80));
            let _ = HidInput::tap(udid, 0.5, 0.08, w, h);
            std::thread::sleep(Duration::from_millis(150));
            let _ = HidInput::key_named(udid, "escape");
            std::thread::sleep(Duration::from_millis(80));
            true
        }
        Overlay::Alert | Overlay::Sheet | Overlay::SystemSurface => {
            let policy = ligh_core::policy_for_overlay(overlay, system_role);
            if !policy.auto_dismiss {
                false
            } else {
                try_dismiss_modal(udid, w, h)
            }
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
            if !clear_overlay(last_overlay, udid, w, h, surface_role(&last_snap)) {
                return Err(CapabilityResult::fail(
                    FaultClass::Blocked,
                    phase_of(&last_snap),
                    surface_of(&last_snap),
                    "ensure_path",
                    fault_evidence(
                        &last_snap,
                        udid,
                        label,
                        id,
                        last_overlay,
                        "overlay cannot be cleared by motor",
                    ),
                    Some(last_snap),
                ));
            }
        } else if last_overlay.blocks_path()
            && !matches!(
                last_overlay,
                Overlay::Sheet | Overlay::Alert | Overlay::SystemSurface
            )
        {
            let _ = clear_overlay(last_overlay, udid, w, h, surface_role(&last_snap));
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
        fault_evidence(
            &last_snap,
            udid,
            label,
            id,
            last_overlay,
            "timeout waiting for clear path",
        ),
        Some(last_snap),
    ))
}

/// Try fire strategies until UI changes or all exhausted.
fn motor_fire_verified(
    build: &dyn Fn() -> ObserveSnapshot,
    udid: &str,
    target: &ResolvedTarget,
    w: f64,
    h: f64,
    id: Option<&str>,
    label: Option<&str>,
    overlay: Overlay,
    before: &ObserveSnapshot,
    settle_ms: u64,
) -> Result<(&'static str, ObserveSnapshot), CapabilityResult> {
    if target_focused_editable(before, label, id) {
        return Ok(("already_focused", before.clone()));
    }
    let mut strategies: Vec<(&str, Box<dyn Fn() -> bool>)> = Vec::new();
    let prefer_ax = {
        let role = before.system_surface.as_ref().map(|s| s.role);
        ligh_core::policy_for_overlay(overlay, role).prefer_ax || snap_on_sheet(before)
    };
    let u = udid.to_string();
    let t = target.clone();
    let tid = id.map(|s| s.to_string());
    let tlab = label.map(|s| s.to_string());

    // Physical: AX activate first. Coordinate HID hits RN glyph views, not onPress.
    let prefer_ax = prefer_ax || ligh_host::physical_ui_active();
    if prefer_ax {
        if id.is_some() {
            let u2 = u.clone();
            let eid = tid.clone().unwrap();
            strategies.push((
                "ax_press_id",
                Box::new(move || AxDump::press_id(&u2, &eid).is_ok()),
            ));
        }
        if label.is_some() {
            let u2 = u.clone();
            let lab = tlab.clone().unwrap();
            strategies.push((
                "ax_press_label",
                Box::new(move || AxDump::press_label(&u2, &lab).is_ok()),
            ));
        }
    }
    {
        let u2 = u.clone();
        let tc = t.clone();
        strategies.push((
            "hid_tap",
            Box::new(move || HidInput::tap(&u2, tc.nx, tc.ny, w, h).is_ok()),
        ));
    }
    {
        let u2 = u.clone();
        let tc = t.clone();
        strategies.push((
            "hid_hold",
            Box::new(move || HidInput::tap_hold(&u2, tc.nx, tc.ny, w, h, 180.0).is_ok()),
        ));
    }
    if id.is_some() {
        let u2 = u.clone();
        let eid = tid.clone().unwrap();
        strategies.push((
            "ax_press_fallback",
            Box::new(move || AxDump::press_id(&u2, &eid).is_ok()),
        ));
    }

    let mut last_snap = before.clone();
    let mut last_method = "none";
    let effect_deadline = Duration::from_millis(settle_ms.max(1800));
    for (name, fire) in strategies {
        if !fire() {
            continue;
        }
        last_method = name;
        let t_effect = Instant::now();
        loop {
            last_snap = settle_eyes(build, 280);
            if tap_effect_observed(before, &last_snap, id)
                || focus_gained(before, &last_snap, label, id)
            {
                return Ok((last_method, last_snap));
            }
            if t_effect.elapsed() >= effect_deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    let final_snap = settle_eyes(build, settle_ms.min(1500));
    if tap_effect_observed(before, &final_snap, id) || focus_gained(before, &final_snap, label, id) {
        return Ok(("delayed_effect", final_snap));
    }

    Err(CapabilityResult::fail(
        FaultClass::MotorNoEffect,
        phase_of(&last_snap),
        surface_of(&last_snap),
        "act_tap",
        fault_evidence(
            &last_snap,
            udid,
            label,
            id,
            overlay_from_snapshot(&last_snap),
            "fire succeeded but UI unchanged (motor_no_effect)",
        ),
        Some(last_snap),
    ))
}

fn node_center_norm(n: &serde_json::Value) -> Option<(f64, f64)> {
    let cn = n.get("center_norm")?;
    Some((
        cn.get("x").and_then(|v| v.as_f64())?,
        cn.get("y").and_then(|v| v.as_f64())?,
    ))
}

fn typeable_nodes<'a>(
    snap: &'a ObserveSnapshot,
    label: Option<&str>,
    id: Option<&str>,
) -> Vec<&'a serde_json::Value> {
    let all: Vec<&serde_json::Value> = snap_node_sources(snap).collect();
    let mut exact: Vec<&serde_json::Value> = all
        .iter()
        .copied()
        .filter(|n| node_matches_target(n, label, id) && node_editable(n))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    if let Some(anchor) = all.iter().copied().find(|n| node_matches_target(n, label, id)) {
        let ac = node_center_norm(anchor);
        exact = all
            .iter()
            .copied()
            .filter(|n| node_editable(n))
            .collect();
        exact.sort_by(|a, b| {
            let da = match (ac, node_center_norm(a)) {
                (Some((ax, ay)), Some((bx, by))) => (ax - bx).hypot(ay - by),
                _ => f64::MAX,
            };
            let db = match (ac, node_center_norm(b)) {
                (Some((ax, ay)), Some((bx, by))) => (ax - bx).hypot(ay - by),
                _ => f64::MAX,
            };
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        return exact.into_iter().take(2).collect();
    }
    all.into_iter()
        .filter(|n| {
            node_editable(n)
                && (n.get("focused").and_then(|v| v.as_bool()) == Some(true)
                    || node_matches_target(n, label, id))
        })
        .collect()
}

fn value_committed(node: &serde_json::Value, id: Option<&str>, text: &str) -> bool {
    let val = node.get("value").and_then(|v| v.as_str()).unwrap_or("");
    let ph = node
        .get("placeholder")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if val.is_empty() || val.eq_ignore_ascii_case(ph) {
        return false;
    }
    let secure = id
        .map(|s| s.to_ascii_lowercase().contains("secure") || s.to_ascii_lowercase().contains("password"))
        .unwrap_or(false)
        || ph.eq_ignore_ascii_case("password")
        || node
            .get("role")
            .and_then(|v| v.as_str())
            .is_some_and(|r| r.to_ascii_lowercase().contains("secure"));
    if secure {
        return val.chars().count() >= text.chars().count();
    }
    val.to_ascii_lowercase()
        .contains(&text.to_ascii_lowercase())
}

fn field_value_hash(node: &serde_json::Value) -> String {
    format!(
        "{}|{}",
        node.get("value").and_then(|v| v.as_str()).unwrap_or(""),
        node.get("placeholder")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    )
}

fn typeable_hashes(
    snap: &ObserveSnapshot,
    label: Option<&str>,
    id: Option<&str>,
) -> Vec<String> {
    typeable_nodes(snap, label, id)
        .into_iter()
        .map(field_value_hash)
        .collect()
}

fn deliver_typed_text(
    udid: &str,
    text: &str,
    strategy: MotorTypeStrategy,
) -> Result<(), ligh_core::LighError> {
    let paste = matches!(strategy, MotorTypeStrategy::ClearRetype)
        || !hid_type_is_layout_stable(text);
    if paste {
        HidInput::paste_text(udid, text)
    } else {
        HidInput::type_text(udid, text)
    }
}

fn hid_type_is_layout_stable(text: &str) -> bool {
    text.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || c == '.'
            || c == '-'
            || c == '_'
            || c == ' '
            || c == '\n'
            || c == '\t'
    })
}

fn keyboard_up(snap: &ObserveSnapshot) -> bool {
    snap.scene
        .as_ref()
        .map(|s| s.keyboard_visible)
        .unwrap_or(false)
        || matches!(overlay_from_snapshot(snap), Overlay::Keyboard)
}

/// Type is a verified commit: focused/typeable identity plus value change,
/// or value containing the typed text. Keyboard alone is not enough.
fn type_commit_verified(
    before: &ObserveSnapshot,
    after: &ObserveSnapshot,
    label: Option<&str>,
    id: Option<&str>,
    text: &str,
) -> Option<&'static str> {
    if typeable_nodes(after, label, id)
        .into_iter()
        .any(|n| value_committed(n, id, text))
    {
        return Some("field_value");
    }
    let before_h = typeable_hashes(before, label, id);
    let after_h = typeable_hashes(after, label, id);
    let hash_changed = !after_h.is_empty() && after_h != before_h;
    let focused = target_focused_editable(after, label, id) || any_focused_editable(after);
    if hash_changed && (focused || keyboard_up(after)) {
        // Hash change is not a commit unless the new value actually contains
        // the typed text (or secure length). Shift punctuation is layout-sensitive;
        // a garbled glyph must not count as success.
        if typeable_nodes(after, label, id)
            .into_iter()
            .any(|n| value_committed(n, id, text))
        {
            return Some("value_hash");
        }
    }
    None
}

/// Focus an editable target (idempotent — no fault if already focused).
pub(crate) fn motor_ensure_focus_editable(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let snap = build();
    if target_focused_editable(&snap, label, id) {
        return CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "ensure_focus",
            json!({ "id": id, "label": label, "already": true }),
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
                "ensure_focus",
                json!({ "error": e }),
                Some(snap),
            );
        }
    };
    if let Some(eid) = id {
        if AxDump::press_id(&udid, eid).is_ok() {
            std::thread::sleep(Duration::from_millis(180));
            let after = build();
            if target_focused_editable(&after, label, id) {
                return CapabilityResult::success(
                    phase_of(&after),
                    surface_of(&after),
                    "ensure_focus",
                    json!({ "id": id, "label": label, "via": "ax_press_id" }),
                    Some(after),
                );
            }
        }
    }
    let tap = motor_tap(build, state, label, id, settle_ms, timeout_ms, None, None);
    if tap.ok {
        return tap;
    }
    let after = build();
    if target_focused_editable(&after, label, id) {
        return CapabilityResult::success(
            phase_of(&after),
            surface_of(&after),
            "ensure_focus",
            json!({ "id": id, "label": label, "via": "tap_side_effect" }),
            Some(after),
        );
    }
    tap
}

/// Tap through the motor pipeline. Optional `until_*` polls for postcondition (async nav).
pub(crate) fn motor_tap(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
    until_id: Option<&str>,
    until_label: Option<&str>,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(2000), 3);
    if !ready.ok {
        return ready;
    }
    let _ = crate::cognition::wait_settled(build, settle_ms.min(2800));
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
    let (target, pre_snap) = match ensure_path(
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
    let overlay = overlay_from_snapshot(&pre_snap);
    let tap_out = match motor_fire_verified(
        build,
        &udid,
        &target,
        w,
        h,
        id,
        label,
        overlay,
        &pre_snap,
        settle_ms,
    ) {
        Ok((method, snap)) => {
            state.lock().unwrap().push_action_result(
                true,
                "act_tap",
                json!({ "target": target.name, "method": method, "verified": true }),
            );
            Ok((method, snap))
        }
        Err(e) => Err(e),
    };

    if until_id.is_some() || until_label.is_some() {
        let deadline =
            Instant::now() + Duration::from_millis(timeout_ms.max(settle_ms.max(2500)));
        let mut last_snap = pre_snap.clone();
        while Instant::now() < deadline {
            if target_onscreen_udid(&udid, until_label, until_id) {
                last_snap = settle_eyes(build, settle_ms.min(600));
                return CapabilityResult::success(
                    phase_of(&last_snap),
                    surface_of(&last_snap),
                    "act_tap",
                    json!({
                        "target": target.name,
                        "motor": "tap_until",
                        "until_id": until_id,
                        "until_label": until_label,
                        "verified": true,
                    }),
                    Some(last_snap),
                )
                .with_action_outcome(ActionOutcome::DeliveredAndVerified);
            }
            std::thread::sleep(Duration::from_millis(120));
            last_snap = build();
        }
        return CapabilityResult::fail(
            FaultClass::TargetMissing,
            phase_of(&last_snap),
            surface_of(&last_snap),
            "act_tap",
            json!({
                "error": "until postcondition not reached after tap",
                "until_id": until_id,
                "until_label": until_label,
                "target": target.name,
                "tap_ok": tap_out.is_ok(),
            }),
            Some(last_snap),
        );
    }

    match tap_out {
        Ok((method, snap)) => CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "act_tap",
            json!({
                "target": target.name,
                "motor": "ensure_path",
                "method": method,
                "verified": true,
            }),
            Some(snap),
        )
        .with_action_outcome(ActionOutcome::DeliveredAndVerified),
        Err(e) => CapabilityResult::fail(
            e.fault,
            e.phase,
            e.surface,
            "act_tap",
            e.detail.unwrap_or(json!({ "target": target.name })),
            e.observe,
        ),
    }
}

/// Atomic focus + type with field-value verification (Motor 2.0).
/// Strategy is chosen by the planner; the motor does not retry the same method.
pub(crate) fn motor_type(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    text: &str,
    label: Option<&str>,
    id: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
    strategy: MotorTypeStrategy,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(800), 0);
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
                    "act_type",
                    json!({ "error": e }),
                    ready.observe,
                );
            }
        }
    };

    if id.is_none() && label.is_none() {
        let pre = ready.observe.clone().unwrap_or_else(|| build());
        if !any_focused_editable(&pre) {
            return CapabilityResult::fail(
                FaultClass::MotorRejected,
                phase_of(&pre),
                surface_of(&pre),
                "act_type",
                json!({
                    "error": "no focused editable field — tap target before type",
                    "actionable_topk": slim_topk(&pre, 8),
                }),
                Some(pre),
            );
        }
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
        let snap = settle_eyes(build, settle_ms.min(800));
        return CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "act_type",
            json!({ "text": text, "verified": "host_accepted", "motor": "type" }),
            Some(snap),
        )
        .with_action_outcome(ActionOutcome::DeliveredAndVerified);
    }

    let before = ready.observe.clone().unwrap_or_else(|| build());
    let target = match AxDump::dump(&udid).ok().and_then(|dump| resolve_from_dump(&dump, label, id))
    {
        Some(t) => t,
        None => {
            return CapabilityResult::fail(
                FaultClass::TargetMissing,
                phase_of(&before),
                surface_of(&before),
                "act_type",
                json!({ "error": "typeable node not on screen", "id": id, "label": label }),
                Some(before),
            )
            .with_action_outcome(ActionOutcome::NotDelivered);
        }
    };

    let (tap_x, tap_y) = match strategy {
        MotorTypeStrategy::CoordOffsetHid => (target.nx, (target.ny + 0.012).min(0.92)),
        _ => (target.nx, target.ny),
    };
    let wait_keyboard = matches!(
        strategy,
        MotorTypeStrategy::TapThenHid | MotorTypeStrategy::CoordOffsetHid
    );

    // AX press is not trusted as first responder. Always HID-tap the typeable node.
    let _ = HidInput::tap(&udid, tap_x, tap_y, w, h);
    std::thread::sleep(Duration::from_millis(180));
    if matches!(strategy, MotorTypeStrategy::FocusHid) {
        if let Some(eid) = id {
            let _ = AxDump::press_id(&udid, eid);
            std::thread::sleep(Duration::from_millis(80));
        }
    }

    let focus_deadline = Instant::now() + Duration::from_millis(if wait_keyboard { 1600 } else { 700 });
    let mut last_snap = settle_eyes(build, 120);
    while Instant::now() < focus_deadline {
        last_snap = build();
        if target_focused_editable(&last_snap, label, id) || any_focused_editable(&last_snap) {
            break;
        }
        if wait_keyboard && keyboard_up(&last_snap) {
            break;
        }
        std::thread::sleep(Duration::from_millis(60));
    }

    if let Err(e) = deliver_typed_text(&udid, text, strategy) {
        return CapabilityResult::fail(
            FaultClass::MotorRejected,
            SessionPhase::Degraded,
            ready.surface.clone(),
            "act_type",
            json!({
                "error": e.to_string(),
                "strategy": strategy.as_str(),
            }),
            Some(last_snap),
        )
        .with_action_outcome(ActionOutcome::NotDelivered);
    }

    let poll_until = Instant::now() + Duration::from_millis(timeout_ms.min(2200).max(900));
    while Instant::now() < poll_until {
        last_snap = settle_eyes(build, settle_ms.min(220));
        if let Some(signal) = type_commit_verified(&before, &last_snap, label, id, text) {
            state.lock().unwrap().push_action_result(
                true,
                "act_type",
                json!({ "text": text, "strategy": strategy.as_str(), "verified": signal }),
            );
            return CapabilityResult::success(
                phase_of(&last_snap),
                surface_of(&last_snap),
                "act_type",
                json!({
                    "text": text,
                    "verified": signal,
                    "motor": "type_commit",
                    "strategy": strategy.as_str(),
                    "id": id,
                    "label": label,
                }),
                Some(last_snap),
            )
            .with_action_outcome(ActionOutcome::DeliveredAndVerified);
        }
        std::thread::sleep(Duration::from_millis(80));
    }

    CapabilityResult::fail(
        FaultClass::MotorNoEffect,
        phase_of(&last_snap),
        surface_of(&last_snap),
        "act_type",
        json!({
            "error": "type commit failed: field value unchanged",
            "id": id,
            "label": label,
            "text": text,
            "strategy": strategy.as_str(),
            "keyboard": keyboard_up(&last_snap),
            "focused": target_focused_editable(&last_snap, label, id),
            "actionable_topk": slim_topk(&last_snap, 8),
        }),
        Some(last_snap),
    )
    .with_action_outcome(ActionOutcome::DeliveredNoEffect)
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
    let _ = crate::cognition::wait_settled(build, settle_ms.min(2800));
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

pub(crate) fn motor_scroll_until(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    max_swipes: u32,
    timeout_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, 1500, 3);
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
                    "scroll_until",
                    json!({ "error": e }),
                    ready.observe,
                );
            }
        }
    };
    let scroll_budget = max_swipes as u64 * 450 + 4000;
    let deadline = Instant::now()
        + Duration::from_millis(timeout_ms.max(scroll_budget));
    let mut swipes = 0u32;
    loop {
        if let Ok(dump) = AxDump::dump(&udid) {
            // Prefer viewport-hittable; accept in-tree for virtualized lists (tap uses reach next).
            let found = if let Some(eid) = id {
                find_onscreen_id_in_dump(&dump, eid).is_some()
            } else if let Some(lab) = label {
                find_hittable_label_in_dump(&dump, lab).is_some()
            } else {
                false
            };
            if found {
                let snap = build();
                return CapabilityResult::success(
                    phase_of(&snap),
                    surface_of(&snap),
                    "scroll_until",
                    json!({
                        "found": true,
                        "id": id,
                        "label": label,
                        "swipes": swipes,
                        "hittable": true,
                    }),
                    Some(snap),
                );
            }
        }
        if swipes >= max_swipes || Instant::now() >= deadline {
            let snap = build();
            let overlay = overlay_from_snapshot(&snap);
            return CapabilityResult::fail(
                FaultClass::TargetMissing,
                phase_of(&snap),
                surface_of(&snap),
                "scroll_until",
                fault_evidence(
                    &snap,
                    &udid,
                    label,
                    id,
                    overlay,
                    &format!("scroll_until miss after {swipes} swipes"),
                ),
                Some(snap),
            );
        }
        let swipe_x = AxDump::dump(&udid)
            .ok()
            .and_then(|d| find_id_in_dump(&d, "FeedList").map(|(x, _)| x))
            .unwrap_or(0.5);
        if let Err(e) = HidInput::swipe(&udid, swipe_x, 0.84, swipe_x, 0.16, w, h) {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Ready,
                surface_of(&build()),
                "scroll_until",
                json!({ "error": e.to_string() }),
                ready.observe,
            );
        }
        swipes += 1;
        std::thread::sleep(Duration::from_millis(320));
    }
}

/// Host-owned reach: dismiss overlays, scroll, wait until id/label is on a clear path.
pub(crate) fn motor_reach(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    max_swipes: u32,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(2000), 3);
    if !ready.ok {
        return ready;
    }
    let _ = crate::cognition::wait_settled(build, settle_ms.min(2800));
    let (udid, w, h) = {
        let st = state.lock().unwrap();
        match st.current_udid() {
            Ok(u) => (u, st.sim_width, st.sim_height),
            Err(e) => {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Dead,
                    None,
                    "reach",
                    json!({ "error": e }),
                    ready.observe,
                );
            }
        }
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut scrolls = 0u32;
    loop {
        match ensure_path(
            build,
            &udid,
            w,
            h,
            label,
            id,
            Duration::from_millis(2500.min(timeout_ms / 4)),
        ) {
            Ok((target, snap)) => {
                return CapabilityResult::success(
                    phase_of(&snap),
                    surface_of(&snap),
                    "reach",
                    json!({
                        "target": target.name,
                        "scrolls": scrolls,
                        "motor": "reach",
                    }),
                    Some(snap),
                );
            }
            Err(e) => {
                if Instant::now() >= deadline {
                    return CapabilityResult::fail(
                        e.fault,
                        e.phase,
                        e.surface,
                        "reach",
                        e.detail.unwrap_or(json!({})),
                        e.observe,
                    );
                }
                let snap = build();
                let overlay = overlay_from_snapshot(&snap);
                // Do not dismiss sheet/alert while searching — target may live on the overlay.
                if matches!(
                    overlay,
                    Overlay::Sheet | Overlay::Alert | Overlay::SystemSurface
                ) {
                    // fall through to scroll attempt
                } else if overlay.blocks_path() {
                    let _ = clear_overlay(overlay, &udid, w, h, surface_role(&snap));
                    continue;
                }
                if id.is_some() || label.is_some() {
                    if scrolls < max_swipes {
                        let _ = HidInput::swipe(&udid, 0.5, 0.84, 0.5, 0.16, w, h);
                        scrolls += 1;
                        std::thread::sleep(Duration::from_millis(280));
                        continue;
                    }
                }
                return CapabilityResult::fail(
                    e.fault,
                    e.phase,
                    e.surface,
                    "reach",
                    e.detail.unwrap_or(json!({})),
                    e.observe,
                );
            }
        }
    }
}

/// Launch an installed app by bundle id (system apps, no .app path).
pub(crate) fn motor_launch(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    bundle_id: &str,
    settle_ms: u64,
) -> CapabilityResult {
    let udid = match state.lock().unwrap().current_udid() {
        Ok(u) => u,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Dead,
                None,
                "launch",
                json!({ "error": e }),
                None,
            );
        }
    };
    if let Err(e) = ligh_sim::Simctl::run(&[
        "launch",
        &udid,
        bundle_id,
        "--terminate-running-process",
    ]) {
        return CapabilityResult::fail(
            FaultClass::Infra,
            SessionPhase::Degraded,
            None,
            "launch",
            json!({ "error": e.to_string(), "bundle_id": bundle_id }),
            None,
        );
    }
    std::thread::sleep(Duration::from_millis(400));
    if let Ok(cfg) = ligh_core::LighConfig::load() {
        if let Ok(Some(mut s)) = ligh_core::SessionState::load(&cfg.state_dir) {
            s.app_bundle_id = Some(bundle_id.to_string());
            let _ = s.save(&cfg.state_dir);
        }
    }
    let snap = settle_eyes(build, settle_ms);
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "launch",
        json!({ "bundle_id": bundle_id }),
        Some(snap),
    )
}

/// Named HID key (return, delete, escape, …).
pub(crate) fn motor_key(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    key_name: &str,
    settle_ms: u64,
) -> CapabilityResult {
    let udid = match state.lock().unwrap().current_udid() {
        Ok(u) => u,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Dead,
                None,
                "key",
                json!({ "error": e }),
                None,
            );
        }
    };
    if let Err(e) = HidInput::key_named(&udid, key_name) {
        return CapabilityResult::fail(
            FaultClass::Infra,
            SessionPhase::Ready,
            None,
            "key",
            json!({ "error": e.to_string(), "key": key_name }),
            None,
        );
    }
    std::thread::sleep(Duration::from_millis(120));
    let snap = settle_eyes(build, settle_ms.min(2000));
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "key",
        json!({ "key": key_name }),
        Some(snap),
    )
}

/// Try to clear keyboard/sheet/alert without changing app surface.
pub(crate) fn motor_dismiss_overlay(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms.min(1500), 2);
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
                    "dismiss_overlay",
                    json!({ "error": e }),
                    ready.observe,
                );
            }
        }
    };
    let snap = build();
    let overlay = overlay_from_snapshot(&snap);
    if overlay == Overlay::None {
        return CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "dismiss_overlay",
            json!({ "overlay": "none", "cleared": false }),
            Some(snap),
        );
    }
    let cleared = clear_overlay(overlay, &udid, w, h, surface_role(&snap));
    let snap2 = settle_eyes(build, settle_ms);
    let after = overlay_from_snapshot(&snap2);
    CapabilityResult::success(
        phase_of(&snap2),
        surface_of(&snap2),
        "dismiss_overlay",
        json!({
            "overlay_before": overlay.as_str(),
            "overlay_after": after.as_str(),
            "cleared": cleared && after == Overlay::None,
        }),
        Some(snap2),
    )
}

/// Explore: reach → probe gestures → reach again. Returns probe_log in detail.
pub(crate) fn motor_explore(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    label: Option<&str>,
    id: Option<&str>,
    max_probes: u32,
    max_swipes: u32,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let _ = crate::cognition::wait_settled(build, settle_ms.min(3000));
    let half = timeout_ms / 2;
    let mut r = motor_reach(
        build,
        state,
        label,
        id,
        max_swipes.min(6),
        settle_ms,
        half.max(3000),
    );
    if r.ok {
        r.capability = Some("explore".into());
        if let Some(ref mut det) = r.detail {
            if let Some(obj) = det.as_object_mut() {
                obj.insert("phase".into(), json!("reach_first"));
            }
        }
        return r;
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
                    "explore",
                    json!({ "error": e }),
                    r.observe,
                );
            }
        }
    };

    let (probes, _) = crate::cognition::run_probes(build, &udid, w, h, max_probes.max(1).min(6));

    r = motor_reach(
        build,
        state,
        label,
        id,
        max_swipes,
        settle_ms,
        half.max(4000),
    );
    if r.ok {
        r.capability = Some("explore".into());
        if let Some(det) = r.detail.as_mut() {
            if let Some(obj) = det.as_object_mut() {
                obj.insert("probes_tried".into(), json!(probes));
                obj.insert("phase".into(), json!("reach_after_probes"));
            }
        }
        return r;
    }

    let snap = r.observe.clone().unwrap_or_else(|| build());
    let mut detail = fault_evidence(
        &snap,
        &udid,
        label,
        id,
        overlay_from_snapshot(&snap),
        "explore exhausted reach + probes",
    );
    attach_probes(&mut detail, &probes);
    CapabilityResult::fail(
        r.fault,
        r.phase,
        r.surface,
        "explore",
        detail,
        Some(snap),
    )
}
