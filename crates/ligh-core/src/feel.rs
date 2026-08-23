//! Feel IR — live interaction representation (not UX graph memory).
//!
//! Host-owned frame of the current scene: where you are, what weighs,
//! what blocks, what just changed. Built in ~ms from settle + AX.
//! Consumers: host planners (`app-job`, compiled replay, killer exercise).
//! Do **not** treat this as LLM long-term memory (that path failed for UX graph).

use serde::{Deserialize, Serialize};

use crate::observe::ObserveSnapshot;
use crate::qa::{Affordance, AffordanceKind, PerceiveView};

pub const FEEL_SCHEMA_VERSION: u32 = 1;

/// Temporal / trust phase of the live scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeelPhase {
    Settled,
    Transition,
    EyesUnusable,
    Blocked,
}

/// One ranked interactive target in the scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalienceItem {
    pub rank: u32,
    pub kind: AffordanceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Host weight: higher = more likely primary next act.
    pub weight: f64,
}

/// Delta since the previous Feel / fingerprint (when known).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeelDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_fp: Option<String>,
    pub fingerprint_changed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_ms: Option<u64>,
}

/// Live Feel IR — one frame of UX-as-computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeelIR {
    pub schema: u32,
    pub place: FeelPlace,
    /// Ranked actionable targets (cap small — host/LLM top-N only).
    pub salience: Vec<SalienceItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<FeelBlock>,
    pub delta: FeelDelta,
    pub feel: FeelMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeelPlace {
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeelBlock {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeelMeta {
    pub phase: FeelPhase,
    pub keyboard: bool,
    pub ready: bool,
}

/// Suggested next host act derived from FeelIR (zero-LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeelSuggestedAct {
    pub intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub reason: String,
}

fn kind_weight(kind: AffordanceKind, focused: bool) -> f64 {
    let base = match kind {
        AffordanceKind::PrimaryButton => 100.0,
        AffordanceKind::TextField | AffordanceKind::SecureField | AffordanceKind::SearchField => {
            if focused {
                90.0
            } else {
                55.0
            }
        }
        AffordanceKind::Button => 70.0,
        AffordanceKind::Link => 60.0,
        AffordanceKind::Switch => 50.0,
        AffordanceKind::Cell => 45.0,
        AffordanceKind::NavBack => 40.0,
        AffordanceKind::StaticText => 10.0,
        AffordanceKind::Other => 20.0,
    };
    if focused {
        base + 15.0
    } else {
        base
    }
}

fn phase_of(view: &PerceiveView, snap: &ObserveSnapshot) -> FeelPhase {
    if view.eyes_unusable || !view.ready {
        return FeelPhase::EyesUnusable;
    }
    if view.blocking.is_some() {
        return FeelPhase::Blocked;
    }
    if !snap.settled {
        return FeelPhase::Transition;
    }
    FeelPhase::Settled
}

/// Build FeelIR from a settled perceive view (+ optional previous fingerprint).
pub fn build_feel(
    view: &PerceiveView,
    snap: &ObserveSnapshot,
    prev_fp: Option<&str>,
    settle_ms: Option<u64>,
) -> FeelIR {
    let keyboard = snap
        .scene
        .as_ref()
        .map(|s| s.keyboard_visible)
        .unwrap_or(false);

    let mut scored: Vec<(f64, &Affordance)> = view
        .affordances
        .iter()
        .filter(|a| a.hittable)
        .map(|a| (kind_weight(a.kind, a.focused), a))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let salience: Vec<SalienceItem> = scored
        .into_iter()
        .take(8)
        .enumerate()
        .map(|(i, (w, a))| SalienceItem {
            rank: (i as u32) + 1,
            kind: a.kind,
            label: a.label.clone(),
            id: a.id.clone(),
            weight: w,
        })
        .collect();

    let fp = view.location.fingerprint.clone();
    let fingerprint_changed = prev_fp.map(|p| p != fp).unwrap_or(false);

    FeelIR {
        schema: FEEL_SCHEMA_VERSION,
        place: FeelPlace {
            fingerprint: fp,
            surface: view.location.surface.clone(),
            title: view.location.title.clone(),
            bundle_id: view.location.bundle_id.clone(),
        },
        salience,
        block: view.blocking.as_ref().map(|b| FeelBlock {
            kind: b.kind.clone(),
            detail: b.detail.clone(),
        }),
        delta: FeelDelta {
            from_fp: prev_fp.map(|s| s.to_string()),
            fingerprint_changed,
            events: view.since_last.clone(),
            settled_ms: settle_ms,
        },
        feel: FeelMeta {
            phase: phase_of(view, snap),
            keyboard,
            ready: view.ready && !view.eyes_unusable,
        },
    }
}

/// Host planner: pick next act from FeelIR (primary CTA, else top salience).
pub fn suggest_act(feel: &FeelIR) -> Option<FeelSuggestedAct> {
    match feel.feel.phase {
        FeelPhase::EyesUnusable => None,
        FeelPhase::Blocked => {
            Some(FeelSuggestedAct {
                intent: "dismiss".into(),
                label: None,
                id: None,
                reason: format!(
                    "blocked by {}",
                    feel.block.as_ref().map(|b| b.kind.as_str()).unwrap_or("overlay")
                ),
            })
        }
        FeelPhase::Transition => None,
        FeelPhase::Settled => {
            let top = feel.salience.first()?;
            if top.label.is_none() && top.id.is_none() {
                return None;
            }
            Some(FeelSuggestedAct {
                intent: "tap".into(),
                label: top.label.clone(),
                id: top.id.clone(),
                reason: format!("salience rank {} ({:?})", top.rank, top.kind),
            })
        }
    }
}

/// Agent-facing compact Feel (no full affordance dump).
pub fn feel_agent_view(feel: &FeelIR) -> serde_json::Value {
    serde_json::json!({
        "schema": feel.schema,
        "place": feel.place,
        "salience": feel.salience.iter().take(5).collect::<Vec<_>>(),
        "block": feel.block,
        "delta": {
            "fingerprint_changed": feel.delta.fingerprint_changed,
            "events": feel.delta.events,
        },
        "feel": feel.feel,
        "suggest": suggest_act(feel),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{AccessibilityTree, ObserveSnapshot};
    use crate::qa::{Affordance, AffordanceKind, BlockingView, LocationView, PerceiveView};
    use serde_json::json;

    fn snap_ready() -> ObserveSnapshot {
        ObserveSnapshot {
            schema_version: 2,
            udid: "test".into(),
            booted: true,
            simulator_app_running: false,
            frame: None,
            app_bundle_id: Some("me.demo".into()),
            accessibility_tree: AccessibilityTree::Available {
                nodes: vec![json!({
                    "role": "AXButton",
                    "identifier": "go",
                    "label": "Get Started",
                    "hittable": true,
                    "enabled": true
                })],
                root: None,
                element_count: Some(1),
                point_size: Some((393.0, 852.0)),
            },
            scene: None,
            actionable_topk: vec![],
            events: vec![],
            ax_quality: "ready".into(),
            settled: true,
            observe_ms: None,
            path: Some("test".into()),
            phase: Some("ready".into()),
            eyes_unusable: false,
            overlay: Some("none".into()),
        }
    }

    fn view_onboarding() -> PerceiveView {
        PerceiveView {
            ready: true,
            eyes_unusable: false,
            location: LocationView {
                fingerprint: "fp_test".into(),
                surface: Some("app".into()),
                title: Some("Welcome".into()),
                bundle_id: Some("me.demo".into()),
            },
            blocking: None,
            affordances: vec![
                Affordance {
                    kind: AffordanceKind::PrimaryButton,
                    label: Some("Get Started".into()),
                    id: Some("GetStarted".into()),
                    value: None,
                    focused: false,
                    hittable: true,
                    center_norm: None,
                },
                Affordance {
                    kind: AffordanceKind::Button,
                    label: Some("Skip".into()),
                    id: None,
                    value: None,
                    focused: false,
                    hittable: true,
                    center_norm: None,
                },
            ],
            since_last: vec!["tap:Show Onboarding".into()],
        }
    }

    #[test]
    fn feel_ranks_primary_cta_first() {
        let feel = build_feel(&view_onboarding(), &snap_ready(), Some("fp_prev"), Some(200));
        assert_eq!(feel.schema, FEEL_SCHEMA_VERSION);
        assert_eq!(feel.feel.phase, FeelPhase::Settled);
        assert!(feel.delta.fingerprint_changed);
        assert_eq!(feel.salience[0].label.as_deref(), Some("Get Started"));
        let act = suggest_act(&feel).unwrap();
        assert_eq!(act.intent, "tap");
        assert_eq!(act.label.as_deref(), Some("Get Started"));
    }

    #[test]
    fn feel_blocked_suggests_dismiss() {
        let mut v = view_onboarding();
        v.blocking = Some(BlockingView {
            kind: "keyboard".into(),
            detail: None,
        });
        let feel = build_feel(&v, &snap_ready(), None, None);
        assert_eq!(feel.feel.phase, FeelPhase::Blocked);
        assert_eq!(suggest_act(&feel).unwrap().intent, "dismiss");
    }
}
