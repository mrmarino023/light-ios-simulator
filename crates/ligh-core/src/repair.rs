//! L5 Repair plane — typed repair contracts from verified motor failures.
//!
//! A coding agent must not discover edit scope from prose. The host emits a
//! `RepairContract`: failure mode, invariant, allowed/forbidden paths, oracles,
//! and the same structured world evidence the motor used to fail.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::autopilot::{PilotDiagnosis, PilotGoal};
use crate::feel::{feel_agent_view, FeelIR};

pub const REPAIR_SCHEMA_VERSION: u32 = 1;

/// Hard wall budget for TRAIL repair (product proof target).
pub const REPAIR_JOB_WALL_MS: u64 = 120_000;

/// Prove + localize budget (R1–R3, no LLM).
pub const TRAIL_PROVE_LOCALIZE_MS: u64 = 45_000;

/// How a scoped file edit was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchSource {
    Llm,
    Manual,
}

/// One file replacement — host applies, agent never streams a repo walk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPlan {
    /// Path relative to repo root (includes task `source_root` prefix).
    pub path: String,
    pub content: String,
    pub source: PatchSource,
}

/// TRAIL repair job phases (in-daemon target).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairJobPhase {
    TraceExercise,
    Localize,
    Patch,
    Build,
    TraceCertify,
    Done,
    Timeout,
}

/// Functiona11ity failure from a harness exercise step (TaskAudit-style trace oracle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceFailure {
    pub step: u32,
    pub action: String,
    pub expected_identity: String,
    pub observed_identities: Vec<String>,
    pub fault: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Result envelope for `repair_job` RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairJobResult {
    pub reached: bool,
    pub verified: bool,
    pub mode: RepairMode,
    pub phase: RepairJobPhase,
    pub patch_source: Option<PatchSource>,
    pub wall_ms: u64,
    pub build_ms: Option<u64>,
    pub run_goal_ms: Option<u64>,
    pub llm_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<RepairContract>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairMode {
    TabChromeMissing,
    StateGateStuck,
    BlockedOverlay,
    AcceptanceNotInAx,
    TypeNeverCommitted,
    MotorRejected,
    TargetNeverVisible,
    EyesUnusable,
    Unknown,
}

impl RepairMode {
    pub fn from_diagnosis_code(code: &str) -> Self {
        match code {
            "tab_chrome_missing" => Self::TabChromeMissing,
            "control_fired_no_transition" => Self::StateGateStuck,
            "blocked_overlay" => Self::BlockedOverlay,
            "acceptance_not_in_ax" => Self::AcceptanceNotInAx,
            "type_never_committed" => Self::TypeNeverCommitted,
            "motor_rejected" => Self::MotorRejected,
            "target_never_visible" => Self::TargetNeverVisible,
            "eyes_unusable" => Self::EyesUnusable,
            _ => Self::Unknown,
        }
    }
}

/// Structured world delta — what the motor saw vs what the goal requires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEvidence {
    pub goal_target: String,
    pub goal_present: bool,
    pub has_tab_bar: bool,
    pub tab_items: Vec<String>,
    pub missing_identities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    pub perception: Value,
}

/// Static + dynamic edit scope for the repair agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairScope {
    /// Glob patterns relative to `source_root` the agent may edit.
    pub edit_globs: Vec<String>,
    /// Glob patterns that must not be edited for this failure mode.
    pub forbidden_globs: Vec<String>,
    /// Highest-confidence file to open first (repo-relative).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_path: Option<String>,
    /// Human-readable edit intent.
    pub edit_intent: String,
}

/// L5 contract emitted on every host-side goal failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairContract {
    pub schema: u32,
    pub mode: RepairMode,
    pub diagnosis_code: String,
    pub invariant: String,
    pub scope: RepairScope,
    pub oracle_pre: String,
    pub oracle_post: String,
    /// Natural-language certify target derived from the exercise trace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oracle_trace: Option<String>,
    pub evidence: WorldEvidence,
    pub max_patch_candidates: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_failure: Option<TraceFailure>,
}

fn tab_labels(feel: &FeelIR) -> Vec<String> {
    feel.world
        .elements
        .iter()
        .filter(|el| el.tab_chrome)
        .filter_map(|el| {
            el.label
                .clone()
                .or_else(|| el.identifier.clone())
        })
        .collect()
}

fn goal_identities(goal: &PilotGoal) -> Vec<String> {
    goal.required_predicates()
        .iter()
        .filter_map(|p| {
            p.identity
                .clone()
                .or_else(|| p.id.clone())
                .or_else(|| p.label.clone())
        })
        .collect()
}

