//! Frontier control plane: session phase, overlays, fault classes, capability results.
//!
//! Architecture (not patches):
//! - Every observe stamps `phase` + `overlay`.
//! - Motor pipeline: ready → resolve → ensure_path → fire → settle.
//! - Overlays (keyboard/sheet/alert) are first-class — clearing is part of ensure_path,
//!   never bolted onto type/tap as ad-hoc side effects.
//! - Capabilities return structured faults — gates must not score infra as model failure.

use serde::{Deserialize, Serialize};

use crate::observe::ObserveSnapshot;

/// Session lifecycle owned by `lighd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    #[default]
    Booting,
    AxWarming,
    Ready,
    Acting,
    Settling,
    Degraded,
    Dead,
}

impl SessionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Booting => "booting",
            Self::AxWarming => "ax_warming",
            Self::Ready => "ready",
            Self::Acting => "acting",
            Self::Settling => "settling",
            Self::Degraded => "degraded",
            Self::Dead => "dead",
        }
    }

    pub fn allows_act(self) -> bool {
        matches!(self, Self::Ready | Self::Settling | Self::Acting)
    }
}

/// Modal / occlusion layer above the app surface. Acts target the top clear path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Overlay {
    #[default]
    None,
    Keyboard,
    Alert,
    Sheet,
    Transition,
}

impl Overlay {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Keyboard => "keyboard",
            Self::Alert => "alert",
            Self::Sheet => "sheet",
            Self::Transition => "transition",
        }
    }

    pub fn blocks_path(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Kernel fault taxonomy — never conflate dead eyes with a bad LLM plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FaultClass {
    #[default]
    Ok,
    Infra,
    EyesUnusable,
    TargetMissing,
    WrongSurface,
    /// Expected app is not in the foreground (e.g. SpringBoard showing the icon).
    AppNotForeground,
    /// Expected app process is not running.
    AppNotRunning,
    MotorRejected,
    Timeout,
    /// Target exists but an overlay prevents a clear path and could not be cleared.
    Blocked,
    /// HID/AX reported success but observable UI state did not change.
    MotorNoEffect,
    Model,
}

impl FaultClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Infra => "infra",
            Self::EyesUnusable => "eyes_unusable",
            Self::TargetMissing => "target_missing",
            Self::WrongSurface => "wrong_surface",
            Self::AppNotForeground => "app_not_foreground",
            Self::AppNotRunning => "app_not_running",
            Self::MotorRejected => "motor_rejected",
            Self::MotorNoEffect => "motor_no_effect",
            Self::Timeout => "timeout",
            Self::Blocked => "blocked",
            Self::Model => "model",
        }
    }

    pub fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }

    pub fn is_infra(self) -> bool {
        matches!(
            self,
            Self::Infra | Self::EyesUnusable | Self::Timeout | Self::Blocked
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub ok: bool,
    pub fault: FaultClass,
    pub phase: SessionPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Overlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observe: Option<ObserveSnapshot>,
}

impl CapabilityResult {
    pub fn success(
        phase: SessionPhase,
        surface: Option<String>,
        capability: &str,
        detail: serde_json::Value,
        observe: Option<ObserveSnapshot>,
    ) -> Self {
        let overlay = observe.as_ref().map(overlay_from_snapshot);
        Self {
            ok: true,
            fault: FaultClass::Ok,
            phase,
            surface,
            overlay,
            capability: Some(capability.into()),
            detail: Some(detail),
            observe,
        }
    }

    pub fn fail(
        fault: FaultClass,
        phase: SessionPhase,
        surface: Option<String>,
        capability: &str,
        detail: serde_json::Value,
        observe: Option<ObserveSnapshot>,
    ) -> Self {
        let overlay = observe.as_ref().map(overlay_from_snapshot);
        Self {
            ok: false,
            fault,
            phase,
            surface,
            overlay,
            capability: Some(capability.into()),
            detail: Some(detail),
            observe,
        }
    }
}

pub fn overlay_from_snapshot(snap: &ObserveSnapshot) -> Overlay {
    if snap.eyes_unusable || snap.ax_quality == "transition" {
        return Overlay::Transition;
    }
    let Some(scene) = snap.scene.as_ref() else {
        return Overlay::None;
    };
    if !scene.alerts.is_empty() {
        return Overlay::Alert;
    }
    if !scene.sheets.is_empty() {
        return Overlay::Sheet;
    }
    if scene.keyboard_visible {
        return Overlay::Keyboard;
    }
    Overlay::None
}

pub fn phase_from_snapshot(snap: &ObserveSnapshot, has_udid: bool) -> SessionPhase {
    if !has_udid || !snap.booted {
        return SessionPhase::Dead;
    }
    if eyes_unusable(snap) {
        if snap.ax_quality == "error" {
            return SessionPhase::Dead;
        }
        return SessionPhase::Degraded;
    }
    if snap.settled && snap.is_actionable_eyes() {
        return SessionPhase::Ready;
    }
    SessionPhase::AxWarming
}

pub fn eyes_unusable(snap: &ObserveSnapshot) -> bool {
    let aq = snap.ax_quality.as_str();
    aq == "empty" || aq == "transition" || aq == "error" || !snap.settled
}

/// Attach control-plane fields onto an observe snapshot (additive, schema v2).
pub fn stamp_control_fields(snap: &mut ObserveSnapshot, has_udid: bool) {
    let phase = phase_from_snapshot(snap, has_udid);
    snap.phase = Some(phase.as_str().into());
    snap.eyes_unusable = eyes_unusable(snap);
    snap.overlay = Some(overlay_from_snapshot(snap).as_str().into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{AccessibilityTree, ObserveSnapshot, SceneMeta};

    fn snap(aq: &str, settled: bool, actionable: usize) -> ObserveSnapshot {
        let mut s = ObserveSnapshot {
            schema_version: 2,
            udid: "x".into(),
            booted: true,
            simulator_app_running: false,
            frame: None,
            app_bundle_id: None,
            accessibility_tree: AccessibilityTree::Empty,
            scene: None,
            actionable_topk: (0..actionable)
                .map(|i| serde_json::json!({"label": format!("L{i}")}))
                .collect(),
            events: vec![],
            ax_quality: aq.into(),
            settled,
            observe_ms: None,
            path: None,
            phase: None,
            eyes_unusable: false,
            overlay: None,
        };
        stamp_control_fields(&mut s, true);
        s
    }

    #[test]
    fn ready_when_settled_actionable() {
        let s = snap("ready", true, 5);
        assert!(!s.eyes_unusable);
        assert_eq!(s.phase.as_deref(), Some("ready"));
        assert_eq!(s.overlay.as_deref(), Some("none"));
    }

    #[test]
    fn degraded_on_transition() {
        let s = snap("transition", false, 0);
        assert!(s.eyes_unusable);
        assert_eq!(s.phase.as_deref(), Some("degraded"));
        assert_eq!(s.overlay.as_deref(), Some("transition"));
    }

    #[test]
    fn keyboard_overlay_stamped() {
        let mut s = snap("ready", true, 3);
        s.scene = Some(SceneMeta {
            keyboard_visible: true,
            ..Default::default()
        });
        stamp_control_fields(&mut s, true);
        assert_eq!(s.overlay.as_deref(), Some("keyboard"));
    }

    #[test]
    fn infra_faults_classified() {
        assert!(FaultClass::EyesUnusable.is_infra());
        assert!(FaultClass::Blocked.is_infra());
        assert!(!FaultClass::TargetMissing.is_infra());
        assert!(!FaultClass::Ok.is_infra());
    }
}
