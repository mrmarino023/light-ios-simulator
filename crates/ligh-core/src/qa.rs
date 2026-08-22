//! Agent QA layer — screen fingerprints, affordances, perceive/attempt verdicts.
//!
//! Host-owned: agents get compact world models and action evidence, not raw AX dumps.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::observe::{
    build_actionable_topk, detect_surface, is_chrome_node, is_editable_role, ObserveSnapshot,
    SenseEvent,
};

/// Stable 16-hex screen fingerprint from role + identifier/label hierarchy (no coordinates).
pub fn screen_fingerprint(nodes: &[Value]) -> String {
    let mut parts: Vec<String> = nodes
        .iter()
        .filter(|n| !is_chrome_node(n))
        .filter_map(|n| {
            let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("?");
            let hittable = n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true);
            let enabled = n.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            if !hittable || !enabled {
                return None;
            }
            let id = n
                .get("identifier")
                .and_then(|v| v.as_str())
                .or_else(|| n.get("id").and_then(|v| v.as_str()))
                .unwrap_or("");
            let label = n.get("label").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() && label.is_empty() {
                return None;
            }
            let key = if !id.is_empty() {
                format!("{role}#{id}")
            } else {
                format!("{role}~{label}")
            };
            Some(key.to_ascii_lowercase())
        })
        .collect();
    parts.sort();
    parts.dedup();
    let body = parts.join("|");
    format!("fp_{:016x}", fnv1a64(body.as_bytes()))
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Inferred affordance kind for agent planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffordanceKind {
    TextField,
    SecureField,
    SearchField,
    PrimaryButton,
    Button,
    Link,
    Switch,
    Cell,
    NavBack,
    StaticText,
    Other,
}

/// One actionable element in the agent world model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Affordance {
    pub kind: AffordanceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default = "default_true")]
    pub hittable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center_norm: Option<Value>,
}

fn default_true() -> bool {
    true
}

fn is_primary_cta_label(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    [
        "login", "log in", "sign in", "submit", "continue", "next", "done", "go", "ok", "save",
        "invia", "send", "avanti", "fatto", "accedi", "conferma",
    ]
    .iter()
    .any(|k| l == *k || l.contains(k))
}

fn is_nav_back_label(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    ["back", "indietro", "annulla", "cancel", "close", "chiudi"]
        .iter()
        .any(|k| l == *k || l.starts_with(&format!("{k} ")))
}

/// Map AX nodes → typed affordances (capped).
pub fn infer_affordances(nodes: &[Value], cap: usize) -> Vec<Affordance> {
    let top = build_actionable_topk(nodes, cap.saturating_mul(2));
    let mut out = Vec::new();
    for n in top.into_iter().take(cap) {
        let role = n
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let label = n
            .get("label")
            .and_then(|v| v.as_str())
            .or_else(|| n.get("text").and_then(|v| v.as_str()))
            .map(|s| s.to_string());
        let identifier = n
            .get("identifier")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let tree_id = n.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let id = identifier.or(tree_id);
        let value = n
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let focused = n.get("focused").and_then(|v| v.as_bool()).unwrap_or(false);
        let hittable = n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true);
        let center_norm = n.get("center_norm").cloned();

        let lab_str = label.as_deref().unwrap_or("");
        let kind = if is_nav_back_label(lab_str) {
            AffordanceKind::NavBack
        } else if role.contains("secure") {
            AffordanceKind::SecureField
        } else if is_editable_role(&role) {
            if lab_str.to_ascii_lowercase().contains("search")
                || lab_str.to_ascii_lowercase().contains("cerca")
                || role.contains("search")
            {
                AffordanceKind::SearchField
            } else {
                AffordanceKind::TextField
            }
        } else if role.contains("button") {
            if is_primary_cta_label(lab_str) {
                AffordanceKind::PrimaryButton
            } else {
                AffordanceKind::Button
            }
        } else if role.contains("link") {
            AffordanceKind::Link
        } else if role.contains("switch") {
            AffordanceKind::Switch
        } else if role.contains("cell") {
            AffordanceKind::Cell
        } else if role.contains("static") && !lab_str.is_empty() {
            AffordanceKind::StaticText
        } else {
            AffordanceKind::Other
        };

        out.push(Affordance {
            kind,
            label,
            id,
            value,
            focused,
            hittable,
            center_norm,
        });
    }
    out
}

