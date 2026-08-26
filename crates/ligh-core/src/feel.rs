//! Feel IR — live interaction representation (not UX graph memory).
//!
//! Host-owned frame of the current scene: where you are, what weighs,
//! what blocks, what just changed. Built in ~ms from settle + AX.
//! Consumers: host planners (`app-job`, compiled replay, killer exercise).
//! Do **not** treat this as LLM long-term memory (that path failed for UX graph).

use serde::{Deserialize, Serialize};

use crate::observe::ObserveSnapshot;
use crate::qa::{infer_affordances, Affordance, AffordanceKind, PerceiveView};

pub const FEEL_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldElement {
    pub stable_key: String,
    pub ax_path: String,
    pub kind: AffordanceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_hash: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub editable: bool,
    pub on_screen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_scope: Option<String>,
    /// True when this element is tab-bar chrome (container or item).
    #[serde(default)]
    pub tab_chrome: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorldModel {
    pub screen_epoch: u64,
    pub structural_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_app_label: Option<String>,
    pub ownership_confidence: f64,
    pub elements: Vec<WorldElement>,
    pub has_scroll_container: bool,
    pub has_tab_bar: bool,
    pub can_navigate_back: bool,
    pub stability_streak: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_events: Vec<String>,
}

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
    pub world: WorldModel,
    /// Hyper-computational Scene IR digest (regions + ε). Built with Feel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<crate::scene::SceneDigest>,
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

fn value_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in value.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("v_{hash:016x}")
}