fn identity_visible(needle: &str, feel: &FeelIR) -> bool {
    feel.world.elements.iter().any(|el| {
        el.identifier.as_deref() == Some(needle)
            || el.label.as_deref() == Some(needle)
            || el.stable_key.contains(needle)
    }) || feel.salience.iter().any(|s| {
        s.id.as_deref() == Some(needle) || s.label.as_deref() == Some(needle)
    })
}

/// Build structured evidence the agent sees instead of raw AX dumps.
pub fn world_evidence(goal: &PilotGoal, feel: &FeelIR, control: Option<&str>) -> WorldEvidence {
    let identities = goal_identities(goal);
    let missing: Vec<String> = identities
        .iter()
        .filter(|id| !identity_visible(id, feel))
        .cloned()
        .collect();
    WorldEvidence {
        goal_target: goal.target_name(),
        goal_present: missing.is_empty(),
        has_tab_bar: feel.world.has_tab_bar,
        tab_items: tab_labels(feel),
        missing_identities: missing,
        control: control.map(|s| s.to_string()),
        perception: feel_agent_view(feel),
    }
}

fn skip_repo_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "build" | "target" | "DerivedData" | "node_modules" | ".ligh"
    )
}

fn task_relative_path(source_root: &Path, rel_inside: &str) -> String {
    source_root
        .join(rel_inside)
        .to_string_lossy()
        .replace('\\', "/")
}

fn find_swift_matching(root: &Path, dir: &Path, depth: u32, pred: &dyn Fn(&str, &str) -> bool) -> Option<String> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if skip_repo_dir(&name) {
                continue;
            }
            if let Some(found) = find_swift_matching(root, &path, depth - 1, pred) {
                matches.push(found);
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("swift") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.display().to_string());
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if pred(&rel, file_name) {
            matches.push(rel);
        }
    }
    matches.sort_by_key(|p| p.len());
    matches.into_iter().next()
}