/// Post-action expectation (optional — host verifies when set).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Expectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub see_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub see_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_changed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within_ms: Option<u64>,
}

/// Agent-facing settled world model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceiveView {
    pub ready: bool,
    pub eyes_unusable: bool,
    pub location: LocationView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocking: Option<BlockingView>,
    pub affordances: Vec<Affordance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub since_last: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationView {
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingView {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Evidence bundle returned by `attempt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptEvidence {
    pub pre_fingerprint: String,
    pub post_fingerprint: String,
    pub fingerprint_changed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delta_events: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspect: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<Hypothesis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub kind: String,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Full attempt verdict for agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptVerdict {
    pub intent_met: bool,
    pub intent: String,
    pub motor_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<String>,
    pub evidence: AttemptEvidence,
    pub perceive_after: PerceiveView,
}

/// Build agent world model from a settled observe snapshot.
pub fn build_perceive(snap: &ObserveSnapshot, since_last: &[SenseEvent]) -> PerceiveView {
    let nodes = snap.accessibility_tree.nodes();
    let fp = screen_fingerprint(nodes);
    let scene = snap.scene.as_ref();
    let surface = scene.and_then(|s| s.surface.clone());
    let title = scene.and_then(|s| s.screen_title.clone());
    let bundle_id = snap
        .app_bundle_id
        .clone()
        .or_else(|| scene.and_then(|s| s.bundle_id.clone()));

    let blocking = snap.overlay.as_ref().and_then(|o| {
        if o == "none" {
            None
        } else {
            Some(BlockingView {
                kind: o.clone(),
                detail: scene.and_then(|s| {
                    if !s.alerts.is_empty() {
                        Some(s.alerts.join("; "))
                    } else if !s.sheets.is_empty() {
                        Some(s.sheets.join("; "))
                    } else {
                        None
                    }
                }),
            })
        }
    });

    let since: Vec<String> = since_last
        .iter()
        .map(|e| e.kind.clone())
        .collect();

    PerceiveView {
        ready: snap.is_actionable_eyes() && !snap.eyes_unusable,
        eyes_unusable: snap.eyes_unusable,
        location: LocationView {
            fingerprint: fp,
            surface,
            title,
            bundle_id,
        },
        blocking,
        affordances: infer_affordances(nodes, 24),
        since_last: since,
    }
}

fn event_summary(events: &[SenseEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            if let Some(p) = &e.payload {
                if let Some(id) = p.get("id").and_then(|v| v.as_str()) {
                    return format!("{}:{id}", e.kind);
                }
                if let Some(label) = p.get("label").and_then(|v| v.as_str()) {
                    return format!("{}:{label}", e.kind);
                }
            }
            e.kind.clone()
        })
        .collect()
}

fn node_has_id(nodes: &[Value], needle: &str) -> bool {
    nodes.iter().any(|n| {
        n.get("identifier").and_then(|v| v.as_str()) == Some(needle)
            || n.get("id").and_then(|v| v.as_str()) == Some(needle)
    })
}

fn node_has_label_contains(nodes: &[Value], needle: &str) -> bool {
    let needle_lc = needle.to_ascii_lowercase();
    nodes.iter().any(|node| {
        node.get("label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase().contains(&needle_lc))
            .unwrap_or(false)
            || node
                .get("identifier")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase().contains(&needle_lc))
                .unwrap_or(false)
    })
}

fn similar_identifiers(nodes: &[Value], wanted: &str) -> Vec<String> {
    let w = wanted.to_ascii_lowercase();
    let mut out = Vec::new();
    for n in nodes {
        if let Some(id) = n.get("identifier").and_then(|v| v.as_str()) {
            let l = id.to_ascii_lowercase();
            if l == w {
                continue;
            }
            let common = w
                .chars()
                .zip(l.chars())
                .take_while(|(a, b)| a == b)
                .count();
            if common >= 5 || l.contains(&w) || w.contains(&l) {
                out.push(id.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Evaluate post-action state against expectation; build evidence + hypotheses.
pub fn evaluate_attempt(
    intent: &str,
    motor_ok: bool,
    pre: &ObserveSnapshot,
    post: &ObserveSnapshot,
    events: &[SenseEvent],
    expect: Option<&Expectation>,
    target_id: Option<&str>,
    target_label: Option<&str>,
) -> AttemptVerdict {
    let pre_nodes = pre.accessibility_tree.nodes();
    let post_nodes = post.accessibility_tree.nodes();
    let pre_fp = screen_fingerprint(pre_nodes);
    let post_fp = screen_fingerprint(post_nodes);
    let fp_changed = pre_fp != post_fp;
    let delta = event_summary(events);

    let mut missing = Vec::new();
    let mut intent_met = motor_ok;
    let mut suspect: Option<String> = None;
    let mut hypotheses = Vec::new();

    if let Some(exp) = expect {
        if let Some(ref sid) = exp.see_id {
            if !node_has_id(post_nodes, sid) {
                missing.push(format!("see_id:{sid}"));
                intent_met = false;
                let similar = similar_identifiers(post_nodes, sid);
                if !similar.is_empty() {
                    hypotheses.push(Hypothesis {
                        kind: "a11y_id_mismatch".into(),
                        confidence: 0.75,
                        detail: Some(format!("expected {sid}, saw similar: {}", similar.join(", "))),
                    });
                }
            }
        }
        if let Some(ref slab) = exp.see_label {
            if !node_has_label_contains(post_nodes, slab) {
                missing.push(format!("see_label:{slab}"));
                intent_met = false;
            }
        }
        if let Some(ref surf) = exp.surface {
            let got = post
                .scene
                .as_ref()
                .and_then(|s| s.surface.as_deref())
                .unwrap_or("");
            if got != surf.as_str() {
                missing.push(format!("surface:{surf}"));
                intent_met = false;
            }
        }
        if exp.fingerprint_changed == Some(true) && !fp_changed {
            missing.push("fingerprint_changed".into());
            intent_met = false;
        }
    }

    if motor_ok && !fp_changed && intent == "tap" {
        if expect.is_some() && !missing.is_empty() {
            suspect = Some(
                "tap reached control but screen fingerprint unchanged — check handler or wrong target"
                    .into(),
            );
            if hypotheses.is_empty() {
                hypotheses.push(Hypothesis {
                    kind: "silent_tap".into(),
                    confidence: 0.6,
                    detail: target_id.map(|s| format!("target_id={s}")),
                });
            }
        }
    }

    if motor_ok
        && delta.iter().any(|d| d.starts_with("value_changed"))
        && missing.iter().any(|m| m.starts_with("see_"))
    {
        hypotheses.push(Hypothesis {
            kind: "backend_rejection".into(),
            confidence: 0.45,
            detail: Some("fields updated but expected chrome missing — check ViewModel/API".into()),
        });
        if suspect.is_none() {
            suspect = Some(
                "input accepted but navigation/assertion failed — likely app logic not accessibility"
                    .into(),
            );
        }
    }

    if !motor_ok {
        intent_met = false;
    }

    if post.eyes_unusable {
        hypotheses.push(Hypothesis {
            kind: "eyes_unusable".into(),
            confidence: 0.9,
            detail: Some(format!("ax_quality={}", post.ax_quality)),
        });
    }

    if let Some(ov) = &post.overlay {
        if ov != "none" && !missing.is_empty() {
            hypotheses.push(Hypothesis {
                kind: "overlay_blocking".into(),
                confidence: 0.55,
                detail: Some(ov.clone()),
            });
        }
    }

    if hypotheses.is_empty() && !intent_met {
        if let Some(id) = target_id {
            if !node_has_id(pre_nodes, id) {
                hypotheses.push(Hypothesis {
                    kind: "target_missing".into(),
                    confidence: 0.8,
                    detail: Some(format!("id {id} not in pre-action tree")),
                });
            }
        }
        if let Some(lab) = target_label {
            if !node_has_label_contains(pre_nodes, lab) {
                hypotheses.push(Hypothesis {
                    kind: "label_missing".into(),
                    confidence: 0.7,
                    detail: Some(format!("label {lab} not found")),
                });
            }
        }
    }

    let fault = if intent_met {
        None
    } else if !motor_ok {
        Some("motor_failed".into())
    } else {
        Some("intent_unmet".into())
    };

    let perceive_after = build_perceive(post, events);

    AttemptVerdict {
        intent_met,
        intent: intent.into(),
        motor_ok,
        fault,
        evidence: AttemptEvidence {
            pre_fingerprint: pre_fp,
            post_fingerprint: post_fp,
            fingerprint_changed: fp_changed,
            delta_events: delta,
            missing,
            suspect,
            hypotheses,
        },
        perceive_after,
    }
}

/// Parse expectation from JSON value (MCP / RPC).
pub fn parse_expectation(v: Option<&Value>) -> Option<Expectation> {
    let v = v?;
    serde_json::from_value(v.clone()).ok()
}

/// Convenience: fingerprint from observe snapshot.
pub fn fingerprint_of(snap: &ObserveSnapshot) -> String {
    screen_fingerprint(snap.accessibility_tree.nodes())
}

/// Surface string from nodes (re-export path for tests).
pub fn surface_of_nodes(nodes: &[Value]) -> String {
    detect_surface(nodes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{AccessibilityTree, ObserveSnapshot};
    use serde_json::json;

    fn snap_from_nodes(nodes: Vec<Value>) -> ObserveSnapshot {
        let mut s = ObserveSnapshot {
            schema_version: 2,
            udid: "test".into(),
            booted: true,
            simulator_app_running: false,
            frame: None,
            app_bundle_id: Some("com.test.app".into()),
            accessibility_tree: AccessibilityTree::Available {
                nodes: nodes.clone(),
                root: None,
                element_count: Some(nodes.len()),
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
        };
        s.enrich_v2();
        s
    }

    #[test]
    fn fingerprint_stable_and_order_independent() {
        let a = vec![
            json!({"role":"AXButton","identifier":"loginButton","label":"Login","hittable":true,"enabled":true}),
            json!({"role":"AXTextField","identifier":"user","label":"Username","hittable":true,"enabled":true}),
        ];
        let b = vec![a[1].clone(), a[0].clone()];
        assert_eq!(screen_fingerprint(&a), screen_fingerprint(&b));
        assert!(screen_fingerprint(&a).starts_with("fp_"));
    }

    #[test]
    fn fingerprint_changes_when_structure_changes() {
        let before = vec![
            json!({"role":"AXButton","identifier":"loginButton","label":"Login","hittable":true,"enabled":true}),
        ];
        let after = vec![
            json!({"role":"AXStaticText","identifier":"homeTitle","label":"Home","hittable":true,"enabled":true}),
        ];
        assert_ne!(screen_fingerprint(&before), screen_fingerprint(&after));
    }

    #[test]
    fn infer_primary_button_and_fields() {
        let nodes = vec![
            json!({"id":"n1","role":"AXTextField","identifier":"usernameTextField","label":"Username","hittable":true,"enabled":true}),
            json!({"id":"n2","role":"AXSecureTextField","identifier":"passwordSecureField","label":"Password","hittable":true,"enabled":true}),
            json!({"id":"n3","role":"AXButton","identifier":"loginButton","label":"Login","hittable":true,"enabled":true}),
        ];
        let aff = infer_affordances(&nodes, 10);
        assert!(aff.iter().any(|a| a.kind == AffordanceKind::PrimaryButton));
        assert!(aff.iter().any(|a| a.kind == AffordanceKind::SecureField));
        assert!(aff.iter().any(|a| a.kind == AffordanceKind::TextField));
    }

    #[test]
    fn evaluate_attempt_detects_id_typo_hypothesis() {
        let pre = snap_from_nodes(vec![
            json!({"role":"AXButton","identifier":"loginBtnTypo","label":"Login","hittable":true,"enabled":true}),
        ]);
        let post = pre.clone();
        let exp = Expectation {
            see_id: Some("loginButton".into()),
            ..Default::default()
        };
        let v = evaluate_attempt(
            "tap",
            true,
            &pre,
            &post,
            &[],
            Some(&exp),
            Some("loginButton"),
            Some("Login"),
        );
        assert!(!v.intent_met);
        assert!(v
            .evidence
            .hypotheses
            .iter()
            .any(|h| h.kind == "a11y_id_mismatch"));
    }

    #[test]
    fn build_perceive_ready_when_settled() {
        let nodes = vec![
            json!({"role":"AXButton","identifier":"Go","label":"Go","hittable":true,"enabled":true}),
            json!({"role":"AXButton","identifier":"More","label":"More","hittable":true,"enabled":true}),
            json!({"role":"AXButton","identifier":"Extra1","label":"Extra1","hittable":true,"enabled":true}),
            json!({"role":"AXButton","identifier":"Extra2","label":"Extra2","hittable":true,"enabled":true}),
        ];
        let snap = snap_from_nodes(nodes);
        let p = build_perceive(&snap, &[]);
        assert!(p.ready);
        assert!(!p.affordances.is_empty());
        assert!(p.location.fingerprint.starts_with("fp_"));
    }
}
