//! UX Graph — computational, persistent model of user experience.
//!
//! Screens are nodes (fingerprint + affordances). Actions are edges (transitions with evidence).
//! Baselines enable regress diffs; source hints link fingerprints to Swift files over time.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::qa::{Affordance, AffordanceKind, AttemptVerdict, PerceiveView};

pub const UXGRAPH_SCHEMA_VERSION: u32 = 1;

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Default path: `<workspace>/.ligh/uxgraph.json`
pub fn default_graph_path(workspace: &Path) -> PathBuf {
    workspace.join(".ligh").join("uxgraph.json")
}

/// Resolve workspace from env `LIGH_WORKSPACE` or current dir.
pub fn resolve_workspace(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(w) = std::env::var("LIGH_WORKSPACE") {
        if !w.is_empty() {
            return PathBuf::from(w);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceHint {
    pub path: String,
    pub confidence: f64,
    pub edits: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_edit_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UxScreenNode {
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affordance_labels: Vec<String>,
    pub first_seen_ms: f64,
    pub last_seen_ms: f64,
    pub visit_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_hints: Vec<SourceHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UxTransitionEdge {
    pub from_fp: String,
    pub to_fp: String,
    pub intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    pub intent_met: bool,
    pub count: u32,
    pub last_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fault: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypothesis_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UxBaseline {
    pub name: String,
    pub created_ms: f64,
    pub fingerprints: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub nodes: HashMap<String, UxScreenNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UxGraph {
    #[serde(default = "uxgraph_schema_default")]
    pub schema_version: u32,
    pub updated_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub nodes: HashMap<String, UxScreenNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<UxTransitionEdge>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub baselines: HashMap<String, UxBaseline>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_baseline: Option<String>,
    pub stats: UxGraphStats,
}

fn uxgraph_schema_default() -> u32 {
    UXGRAPH_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UxGraphStats {
    pub total_perceives: u64,
    pub total_attempts: u64,
    pub intent_met: u64,
    pub intent_unmet: u64,
    pub explores: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenChange {
    pub fingerprint: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDiff {
    pub baseline: String,
    pub new_screens: Vec<String>,
    pub removed_screens: Vec<String>,
    pub changed_screens: Vec<ScreenChange>,
    pub new_transitions: u32,
    pub regress_pass: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreStep {
    pub step: u32,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    pub from_fp: String,
    pub to_fp: String,
    pub intent_met: bool,
    pub new_screen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreResult {
    pub steps_taken: u32,
    pub screens_discovered: u32,
    pub transitions_recorded: u32,
    pub trace: Vec<ExploreStep>,
    pub graph_summary: GraphSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSummary {
    pub node_count: usize,
    pub edge_count: usize,
    pub baseline: Option<String>,
    pub fingerprints: Vec<String>,
}

impl UxGraph {
    pub fn new() -> Self {
        Self {
            schema_version: UXGRAPH_SCHEMA_VERSION,
            updated_ms: now_secs(),
            app_bundle_id: None,
            nodes: HashMap::new(),
            edges: Vec::new(),
            baselines: HashMap::new(),
            active_baseline: None,
            stats: UxGraphStats::default(),
        }
    }

    pub fn load(path: &Path) -> crate::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let raw = std::fs::read_to_string(path)?;
        let g: Self = serde_json::from_str(&raw)?;
        Ok(g)
    }

    pub fn save(&mut self, path: &Path) -> crate::Result<()> {
        self.updated_ms = now_secs();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn summary(&self) -> GraphSummary {
        GraphSummary {
            node_count: self.nodes.len(),
            edge_count: self.edges.len(),
            baseline: self.active_baseline.clone(),
            fingerprints: self.nodes.keys().cloned().collect(),
        }
    }

    pub fn record_perceive(&mut self, view: &PerceiveView) {
        self.stats.total_perceives += 1;
        if let Some(bid) = &view.location.bundle_id {
            if self.app_bundle_id.is_none() {
                self.app_bundle_id = Some(bid.clone());
            }
        }
        self.upsert_screen(view, None);
    }

    pub fn record_attempt(
        &mut self,
        pre: &PerceiveView,
        verdict: &AttemptVerdict,
        target_label: Option<&str>,
        target_id: Option<&str>,
    ) {
        self.stats.total_attempts += 1;
        if verdict.intent_met {
            self.stats.intent_met += 1;
        } else {
            self.stats.intent_unmet += 1;
        }
        self.upsert_screen(pre, None);
        self.upsert_screen(&verdict.perceive_after, None);
        self.record_edge(pre, verdict, target_label, target_id);
    }

    fn upsert_screen(&mut self, view: &PerceiveView, extra_hints: Option<&[SourceHint]>) {
        let fp = view.location.fingerprint.clone();
        let now = now_secs();
        let affordance_labels: Vec<String> = view
            .affordances
            .iter()
            .filter_map(|a| a.label.clone().or_else(|| a.id.clone()))
            .collect();

        self.nodes
            .entry(fp.clone())
            .and_modify(|n| {
                n.last_seen_ms = now;
                n.visit_count += 1;
                n.surface = view.location.surface.clone().or(n.surface.clone());
                n.title = view.location.title.clone().or(n.title.clone());
                n.bundle_id = view
                    .location
                    .bundle_id
                    .clone()
                    .or(n.bundle_id.clone());
                if !affordance_labels.is_empty() {
                    n.affordance_labels = affordance_labels.clone();
                }
                if let Some(hints) = extra_hints {
                    merge_hints(&mut n.source_hints, hints);
                }
            })
            .or_insert_with(|| UxScreenNode {
                fingerprint: fp,
                surface: view.location.surface.clone(),
                title: view.location.title.clone(),
                bundle_id: view.location.bundle_id.clone(),
                affordance_labels,
                first_seen_ms: now,
                last_seen_ms: now,
                visit_count: 1,
                source_hints: extra_hints.unwrap_or(&[]).to_vec(),
            });
    }

    fn record_edge(
        &mut self,
        pre: &PerceiveView,
        verdict: &AttemptVerdict,
        target_label: Option<&str>,
        target_id: Option<&str>,
    ) {
        let from_fp = pre.location.fingerprint.clone();
        let to_fp = verdict.perceive_after.location.fingerprint.clone();
        let hyps: Vec<String> = verdict
            .evidence
            .hypotheses
            .iter()
            .map(|h| h.kind.clone())
            .collect();
        let target_label = target_label.map(|s| s.to_string());
        let target_id = target_id.map(|s| s.to_string());

        if let Some(edge) = self.edges.iter_mut().find(|e| {
            e.from_fp == from_fp
                && e.to_fp == to_fp
                && e.intent == verdict.intent
                && e.target_label == target_label
                && e.target_id == target_id
        }) {
            edge.count += 1;
            edge.last_ms = now_secs();
            edge.intent_met = verdict.intent_met;
            edge.last_fault = verdict.fault.clone();
            edge.hypothesis_kinds = hyps;
        } else {
            self.edges.push(UxTransitionEdge {
                from_fp,
                to_fp,
                intent: verdict.intent.clone(),
                target_label,
                target_id,
                intent_met: verdict.intent_met,
                count: 1,
                last_ms: now_secs(),
                last_fault: verdict.fault.clone(),
                hypothesis_kinds: hyps,
            });
        }
    }

    /// Correlate a source file edit with the current (or given) screen fingerprint.
    pub fn record_source_hint(&mut self, fingerprint: &str, source_path: &str) {
        let now = now_secs();
        let fp = fingerprint.to_string();
        let path = source_path.to_string();
        if let Some(node) = self.nodes.get_mut(&fp) {
            bump_hint(&mut node.source_hints, &path, now);
        } else {
            self.nodes.insert(
                fp.clone(),
                UxScreenNode {
                    fingerprint: fp,
                    surface: None,
                    title: None,
                    bundle_id: None,
                    affordance_labels: vec![],
                    first_seen_ms: now,
                    last_seen_ms: now,
                    visit_count: 0,
                    source_hints: vec![SourceHint {
                        path,
                        confidence: 0.35,
                        edits: 1,
                        last_edit_ms: Some(now),
                    }],
                },
            );
        }
    }

    pub fn source_hints_for(&self, fingerprint: &str) -> Vec<SourceHint> {
        self.nodes
            .get(fingerprint)
            .map(|n| {
                let mut h = n.source_hints.clone();
                h.sort_by(|a, b| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                h
            })
            .unwrap_or_default()
    }

    pub fn set_baseline(&mut self, name: &str) {
        let snap = UxBaseline {
            name: name.to_string(),
            created_ms: now_secs(),
            fingerprints: self.nodes.keys().cloned().collect(),
            nodes: self.nodes.clone(),
        };
        self.baselines.insert(name.to_string(), snap);
        self.active_baseline = Some(name.to_string());
    }

    pub fn regress(&self, baseline_name: &str, current: &PerceiveView) -> GraphDiff {
        let baseline = self.baselines.get(baseline_name);
        let base_fps: HashSet<String> = baseline
            .map(|b| b.fingerprints.iter().cloned().collect())
            .unwrap_or_default();
        let current_fp = current.location.fingerprint.clone();

        let mut new_screens = Vec::new();
        let mut removed_screens = Vec::new();
        let mut changed_screens = Vec::new();

        if !base_fps.is_empty() {
            for fp in self.nodes.keys() {
                if !base_fps.contains(fp) {
                    new_screens.push(fp.clone());
                }
            }
            for fp in &base_fps {
                if !self.nodes.contains_key(fp) && *fp != current_fp {
                    removed_screens.push(fp.clone());
                }
            }
        }

        if let Some(base) = baseline {
            if let Some(base_node) = base.nodes.get(&current_fp) {
                let cur_labels: HashSet<_> = current
                    .affordances
                    .iter()
                    .filter_map(|a| a.label.clone().or_else(|| a.id.clone()))
                    .collect();
                let base_labels: HashSet<_> = base_node.affordance_labels.iter().cloned().collect();
                if cur_labels != base_labels {
                    let added: Vec<_> = cur_labels.difference(&base_labels).cloned().collect();
                    let removed: Vec<_> = base_labels.difference(&cur_labels).cloned().collect();
                    changed_screens.push(ScreenChange {
                        fingerprint: current_fp.clone(),
                        kind: "affordances_changed".into(),
                        detail: Some(format!("added={added:?} removed={removed:?}")),
                    });
                }
                if base_node.surface != current.location.surface {
                    changed_screens.push(ScreenChange {
                        fingerprint: current_fp.clone(),
                        kind: "surface_changed".into(),
                        detail: Some(format!(
                            "{:?} -> {:?}",
                            base_node.surface, current.location.surface
                        )),
                    });
                }
            } else if !base_fps.is_empty() {
                new_screens.push(current_fp.clone());
            }
        }

        let regress_pass =
            new_screens.is_empty() && removed_screens.is_empty() && changed_screens.is_empty();

        GraphDiff {
            baseline: baseline_name.to_string(),
            new_screens,
            removed_screens,
            changed_screens,
            new_transitions: self.edges.len() as u32,
            regress_pass,
        }
    }

    /// Pick next safe affordance to explore from current screen (BFS helper).
    pub fn plan_explore_tap(
        &self,
        current: &PerceiveView,
        tried: &HashSet<String>,
    ) -> Option<(Affordance, String)> {
        for aff in &current.affordances {
            if !is_safe_explore_kind(aff.kind) {
                continue;
            }
            let key = explore_key(aff);
            if tried.contains(&key) {
                continue;
            }
            if is_destructive_label(aff.label.as_deref().unwrap_or("")) {
                continue;
            }
            return Some((aff.clone(), key));
        }
        None
    }

    pub fn count_new_screens_after(&self, before_count: usize) -> u32 {
        (self.nodes.len().saturating_sub(before_count)) as u32
    }
}

fn merge_hints(dst: &mut Vec<SourceHint>, src: &[SourceHint]) {
    for h in src {
        bump_hint(dst, &h.path, h.last_edit_ms.unwrap_or_else(now_secs));
    }
}

fn bump_hint(hints: &mut Vec<SourceHint>, path: &str, now: f64) {
    if let Some(h) = hints.iter_mut().find(|h| h.path == path) {
        h.edits += 1;
        h.last_edit_ms = Some(now);
        h.confidence = (0.35 + (h.edits as f64) * 0.12).min(0.95);
    } else {
        hints.push(SourceHint {
            path: path.to_string(),
            confidence: 0.35,
            edits: 1,
            last_edit_ms: Some(now),
        });
    }
}

pub fn is_safe_explore_kind(kind: AffordanceKind) -> bool {
    matches!(
        kind,
        AffordanceKind::Button
            | AffordanceKind::PrimaryButton
            | AffordanceKind::Cell
            | AffordanceKind::Link
            | AffordanceKind::StaticText
    )
}

pub fn is_destructive_label(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    [
        "delete", "elimina", "remove", "logout", "sign out", "esci", "cancel account",
        "purchase", "buy", "pay", "compra",
    ]
    .iter()
    .any(|k| l.contains(k))
}

fn explore_key(aff: &Affordance) -> String {
    format!(
        "{:?}:{}:{}",
        aff.kind,
        aff.id.as_deref().unwrap_or(""),
        aff.label.as_deref().unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qa::{
        Affordance, AffordanceKind, AttemptEvidence, AttemptVerdict, LocationView, PerceiveView,
    };

    fn perceive(fp: &str, labels: &[&str]) -> PerceiveView {
        PerceiveView {
            ready: true,
            eyes_unusable: false,
            location: LocationView {
                fingerprint: fp.into(),
                surface: Some("app".into()),
                title: Some("Test".into()),
                bundle_id: Some("com.test".into()),
            },
            blocking: None,
            affordances: labels
                .iter()
                .map(|l| Affordance {
                    kind: AffordanceKind::Button,
                    label: Some(l.to_string()),
                    id: None,
                    value: None,
                    focused: false,
                    hittable: true,
                    center_norm: None,
                })
                .collect(),
            since_last: vec![],
        }
    }

    fn verdict(pre: &PerceiveView, post_fp: &str, met: bool) -> AttemptVerdict {
        AttemptVerdict {
            intent_met: met,
            intent: "tap".into(),
            motor_ok: true,
            fault: if met { None } else { Some("intent_unmet".into()) },
            evidence: AttemptEvidence {
                pre_fingerprint: pre.location.fingerprint.clone(),
                post_fingerprint: post_fp.into(),
                fingerprint_changed: pre.location.fingerprint != post_fp,
                delta_events: vec![],
                missing: vec![],
                suspect: None,
                hypotheses: vec![],
            },
            perceive_after: perceive(post_fp, &["Home"]),
        }
    }

    #[test]
    fn graph_records_nodes_and_edges() {
        let mut g = UxGraph::new();
        let pre = perceive("fp_a", &["Login"]);
        let v = verdict(&pre, "fp_b", true);
        g.record_attempt(&pre, &v, Some("Login"), None);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.stats.intent_met, 1);
    }

    #[test]
    fn baseline_regress_detects_new_screen() {
        let mut g = UxGraph::new();
        g.record_perceive(&perceive("fp_a", &["Login"]));
        g.set_baseline("v1");
        g.record_perceive(&perceive("fp_b", &["Home"]));
        let diff = g.regress("v1", &perceive("fp_b", &["Home"]));
        assert!(!diff.regress_pass);
        assert!(diff.new_screens.contains(&"fp_b".to_string()));
    }

    #[test]
    fn source_hint_increases_confidence() {
        let mut g = UxGraph::new();
        g.record_perceive(&perceive("fp_login", &["Login"]));
        g.record_source_hint("fp_login", "ContentView.swift");
        g.record_source_hint("fp_login", "ContentView.swift");
        let hints = g.source_hints_for("fp_login");
        assert_eq!(hints[0].path, "ContentView.swift");
        assert!(hints[0].confidence > 0.45);
        assert_eq!(hints[0].edits, 2);
    }

    #[test]
    fn explore_skips_destructive_labels() {
        let g = UxGraph::new();
        let view = PerceiveView {
            ready: true,
            eyes_unusable: false,
            location: LocationView {
                fingerprint: "fp".into(),
                surface: None,
                title: None,
                bundle_id: None,
            },
            blocking: None,
            affordances: vec![
                Affordance {
                    kind: AffordanceKind::Button,
                    label: Some("Delete account".into()),
                    id: None,
                    value: None,
                    focused: false,
                    hittable: true,
                    center_norm: None,
                },
                Affordance {
                    kind: AffordanceKind::Button,
                    label: Some("Settings".into()),
                    id: None,
                    value: None,
                    focused: false,
                    hittable: true,
                    center_norm: None,
                },
            ],
            since_last: vec![],
        };
        let tried = HashSet::new();
        let (aff, _) = g.plan_explore_tap(&view, &tried).unwrap();
        assert_eq!(aff.label.as_deref(), Some("Settings"));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ligh-uxgraph-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("uxgraph.json");
        let mut g = UxGraph::new();
        g.record_perceive(&perceive("fp_x", &["Go"]));
        g.save(&path).unwrap();
        let g2 = UxGraph::load(&path).unwrap();
        assert!(g2.nodes.contains_key("fp_x"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