fn find_tab_composition_path(source_root: &Path) -> Option<String> {
    for rel in [
        "Navigation/MainTabView.swift",
        "MainTabView.swift",
        "Navigation/TabView.swift",
    ] {
        let candidate = source_root.join(rel);
        if candidate.is_file() {
            return candidate
                .strip_prefix(source_root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
        }
    }
    find_swift_matching(source_root, source_root, 12, &|rel, file| {
        file.contains("TabView") || (rel.contains("Navigation/") && file.contains("Tab"))
    })
}

fn find_auth_gate_path(source_root: &Path, control: &str) -> Option<String> {
    let control_lower = control.to_lowercase();
    find_swift_matching(source_root, source_root, 12, &|rel, file| {
        let file_lower = file.to_lowercase();
        let rel_lower = rel.to_lowercase();
        (rel_lower.contains("/auth/") || file_lower.contains("login"))
            && (file_lower.contains(&control_lower.replace('_', ""))
                || file_lower.contains("viewmodel")
                || file_lower.contains("login"))
    })
    .or_else(|| {
        find_swift_matching(source_root, source_root, 12, &|rel, file| {
            let rel_lower = rel.to_lowercase();
            rel_lower.contains("/auth/") || file.to_lowercase().contains("login")
        })
    })
}

fn find_overlay_path(source_root: &Path) -> Option<String> {
    find_swift_matching(source_root, source_root, 12, &|_rel, file| {
        let f = file.to_lowercase();
        f.contains("onboard") || f.contains("overlay") || f.contains("welcome")
    })
}

/// L6 — derive edit scope from failure mode + repo layout (no LLM).
pub fn repair_scope(
    mode: RepairMode,
    source_root: Option<&Path>,
    diagnosis: &PilotDiagnosis,
    goal: &PilotGoal,
) -> RepairScope {
    let control = diagnosis.control.as_deref().unwrap_or("");
    match mode {
        RepairMode::TabChromeMissing => {
            let tab = diagnosis
                .control
                .clone()
                .unwrap_or_else(|| goal.target_name());
            RepairScope {
                edit_globs: vec![
                    "**/Navigation/**".into(),
                    "**/*TabView*.swift".into(),
                ],
                forbidden_globs: vec!["**/Auth/**".into(), "**/Login*.swift".into()],
                primary_path: source_root.and_then(|root| {
                    find_tab_composition_path(root).map(|rel| task_relative_path(root, &rel))
                }),
                edit_intent: format!("restore missing '{tab}' in TabView composition"),
            }
        }
        RepairMode::StateGateStuck => RepairScope {
            edit_globs: vec![
                "**/Auth/**".into(),
                "**/*ViewModel*.swift".into(),
                "**/AppState*.swift".into(),
            ],
            forbidden_globs: vec!["**/Navigation/*TabView*.swift".into()],
            primary_path: source_root.and_then(|root| {
                find_auth_gate_path(root, control).map(|rel| task_relative_path(root, &rel))
            }),
            edit_intent: format!(
                "fix state gate for control '{control}' — what it sets vs what the router reads"
            ),
        },
        RepairMode::BlockedOverlay => RepairScope {
            edit_globs: vec![
                "**/Onboard*/**".into(),
                "**/*Overlay*.swift".into(),
                "**/*Welcome*.swift".into(),
            ],
            forbidden_globs: vec![],
            primary_path: source_root.and_then(|root| {
                find_overlay_path(root).map(|rel| task_relative_path(root, &rel))
            }),
            edit_intent: "fix dismiss/finish path so overlay hides and flow proceeds".into(),
        },
        RepairMode::AcceptanceNotInAx | RepairMode::TargetNeverVisible => RepairScope {
            edit_globs: vec![
                "**/Navigation/**".into(),
                "**/*TabView*.swift".into(),
                "**/Features/**".into(),
            ],
            forbidden_globs: vec![],
            primary_path: source_root.and_then(|root| {
                find_tab_composition_path(root).map(|rel| task_relative_path(root, &rel))
            }),
            edit_intent: format!(
                "expose accessibility identity '{}' in the composed UI",
                goal.target_name()
            ),
        },
        RepairMode::TypeNeverCommitted => RepairScope {
            edit_globs: vec!["**/Auth/**".into(), "**/*Field*.swift".into()],
            forbidden_globs: vec![],
            primary_path: source_root.and_then(|root| {
                find_auth_gate_path(root, control).map(|rel| task_relative_path(root, &rel))
            }),
            edit_intent: "fix field focus/commit so typed text sticks in AX".into(),
        },
        RepairMode::MotorRejected => RepairScope {
            edit_globs: vec!["**/*.swift".into()],
            forbidden_globs: vec![],
            primary_path: None,
            edit_intent: "fix hittable accessibility for the target control".into(),
        },
        RepairMode::EyesUnusable | RepairMode::Unknown => RepairScope {
            edit_globs: vec!["**/*.swift".into()],
            forbidden_globs: vec![],
            primary_path: None,
            edit_intent: "inspect source for the failing screen".into(),
        },
    }
}

fn oracle_pair(mode: RepairMode, goal: &PilotGoal, evidence: &WorldEvidence) -> (String, String) {
    match mode {
        RepairMode::TabChromeMissing => (
            "post-login: tab bar visible".into(),
            format!(
                "tab item for '{}' present and goal identity visible",
                evidence.control.as_deref().unwrap_or(&goal.target_name())
            ),
        ),
        RepairMode::StateGateStuck => (
            "login form visible".into(),
            "screen transitions after primary control fires".into(),
        ),
        RepairMode::BlockedOverlay => (
            "overlay blocking main content".into(),
            "overlay dismissed; goal surface reachable".into(),
        ),
        _ => (
            "initial broken state from harness".into(),
            format!("goal identity '{}' visible on-screen", goal.target_name()),
        ),
    }
}

/// Emit the full L5 contract from a verified motor failure.
pub fn emit_repair_contract(
    diagnosis: &PilotDiagnosis,
    goal: &PilotGoal,
    feel: &FeelIR,
    source_root: Option<&Path>,
) -> RepairContract {
    let mode = RepairMode::from_diagnosis_code(&diagnosis.code);
    let evidence = world_evidence(goal, feel, diagnosis.control.as_deref());
    let scope = repair_scope(mode, source_root, diagnosis, goal);
    let (oracle_pre, oracle_post) = oracle_pair(mode, goal, &evidence);
    RepairContract {
        schema: REPAIR_SCHEMA_VERSION,
        mode,
        diagnosis_code: diagnosis.code.clone(),
        invariant: diagnosis.message.clone(),
        scope,
        oracle_pre,
        oracle_post,
        oracle_trace: None,
        evidence,
        max_patch_candidates: 2,
        trace_failure: None,
    }
}

/// Attach trace oracle to an emitted contract (TRAIL R3).
pub fn contract_with_trace(mut contract: RepairContract, trace: TraceFailure) -> RepairContract {
    contract.oracle_trace = Some(format!(
        "step {} {} expected '{}' — observed {:?}",
        trace.step, trace.action, trace.expected_identity, trace.observed_identities
    ));
    contract.trace_failure = Some(trace);
    contract
}

/// FixAlly-style natural-language fix plan for the constrained Fixer (R4) — not a code template.
pub fn fix_plan_for_mode(contract: &RepairContract) -> String {
    let target = contract
        .trace_failure
        .as_ref()
        .map(|t| t.expected_identity.as_str())
        .or_else(|| contract.evidence.control.as_deref())
        .unwrap_or(&contract.evidence.goal_target);
    match contract.mode {
        RepairMode::TabChromeMissing => format!(
            "Restore the missing tab or navigation item for identity '{target}' in the TabView \
             composition file (not Auth/Login). Preserve existing tabs and accessibility identifiers."
        ),
        RepairMode::StateGateStuck => format!(
            "Fix the state gate so tapping '{target}' transitions to the post-login screen. \
             Align what the control sets with what the router/navigation reads."
        ),
        RepairMode::BlockedOverlay => format!(
            "Fix the onboarding/overlay finish handler so '{target}' dismisses the overlay and \
             reveals the home surface."
        ),
        RepairMode::AcceptanceNotInAx | RepairMode::TargetNeverVisible => format!(
            "Expose accessibility identity '{target}' on the composed on-screen UI."
        ),
        RepairMode::TypeNeverCommitted => {
            "Fix field focus/commit so typed credentials stick in accessibility.".into()
        }
        RepairMode::MotorRejected => {
            "Fix hittable accessibility for the target control.".into()
        }
        RepairMode::EyesUnusable | RepairMode::Unknown => format!(
            "Minimal fix so exercise step for '{target}' succeeds without breaking other flows."
        ),
    }
}

/// Map harness motor fault strings to repair modes for trace-driven prove.
pub fn repair_mode_from_trace(fault: &str, expected_identity: &str) -> RepairMode {
    match fault {
        "target_missing" | "target_never_visible" if expected_identity.starts_with("tab_") => {
            RepairMode::TabChromeMissing
        }
        "target_missing" | "target_never_visible" => RepairMode::TargetNeverVisible,
        "motor_no_effect" | "control_fired_no_transition" => RepairMode::StateGateStuck,
        "blocked" => RepairMode::BlockedOverlay,
        "type_never_committed" => RepairMode::TypeNeverCommitted,
        "motor_rejected" => RepairMode::MotorRejected,
        "eyes_unusable" => RepairMode::EyesUnusable,
        _ => RepairMode::Unknown,
    }
}

/// Returns true when `rel_path` (posix-style) is allowed under the contract scope.
pub fn path_allowed_by_contract(rel_path: &str, contract: &RepairContract) -> bool {
    let norm = rel_path.replace('\\', "/");
    for forbidden in &contract.scope.forbidden_globs {
        if glob_match(forbidden, &norm) {
            return false;
        }
    }
    if contract.scope.edit_globs.iter().any(|g| glob_match(g, &norm)) {
        return true;
    }
    if let Some(primary) = &contract.scope.primary_path {
        if norm == primary.replace('\\', "/") {
            return true;
        }
    }
    false
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let pat = pattern.trim_start_matches("./");
    if pat == "**/*.swift" {
        return path.ends_with(".swift");
    }
    if let Some(rest) = pat.strip_prefix("**/") {
        if rest.ends_with("/**") {
            let seg = rest.trim_end_matches("/**");
            return path.contains(seg);
        }
        if rest.contains('*') {
            let prefix = rest.split('*').next().unwrap_or("");
            return path.contains(prefix.trim_end_matches('/'));
        }
        return path.contains(rest);
    }
    if pat.contains('*') {
        let stem = pat.trim_start_matches('*');
        return path.ends_with(stem);
    }
    path.contains(pat)
}

/// Agent wire — compact contract + evidence (never raw AX dump).
pub fn repair_agent_view(contract: &RepairContract) -> Value {
    json!({
        "schema": contract.schema,
        "mode": contract.mode,
        "diagnosis_code": contract.diagnosis_code,
        "invariant": contract.invariant,
        "scope": contract.scope,
        "oracle_pre": contract.oracle_pre,
        "oracle_post": contract.oracle_post,
        "oracle_trace": contract.oracle_trace,
        "fix_plan": fix_plan_for_mode(contract),
        "trace_failure": contract.trace_failure,
        "evidence": {
            "goal_target": contract.evidence.goal_target,
            "goal_present": contract.evidence.goal_present,
            "has_tab_bar": contract.evidence.has_tab_bar,
            "tab_items": contract.evidence.tab_items,
            "missing_identities": contract.evidence.missing_identities,
            "control": contract.evidence.control,
            "perception": contract.evidence.perception,
        },
        "max_patch_candidates": contract.max_patch_candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::{GoalPredicate, GoalSpec};
    use crate::feel::{FeelMeta, FeelPhase, FeelPlace, WorldElement};
    use crate::qa::AffordanceKind;

    fn minimal_feel(tab_bar: bool) -> FeelIR {
        FeelIR {
            schema: 1,
            place: FeelPlace {
                fingerprint: "fp".into(),
                surface: Some("app".into()),
                title: None,
                bundle_id: Some("test".into()),
            },
            salience: vec![],
            block: None,
            delta: Default::default(),
            feel: FeelMeta {
                phase: FeelPhase::Settled,
                ready: true,
                keyboard: false,
            },
            world: {
                let mut w = crate::feel::WorldModel::default();
                w.has_tab_bar = tab_bar;
                w.elements = vec![WorldElement {
                    stable_key: "id:tab_home".into(),
                    ax_path: "tab_home".into(),
                    kind: AffordanceKind::Button,
                    identifier: Some("tab_home".into()),
                    label: Some("Home".into()),
                    role: Some("button".into()),
                    frame_bucket: None,
                    value_hash: None,
                    enabled: true,
                    focused: false,
                    editable: false,
                    on_screen: true,
                    overlay_scope: None,
                    tab_chrome: true,
                }];
                w
            },
            scene: None,
        }
    }

    #[test]
    fn tab_chrome_contract_forbids_auth() {
        let diagnosis = PilotDiagnosis {
            code: "tab_chrome_missing".into(),
            message: "tab missing".into(),
            fingerprint: Some("fp".into()),
            control: Some("Notes".into()),
        };
        let mut goal = GoalSpec::default();
        goal.all = vec![GoalPredicate {
            identity: Some("notes_title".into()),
            ..Default::default()
        }];
        let feel = minimal_feel(true);
        let contract = emit_repair_contract(&diagnosis, &goal, &feel, None);
        assert_eq!(contract.mode, RepairMode::TabChromeMissing);
        assert!(!path_allowed_by_contract(
            "Features/Auth/LoginView.swift",
            &contract
        ));
        assert!(path_allowed_by_contract(
            "Navigation/MainTabView.swift",
            &contract
        ));
    }

    #[test]
    fn state_gate_scope_prefers_auth() {
        let diagnosis = PilotDiagnosis {
            code: "control_fired_no_transition".into(),
            message: "stuck".into(),
            fingerprint: Some("fp".into()),
            control: Some("login_button".into()),
        };
        let goal = GoalSpec::default();
        let feel = minimal_feel(false);
        let contract = emit_repair_contract(&diagnosis, &goal, &feel, None);
        assert_eq!(contract.mode, RepairMode::StateGateStuck);
        assert!(path_allowed_by_contract("Features/Auth/LoginView.swift", &contract));
    }

    #[test]
    fn trace_failure_wires_oracle_and_fix_plan() {
        let trace = TraceFailure {
            step: 4,
            action: "tap".into(),
            expected_identity: "tab_notes".into(),
            observed_identities: vec!["tab_home".into()],
            fault: "target_missing".into(),
            scene_before: Some("chrome_band".into()),
            scene_after: Some("chrome_band".into()),
            label: Some("Notes".into()),
        };
        let diagnosis = PilotDiagnosis {
            code: "tab_chrome_missing".into(),
            message: "tab missing".into(),
            fingerprint: Some("fp".into()),
            control: Some("tab_notes".into()),
        };
        let mut goal = GoalSpec::default();
        goal.all = vec![GoalPredicate {
            identity: Some("notes_title".into()),
            ..Default::default()
        }];
        let feel = minimal_feel(true);
        let contract = contract_with_trace(
            emit_repair_contract(&diagnosis, &goal, &feel, None),
            trace,
        );
        assert!(contract.oracle_trace.as_ref().unwrap().contains("tab_notes"));
        let plan = fix_plan_for_mode(&contract);
        assert!(plan.contains("TabView"));
        assert!(repair_mode_from_trace("target_missing", "tab_notes") == RepairMode::TabChromeMissing);
    }
}