fn world_from_snapshot(snap: &ObserveSnapshot, fingerprint: &str) -> WorldModel {
    let nodes = snap.accessibility_tree.nodes();
    let affordances = infer_affordances(nodes, nodes.len().max(1));
    let mut elements = Vec::with_capacity(affordances.len());
    for affordance in affordances {
        let (index, raw) = nodes
            .iter()
            .enumerate()
            .find(|(_, node)| {
                let node_id = node
                    .get("identifier")
                    .and_then(|v| v.as_str())
                    .or_else(|| node.get("id").and_then(|v| v.as_str()));
                let node_label = node.get("label").and_then(|v| v.as_str());
                affordance.id.as_deref().map_or(true, |id| node_id == Some(id))
                    && affordance
                        .label
                        .as_deref()
                        .map_or(true, |label| node_label == Some(label))
            })
            .map(|(i, node)| (i, Some(node)))
            .unwrap_or((elements.len(), None));
        let role = raw
            .and_then(|n| n.get("role"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let frame_bucket = raw
            .and_then(|n| n.get("frame"))
            .and_then(|v| v.as_object())
            .map(|f| {
                let x = f.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) / 24.0;
                let y = f.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) / 24.0;
                format!("{}:{}", x.floor() as i64, y.floor() as i64)
            });
        let stable_key = if let Some(id) = &affordance.id {
            format!("id:{id}")
        } else {
            format!(
                "path:{index}:{}:{}:{}",
                role.as_deref().unwrap_or("?"),
                affordance.label.as_deref().unwrap_or(""),
                frame_bucket.as_deref().unwrap_or("?")
            )
        };
        let enabled = raw
            .and_then(|n| n.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        elements.push(WorldElement {
            stable_key,
            ax_path: format!("/{index}"),
            kind: affordance.kind,
            identifier: affordance.id,
            label: affordance.label,
            role,
            frame_bucket,
            value_hash: affordance.value.as_deref().map(value_hash),
            enabled,
            focused: affordance.focused,
            editable: matches!(
                affordance.kind,
                AffordanceKind::TextField
                    | AffordanceKind::SecureField
                    | AffordanceKind::SearchField
            ),
            on_screen: affordance.hittable,
            overlay_scope: snap.overlay.clone().filter(|o| o != "none"),
            tab_chrome: raw.map(crate::observe::is_tab_bar_node).unwrap_or(false),
        });
    }
    // Identifier-bearing nodes that top-k scoring dropped still exist in AX.
    // Goal matching and "acceptance not in tree" must see them.
    let known: std::collections::HashSet<String> = elements
        .iter()
        .filter_map(|e| e.identifier.clone())
        .collect();
    for (index, node) in nodes.iter().enumerate() {
        let Some(id) = node
            .get("identifier")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if known.contains(id) {
            continue;
        }
        let role = node
            .get("role")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let label = node
            .get("label")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let hittable = node.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true);
        let tab = crate::observe::is_tab_bar_node(node);
        elements.push(WorldElement {
            stable_key: format!("id:{id}"),
            ax_path: format!("/{index}"),
            kind: if tab {
                AffordanceKind::Button
            } else {
                AffordanceKind::Other
            },
            identifier: Some(id.to_string()),
            label,
            role,
            frame_bucket: None,
            value_hash: node
                .get("value")
                .and_then(|v| v.as_str())
                .map(value_hash),
            enabled: node.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            focused: node.get("focused").and_then(|v| v.as_bool()).unwrap_or(false),
            editable: false,
            on_screen: hittable,
            overlay_scope: snap.overlay.clone().filter(|o| o != "none"),
            tab_chrome: tab,
        });
    }
    let has_scroll_container = nodes.iter().any(|n| {
        n.get("role")
            .and_then(|v| v.as_str())
            .map(|r| r.to_ascii_lowercase().contains("scroll"))
            .unwrap_or(false)
    });
    let has_tab_bar = nodes.iter().any(crate::observe::is_tab_bar_node);
    let can_navigate_back = elements
        .iter()
        .any(|e| e.kind == AffordanceKind::NavBack && e.on_screen);
    let observed_non_system = snap
        .observed_app_label
        .as_deref()
        .map(|l| l != "SpringBoard" && l != "Home")
        .unwrap_or(false);
    WorldModel {
        screen_epoch: snap.screen_epoch,
        structural_fingerprint: fingerprint.to_string(),
        expected_bundle_id: snap.expected_bundle_id.clone(),
        observed_app_label: snap.observed_app_label.clone(),
        ownership_confidence: if observed_non_system { 0.8 } else { 0.0 },
        elements,
        has_scroll_container,
        has_tab_bar,
        can_navigate_back,
        stability_streak: snap.stability_streak.max(u32::from(snap.settled)),
        motion_score: snap.motion_score,
        semantic_events: snap.events.iter().map(|e| e.kind.clone()).collect(),
    }
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

fn enrich_salience_from_world(salience: &mut Vec<SalienceItem>, world: &WorldModel) {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = salience
        .iter()
        .filter_map(|s| s.id.clone())
        .collect();
    let mut extra = Vec::new();
    for el in &world.elements {
        if !el.on_screen || !el.enabled {
            continue;
        }
        if !matches!(
            el.kind,
            AffordanceKind::PrimaryButton
                | AffordanceKind::Button
                | AffordanceKind::Link
                | AffordanceKind::Switch
                | AffordanceKind::Cell
        ) {
            continue;
        }
        let Some(id) = el.identifier.clone() else {
            continue;
        };
        if seen.contains(&id) {
            continue;
        }
        seen.insert(id.clone());
        extra.push(SalienceItem {
            rank: 0,
            kind: el.kind,
            label: el.label.clone(),
            id: Some(id),
            weight: kind_weight(el.kind, el.focused),
        });
    }
    if extra.is_empty() {
        return;
    }
    salience.extend(extra);
    salience.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (i, item) in salience.iter_mut().enumerate() {
        item.rank = (i as u32) + 1;
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

    let planner_affordances = infer_affordances(
        snap.accessibility_tree.nodes(),
        snap.accessibility_tree.nodes().len().max(1),
    );
    let mut scored: Vec<(f64, &Affordance)> = planner_affordances
        .iter()
        .filter(|a| a.hittable)
        .map(|a| (kind_weight(a.kind, a.focused), a))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut salience: Vec<SalienceItem> = scored
        .into_iter()
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

    let mut world = world_from_snapshot(snap, &fp);
    enrich_salience_from_world(&mut salience, &world);
    if fingerprint_changed {
        world.semantic_events.push("navigation_occurred".into());
    }
    if view
        .since_last
        .iter()
        .any(|event| event.contains("keyboard"))
    {
        world.semantic_events.push("keyboard_changed".into());
    }
    if view.since_last.iter().any(|event| event == "action_result") {
        world.semantic_events.push("value_or_action_committed".into());
    }
    world.semantic_events.sort();
    world.semantic_events.dedup();
    let mut feel = FeelIR {
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
        world,
        scene: None,
    };
    feel.scene = Some(crate::scene::build_scene_digest(snap, &feel));
    feel
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

/// Agent-facing compact Feel — **Scene IR first**, then thin control meta.
/// Raw AX never belongs here. Salience ≤5 is fallback, not the map.
pub fn feel_agent_view(feel: &FeelIR) -> serde_json::Value {
    let scene = feel
        .scene
        .as_ref()
        .map(crate::scene::scene_agent_view)
        .unwrap_or(serde_json::json!(null));
    serde_json::json!({
        "schema": feel.schema,
        "perception": "scene_ir",
        "scene": scene,
        "place": {
            "fp": feel.place.fingerprint,
            "surface": feel.place.surface,
            "title": feel.place.title,
            "bundle": feel.place.bundle_id,
        },
        "phase": feel.feel.phase,
        "ready": feel.feel.ready,
        "keyboard": feel.feel.keyboard,
        "block": feel.block,
        "delta": {
            "fingerprint_changed": feel.delta.fingerprint_changed,
            "events": feel.delta.events,
        },
        "salience": feel.salience.iter().take(5).collect::<Vec<_>>(),
        "suggest": suggest_act(feel),
        "motor": feel.scene.as_ref().map(|s| &s.motor),
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
            session_id: Some("test".into()),
            boot_epoch: 1,
            launch_epoch: 1,
            screen_epoch: 1,
            stability_streak: 2,
            motion_score: Some(0.0),
            expected_bundle_id: Some("me.demo".into()),
            observed_app_label: Some("Demo".into()),
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
            screen_sig: None,
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

    #[test]
    fn world_detects_tab_bar_and_keeps_tab_identifier() {
        let mut snap = snap_ready();
        if let AccessibilityTree::Available { nodes, .. } = &mut snap.accessibility_tree {
            nodes.push(json!({
                "role": "AXGroup",
                "label": "Tab Bar",
                "hittable": true,
                "enabled": true
            }));
            nodes.push(json!({
                "role": "AXTabButton",
                "identifier": "tab_home",
                "label": "Home",
                "traits": "tabbar",
                "hittable": true,
                "enabled": true
            }));
        }
        let feel = build_feel(&view_onboarding(), &snap, None, None);
        assert!(feel.world.has_tab_bar);
        assert!(feel
            .world
            .elements
            .iter()
            .any(|e| e.identifier.as_deref() == Some("tab_home")));
    }
}
