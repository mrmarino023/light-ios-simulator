//! Speculative navigation — optimistic UI agency (speculative decoding for scenes).
//!
//! Fire → predict (L0 GoalSpec / L1 enter-exit) → poll settle evidence →
//! **Certified** (100%) or **RolledBack**. Never claim success while Speculative.
//!
//! Soundness requires C1–C10 in ARCHITECTURE.md. This module enforces the host
//! side: one outstanding speculation, host-side match, fail-closed forbid.
//!
//! Research contract: `LIGH_SPECULATE=0` disables optimism (ablation baseline).
//! Predictions must be falsifiable — `expect_enter`/`expect_exit` are checked
//! on certify for L1 (not decorative).

use serde::{Deserialize, Serialize};

use crate::autopilot::{PilotAct, PilotGoal, PilotIntent};
use crate::feel::{FeelIR, FeelPhase};
use crate::observe::ObserveSnapshot;
use crate::qa::AffordanceKind;

pub const SPECULATE_SCHEMA_VERSION: u32 = 1;

/// Ablation kill-switch. Default on. Set `LIGH_SPECULATE=0|off|false` for H0.
pub fn speculate_enabled() -> bool {
    match std::env::var("LIGH_SPECULATE") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "off" || v == "false" || v == "no")
        }
        Err(_) => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecLevel {
    /// GoalSpec predicates after the act.
    L0,
    /// L0 + identity enter/exit sets.
    L1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecPhase {
    Idle,
    Speculative,
    Certified,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecVerdict {
    Pending,
    Certified,
    Rejected,
    Forbidden,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecPrediction {
    pub level: SpecLevel,
    pub schema: u32,
    pub expect_goal: bool,
    pub expect_fp_change: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect_enter: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect_exit: Vec<String>,
    pub from_fp: String,
    pub act_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_scene_fp: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from_region_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecTicket {
    pub phase: SpecPhase,
    pub pred: SpecPrediction,
    pub deadline_unix_ms: u64,
    pub started_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preplanned: Option<crate::autopilot::PilotAct>,
}

impl SpecTicket {
    pub fn is_open(&self) -> bool {
        matches!(self.phase, SpecPhase::Speculative)
    }
}

/// Aggregate counters for ablation reports (from autopilot trace events).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpecStats {
    pub begins: u32,
    pub certified: u32,
    pub rejected: u32,
    pub forbidden: u32,
    pub preplan: u32,
    pub fire_preplanned: u32,
    pub preplan_stale: u32,
}

impl SpecStats {
    pub fn from_trace(trace: &[serde_json::Value]) -> Self {
        let mut s = Self::default();
        for ev in trace {
            let Some(name) = ev.get("event").and_then(|v| v.as_str()) else {
                continue;
            };
            match name {
                "speculate_begin" => s.begins += 1,
                "speculate_end" => match ev.get("verdict").and_then(|v| v.as_str()) {
                    Some("certified") => s.certified += 1,
                    Some("rejected") => s.rejected += 1,
                    Some("forbidden") => s.forbidden += 1,
                    _ => {}
                },
                "speculate_preplan" => s.preplan += 1,
                "speculate_fire_preplanned" => s.fire_preplanned += 1,
                "speculate_preplan_stale" => s.preplan_stale += 1,
                _ => {}
            }
        }
        s
    }

    pub fn certify_rate(&self) -> Option<f64> {
        let denom = self.certified + self.rejected;
        if denom == 0 {
            None
        } else {
            Some(f64::from(self.certified) / f64::from(denom))
        }
    }
}

pub fn may_speculate(feel: &FeelIR, outstanding: bool) -> bool {
    if !speculate_enabled() {
        return false;
    }
    if outstanding {
        return false;
    }
    if feel.feel.eyes_unusable_like() {
        return false;
    }
    match feel.feel.phase {
        FeelPhase::EyesUnusable => false,
        FeelPhase::Blocked => false,
        FeelPhase::Transition => true,
        FeelPhase::Settled => true,
    }
}

trait FeelMetaExt {
    fn eyes_unusable_like(&self) -> bool;
}

impl FeelMetaExt for crate::feel::FeelMeta {
    fn eyes_unusable_like(&self) -> bool {
        !self.ready || matches!(self.phase, FeelPhase::EyesUnusable)
    }
}

fn predicate_needles(goal: &PilotGoal) -> (Vec<String>, Vec<String>) {
    let enter: Vec<String> = goal
        .required_predicates()
        .into_iter()
        .filter_map(|p| p.identity.or(p.id).or(p.label))
        .collect();
    let exit: Vec<String> = goal
        .none
        .iter()
        .filter_map(|p| {
            p.identity
                .clone()
                .or_else(|| p.id.clone())
                .or_else(|| p.label.clone())
        })
        .collect();
    (enter, exit)
}

fn surface_has_needle(feel: &FeelIR, snap: &ObserveSnapshot, needle: &str) -> bool {
    if feel.world.elements.iter().any(|el| {
        el.identifier.as_deref() == Some(needle)
            || el.label.as_deref() == Some(needle)
            || el
                .label
                .as_deref()
                .is_some_and(|lab| crate::observe::identity_suggests_tab_label(needle, lab))
    }) {
        return true;
    }
    if feel.salience.iter().any(|s| {
        s.id.as_deref() == Some(needle)
            || s.label.as_deref() == Some(needle)
            || s.label
                .as_deref()
                .is_some_and(|lab| crate::observe::identity_suggests_tab_label(needle, lab))
    }) {
        return true;
    }
    snap.accessibility_tree.nodes().iter().any(|n| {
        crate::observe::node_matches_identity_needle(n, needle)
            || crate::observe::node_matches_identifier(n, needle)
            || n.get("label").and_then(|v| v.as_str()) == Some(needle)
    })
}

fn l1_identities_hold(pred: &SpecPrediction, feel: &FeelIR, snap: &ObserveSnapshot) -> bool {
    if pred.level != SpecLevel::L1 {
        return true;
    }
    for needle in &pred.expect_enter {
        if !surface_has_needle(feel, snap, needle) {
            return false;
        }
    }
    for needle in &pred.expect_exit {
        if surface_has_needle(feel, snap, needle) {
            return false;
        }
    }
    true
}

/// L0/L1 prediction from act + goal. Prefer [`predict_after_act_on_feel`].
pub fn predict_after_act(goal: &PilotGoal, act: &PilotAct, from_fp: &str) -> SpecPrediction {
    let (enter, exit) = predicate_needles(goal);
    let progress_tap = matches!(act.intent, PilotIntent::Tap | PilotIntent::Dismiss)
        && (matches!(act.kind, Some(AffordanceKind::PrimaryButton)) || act_looks_progress(act));

    let expect_goal = progress_tap;
    let expect_fp_change = matches!(
        act.intent,
        PilotIntent::Tap | PilotIntent::Dismiss | PilotIntent::Scroll | PilotIntent::Back
    ) && !expect_goal;

    let level = if expect_goal && (!enter.is_empty() || !exit.is_empty()) {
        SpecLevel::L1
    } else {
        SpecLevel::L0
    };

    SpecPrediction {
        level,
        schema: SPECULATE_SCHEMA_VERSION,
        expect_goal,
        expect_fp_change: expect_fp_change && !expect_goal,
        expect_enter: if expect_goal {
            enter
        } else {
            vec![]
        },
        expect_exit: if expect_goal { exit } else { vec![] },
        from_fp: from_fp.to_string(),
        act_key: act.key.clone(),
        from_scene_fp: None,
        from_region_kinds: vec![],
    }
}

/// Scene-conditioned prediction + research telemetry.
pub fn predict_after_act_on_feel(
    goal: &PilotGoal,
    act: &PilotAct,
    feel: &FeelIR,
) -> SpecPrediction {
    let mut pred = predict_after_act(goal, act, &feel.place.fingerprint);
    if let Some(scene) = feel.scene.as_ref() {
        pred.from_scene_fp = Some(scene.place.fp.clone());
        pred.from_region_kinds = scene
            .regions
            .iter()
            .map(|r| format!("{:?}", r.kind).to_ascii_lowercase())
            .collect();
        // Tab bar live but destination tab absent → do not expect full GoalSpec yet.
        if feel.world.has_tab_bar && pred.expect_goal {
            let wants_tab = pred.expect_enter.iter().any(|n| {
                n.starts_with("tab_")
                    || crate::observe::identity_suggests_tab_label(n, "Notes")
                    || crate::observe::identity_suggests_tab_label(n, "Home")
            });
            if wants_tab {
                let tab_present = pred.expect_enter.iter().any(|n| {
                    feel.world.elements.iter().any(|el| {
                        el.tab_chrome
                            && (el.identifier.as_deref() == Some(n.as_str())
                                || el.label.as_deref().is_some_and(|lab| {
                                    crate::observe::identity_suggests_tab_label(n, lab)
                                }))
                    })
                });
                if !tab_present {
                    pred.expect_goal = false;
                    pred.expect_fp_change = true;
                    pred.level = SpecLevel::L1;
                }
            }
        }
    }
    if pred.expect_goal {
        for s in feel.salience.iter().take(8) {
            for needle in [s.id.clone(), s.label.clone()].into_iter().flatten() {
                let low = needle.to_ascii_lowercase();
                if (low.contains("login") || low.contains("sign in") || low.contains("password"))
                    && !pred.expect_exit.iter().any(|e| e == &needle)
                {
                    pred.expect_exit.push(needle);
                }
            }
        }
    }
    pred
}

fn act_looks_progress(act: &PilotAct) -> bool {
    let lab = act.label.as_deref().unwrap_or("").to_ascii_lowercase();
    let id = act.id.as_deref().unwrap_or("").to_ascii_lowercase();
    const KEYS: &[&str] = &[
        "login", "sign in", "signin", "continue", "next", "finish", "done", "confirm", "submit",
        "save", "start",
    ];
    KEYS.iter()
        .any(|k| lab.contains(k) || id.contains(k))
}

pub fn begin_speculate(
    pred: SpecPrediction,
    now_unix_ms: u64,
    certify_budget_ms: u64,
) -> SpecTicket {
    SpecTicket {
        phase: SpecPhase::Speculative,
        pred,
        deadline_unix_ms: now_unix_ms.saturating_add(certify_budget_ms.max(1)),
        started_unix_ms: now_unix_ms,
        reject_reason: None,
        preplanned: None,
    }
}

pub fn certify(
    ticket: &SpecTicket,
    feel: &FeelIR,
    snap: &ObserveSnapshot,
    goal_holds: bool,
    now_unix_ms: u64,
) -> SpecVerdict {
    if matches!(
        ticket.phase,
        SpecPhase::Certified | SpecPhase::RolledBack | SpecPhase::Idle
    ) {
        return if ticket.phase == SpecPhase::Certified {
            SpecVerdict::Certified
        } else if ticket.phase == SpecPhase::RolledBack {
            SpecVerdict::Rejected
        } else {
            SpecVerdict::Forbidden
        };
    }

    if feel.feel.eyes_unusable_like() || matches!(feel.feel.phase, FeelPhase::EyesUnusable) {
        if now_unix_ms >= ticket.deadline_unix_ms {
            return SpecVerdict::Rejected;
        }
        return SpecVerdict::Pending;
    }

    let settled = matches!(feel.feel.phase, FeelPhase::Settled) && feel.feel.ready;
    let fp = feel.place.fingerprint.as_str();
    let fp_changed = fp != ticket.pred.from_fp;
    let l1_ok = l1_identities_hold(&ticket.pred, feel, snap);

    if ticket.pred.expect_goal {
        if goal_holds && settled && l1_ok {
            return SpecVerdict::Certified;
        }
        if settled && (!goal_holds || !l1_ok) && now_unix_ms >= ticket.deadline_unix_ms {
            return SpecVerdict::Rejected;
        }
        if now_unix_ms >= ticket.deadline_unix_ms {
            return SpecVerdict::Rejected;
        }
        return SpecVerdict::Pending;
    }

    if ticket.pred.expect_fp_change {
        if fp_changed && settled && l1_ok {
            return SpecVerdict::Certified;
        }
        if settled && (!fp_changed || !l1_ok) && now_unix_ms >= ticket.deadline_unix_ms {
            return SpecVerdict::Rejected;
        }
        if now_unix_ms >= ticket.deadline_unix_ms {
            return SpecVerdict::Rejected;
        }
        return SpecVerdict::Pending;
    }

    if settled && l1_ok {
        return SpecVerdict::Certified;
    }
    if now_unix_ms >= ticket.deadline_unix_ms {
        return SpecVerdict::Rejected;
    }
    SpecVerdict::Pending
}

pub fn apply_verdict(ticket: &mut SpecTicket, verdict: SpecVerdict, reason: Option<String>) {
    match verdict {
        SpecVerdict::Certified => {
            ticket.phase = SpecPhase::Certified;
            ticket.reject_reason = None;
        }
        SpecVerdict::Rejected => {
            ticket.phase = SpecPhase::RolledBack;
            ticket.reject_reason = reason.or_else(|| Some("prediction_mismatch".into()));
        }
        SpecVerdict::Pending | SpecVerdict::Forbidden => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::{GoalPredicate, GoalSpec, PilotAct, PilotIntent};
    use crate::feel::{FeelBlock, FeelDelta, FeelIR, FeelMeta, FeelPlace, FeelPhase, WorldModel};
    use crate::observe::{AccessibilityTree, ObserveSnapshot};
    use crate::qa::AffordanceKind;
    use serde_json::json;

    fn feel_settled(fp: &str) -> FeelIR {
        FeelIR {
            schema: 2,
            place: FeelPlace {
                fingerprint: fp.into(),
                surface: Some("app".into()),
                title: None,
                bundle_id: Some("me.demo".into()),
            },
            salience: vec![],
            block: None,
            delta: FeelDelta::default(),
            feel: FeelMeta {
                phase: FeelPhase::Settled,
                keyboard: false,
                ready: true,
            },
            world: WorldModel::default(),
            scene: None,
        }
    }

    fn feel_transition(fp: &str) -> FeelIR {
        let mut f = feel_settled(fp);
        f.feel.phase = FeelPhase::Transition;
        f
    }

    fn feel_blind(fp: &str) -> FeelIR {
        let mut f = feel_settled(fp);
        f.feel.phase = FeelPhase::EyesUnusable;
        f.feel.ready = false;
        f
    }

    fn snap_with(ids: &[&str]) -> ObserveSnapshot {
        let nodes: Vec<_> = ids
            .iter()
            .map(|id| {
                json!({
                    "identifier": id,
                    "label": id,
                    "hittable": true,
                    "visible": true,
                    "enabled": true,
                })
            })
            .collect();
        ObserveSnapshot {
            schema_version: 2,
            udid: "t".into(),
            session_id: None,
            boot_epoch: 1,
            launch_epoch: 1,
            screen_epoch: 1,
            stability_streak: 2,
            motion_score: Some(0.0),
            expected_bundle_id: None,
            observed_app_label: Some("Demo".into()),
            booted: true,
            simulator_app_running: false,
            frame: None,
            app_bundle_id: Some("me.demo".into()),
            accessibility_tree: AccessibilityTree::Available {
                nodes,
                root: None,
                element_count: Some(ids.len()),
                point_size: Some((393.0, 852.0)),
            },
            scene: None,
            actionable_topk: vec![],
            events: vec![],
            ax_quality: "ready".into(),
            settled: true,
            observe_ms: None,
            path: None,
            phase: Some("ready".into()),
            eyes_unusable: false,
            overlay: Some("none".into()),
            screen_sig: None,
        }
    }

    fn login_goal() -> PilotGoal {
        GoalSpec {
            all: vec![
                GoalPredicate {
                    identity: Some("homeTitle".into()),
                    ..Default::default()
                },
                GoalPredicate {
                    identity: Some("Home".into()),
                    ..Default::default()
                },
            ],
            none: vec![GoalPredicate {
                identity: Some("loginButton".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn tap_login() -> PilotAct {
        PilotAct {
            intent: PilotIntent::Tap,
            label: Some("Login".into()),
            id: Some("loginButton".into()),
            kind: Some(AffordanceKind::PrimaryButton),
            text: None,
            secure: false,
            slot_name: None,
            key: "tap|loginButton".into(),
            reason: "test".into(),
            stop_code: None,
            motor_strategy: None,
        }
    }

    #[test]
    fn forbids_speculate_when_eyes_unusable_or_outstanding() {
        assert!(!may_speculate(&feel_blind("fp"), false));
        assert!(!may_speculate(&feel_settled("fp"), true));
        assert!(may_speculate(&feel_settled("fp"), false));
    }

    #[test]
    fn primary_tap_predicts_goal_l1() {
        let pred = predict_after_act(&login_goal(), &tap_login(), "fp_login");
        assert!(pred.expect_goal);
        assert_eq!(pred.level, SpecLevel::L1);
        assert!(pred.expect_enter.iter().any(|e| e == "homeTitle"));
        assert!(pred.expect_exit.iter().any(|e| e == "loginButton"));
    }

    #[test]
    fn certify_goal_when_settled_and_holds() {
        let pred = predict_after_act(&login_goal(), &tap_login(), "fp_login");
        let mut ticket = begin_speculate(pred, 1_000, 2_000);
        let snap = snap_with(&["homeTitle", "Home"]);
        let feel = feel_settled("fp_home");
        let v = certify(&ticket, &feel, &snap, true, 1_500);
        assert_eq!(v, SpecVerdict::Certified);
        apply_verdict(&mut ticket, v, None);
        assert_eq!(ticket.phase, SpecPhase::Certified);
    }

    #[test]
    fn certify_rejects_l1_when_exit_identity_still_present() {
        let pred = predict_after_act(&login_goal(), &tap_login(), "fp_login");
        assert_eq!(pred.level, SpecLevel::L1);
        let ticket = begin_speculate(pred, 1_000, 500);
        // goal_holds=true but loginButton still on surface → L1 must reject.
        let snap = snap_with(&["homeTitle", "Home", "loginButton"]);
        let feel = feel_settled("fp_home");
        let v = certify(&ticket, &feel, &snap, true, 2_000);
        assert_eq!(v, SpecVerdict::Rejected);
    }

    #[test]
    fn reject_when_settled_deadline_and_goal_missing() {
        let pred = predict_after_act(&login_goal(), &tap_login(), "fp_login");
        let ticket = begin_speculate(pred, 1_000, 500);
        let snap = snap_with(&["loginButton", "Welcome"]);
        let feel = feel_settled("fp_login");
        let v = certify(&ticket, &feel, &snap, false, 2_000);
        assert_eq!(v, SpecVerdict::Rejected);
    }

    #[test]
    fn pending_while_transition_before_deadline() {
        let pred = predict_after_act(&login_goal(), &tap_login(), "fp_login");
        let ticket = begin_speculate(pred, 1_000, 5_000);
        let snap = snap_with(&["loginButton"]);
        let feel = feel_transition("fp_login");
        let v = certify(&ticket, &feel, &snap, false, 1_200);
        assert_eq!(v, SpecVerdict::Pending);
    }

    #[test]
    fn blocked_feel_forbids_new_speculation() {
        let mut f = feel_settled("fp");
        f.feel.phase = FeelPhase::Blocked;
        f.block = Some(FeelBlock {
            kind: "sheet".into(),
            detail: None,
        });
        assert!(!may_speculate(&f, false));
    }

    #[test]
    fn spec_stats_from_trace() {
        let trace = vec![
            json!({"event": "speculate_begin"}),
            json!({"event": "speculate_preplan"}),
            json!({"event": "speculate_end", "verdict": "certified"}),
            json!({"event": "speculate_fire_preplanned"}),
        ];
        let s = SpecStats::from_trace(&trace);
        assert_eq!(s.begins, 1);
        assert_eq!(s.certified, 1);
        assert_eq!(s.preplan, 1);
        assert_eq!(s.fire_preplanned, 1);
        assert!((s.certify_rate().unwrap() - 1.0).abs() < 1e-9);
    }
}
