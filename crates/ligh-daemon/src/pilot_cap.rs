//! Autopilot capability — host executes a goal end to end, zero LLM tokens.
//!
//! Perceive → Feel IR → generic policy → motor → verify, in a Rust loop. The
//! caller supplies the acceptance target and typed data only; the path is
//! discovered here. On failure the verdict carries a semantic diagnosis plus a
//! source hint, so a code-fixing agent gets evidence instead of a bare timeout.

use std::path::Path;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    build_feel, diagnose, emit_repair_contract, next_act, recovery_stage, repair_agent_view,
    ActionOutcome, AffordanceKind, CapabilityResult, EpochStamp, FaultClass, GoalPredicate,
    MotorTypeStrategy, ObserveSnapshot, PilotAct, PilotDiagnosis, PilotGoal, PilotIntent,
    PilotLimits, PilotMemory, PilotStepRecord, RecoveryStage, SessionPhase, UxGraph,
};
use ligh_host::HidInput;
use serde_json::json;

use crate::capabilities::{phase_of, run_app, settle_eyes, surface_of};
use crate::qa_cap::{cap_dismiss, perceive_from_snap};
use crate::ux_cap::{graph_file, ux_persist_perceive};
use crate::DaemonState;

struct EventTrace {
    path: std::path::PathBuf,
    file: std::fs::File,
}

impl EventTrace {
    fn open(workspace: Option<&Path>, state: &Arc<Mutex<DaemonState>>) -> Option<Self> {
        let root = workspace.unwrap_or_else(|| Path::new("."));
        let dir = root.join(".ligh").join("runs");
        std::fs::create_dir_all(&dir).ok()?;
        let st = state.lock().unwrap();
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();
        let path = dir.join(format!(
            "autopilot-{}-{millis}.jsonl",
            st.session_id.replace('/', "_")
        ));
        drop(st);
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .ok()?;
        Some(Self { path, file })
    }

    fn append(&mut self, value: &serde_json::Value) {
        if serde_json::to_writer(&mut self.file, value).is_ok() {
            let _ = self.file.write_all(b"\n");
            let _ = self.file.flush();
        }
    }
}

fn push_trace(
    memory: &mut Vec<serde_json::Value>,
    sink: &mut Option<EventTrace>,
    event: serde_json::Value,
) {
    if let Some(sink) = sink {
        sink.append(&event);
    }
    memory.push(event);
}


/// Nodes that participate in GoalSpec acceptance.
///
/// Off-screen / non-hittable AX leftovers (previous SwiftUI views still in the
/// tree) must not satisfy `all` or poison `none`. Acceptance is what a user
/// can see and act on — the settled, on-screen surface.
fn acceptance_nodes(nodes: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    nodes
        .iter()
        .filter(|node| {
            let hittable = node
                .get("hittable")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let visible = node
                .get("visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            hittable && visible
        })
        .collect()
}

fn predicate_matches(nodes: &[serde_json::Value], predicate: &GoalPredicate) -> bool {
    // A constraint-less predicate is a schema error, not a wildcard.
    if predicate.id.is_none()
        && predicate.label.is_none()
        && predicate.identity.is_none()
        && predicate.value_contains.is_none()
        && predicate.enabled.is_none()
        && predicate.focused.is_none()
    {
        return false;
    }
    let nodes = acceptance_nodes(nodes);
    nodes.iter().any(|node| {
        let id_matches = predicate.id.as_deref().map_or(true, |needle| {
            ligh_core::node_matches_identifier(node, needle)
        });
        let label_matches = predicate.label.as_deref().map_or(true, |needle| {
            node.get("label")
                .and_then(|v| v.as_str())
                .map(|label| label == needle || label.contains(needle))
                .unwrap_or(false)
        });
        let identity_matches = predicate.identity.as_deref().map_or(true, |needle| {
            ligh_core::node_matches_identity_needle(node, needle)
        });
        let value_matches = predicate.value_contains.as_deref().map_or(true, |needle| {
            node.get("value")
                .and_then(|v| v.as_str())
                .map(|value| value.contains(needle))
                .unwrap_or(false)
        });
        let enabled_matches = predicate.enabled.map_or(true, |expected| {
            node.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true) == expected
        });
        let focused_matches = predicate.focused.map_or(true, |expected| {
            node.get("focused").and_then(|v| v.as_bool()).unwrap_or(false) == expected
        });
        id_matches
            && label_matches
            && identity_matches
            && value_matches
            && enabled_matches
            && focused_matches
    })
}

fn foreground_owned(snap: &ObserveSnapshot, goal: &PilotGoal) -> bool {
    // In-process DevDriver means the instrumented app is foreground.
    // Physical dumps often omit SpringBoard-style app labels.
    if ligh_host::physical_ui_active() {
        if let Some(bid) = snap.app_bundle_id.as_deref() {
            if let Some(expected) = goal.expected_bundle_id.as_deref() {
                return bid == expected;
            }
            return true;
        }
    }
    if let Some(expected) = goal
        .expected_bundle_id
        .as_deref()
        .or(snap.expected_bundle_id.as_deref())
    {
        if snap.expected_bundle_id.as_deref() != Some(expected)
            && snap.app_bundle_id.as_deref() != Some(expected)
        {
            return false;
        }
    }
    snap.observed_app_label
        .as_deref()
        .map(|label| label != "SpringBoard" && label != "Home")
        .unwrap_or(false)
}

fn goal_matches(snap: &ObserveSnapshot, goal: &PilotGoal) -> bool {
    if !foreground_owned(snap, goal) {
        return false;
    }
    let nodes = snap.accessibility_tree.nodes();
    let required = goal.required_predicates();
    // Empty acceptance is a contract violation (legacy binary / bad compile) —
    // never treat it as a wildcard match.
    if required.is_empty() {
        return false;
    }
    let all_ok = required.iter().all(|p| predicate_matches(nodes, p));
    let none_ok = goal.none.iter().all(|p| !predicate_matches(nodes, p));
    all_ok && none_ok
}

fn stable_goal(
    build: &dyn Fn() -> ObserveSnapshot,
    goal: &PilotGoal,
    settle_ms: u64,
) -> Option<ObserveSnapshot> {
    let required = goal.stable_observations.max(2);
    let deadline = Instant::now()
        + Duration::from_millis(settle_ms.max(goal.stability_window_ms).max(400));
    let mut streak = 0;
    let mut last_fp: Option<String> = None;
    let mut accepted = None;
    while Instant::now() < deadline {
        let snap = build();
        let fp = ligh_core::screen_fingerprint(snap.accessibility_tree.nodes());
        if goal_matches(&snap, goal) && last_fp.as_deref().map_or(true, |last| last == fp) {
            streak += 1;
            last_fp = Some(fp);
            accepted = Some(snap);
            if streak >= required {
                return accepted;
            }
        } else {
            streak = 0;
            last_fp = Some(fp);
            accepted = None;
        }
        std::thread::sleep(Duration::from_millis(45));
    }
    None
}

fn execute(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    act: &PilotAct,
    expected_epoch: &EpochStamp,
    goal: &PilotGoal,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let current = build();
    let current_epoch = EpochStamp::from_snapshot(&current);
    if !expected_epoch.same_target_epoch(&current_epoch) {
        return CapabilityResult::fail(
            FaultClass::StaleSnapshot,
            phase_of(&current),
            surface_of(&current),
            "autopilot",
            json!({ "error": "target epoch invalidated", "expected": expected_epoch, "actual": current_epoch }),
            Some(current),
        )
        .with_action_outcome(ActionOutcome::TargetStale);
    }
    if !foreground_owned(&current, goal) {
        return CapabilityResult::fail(
            FaultClass::WrongSurface,
            phase_of(&current),
            surface_of(&current),
            "autopilot",
            json!({ "error": "expected app does not own foreground" }),
            Some(current),
        )
        .with_action_outcome(ActionOutcome::WrongSurface);
    }
    let mut result = match act.intent {
        PilotIntent::Type => crate::motor::motor_type(
            build,
            state,
            act.text.as_deref().unwrap_or(""),
            act.label.as_deref(),
            act.id.as_deref(),
            settle_ms,
            timeout_ms,
            act.motor_strategy.unwrap_or(MotorTypeStrategy::FocusHid),
        ),
        PilotIntent::Tap | PilotIntent::Back => crate::motor::motor_tap(
            build,
            state,
            act.label.as_deref(),
            act.id.as_deref(),
            settle_ms,
            timeout_ms,
            None,
            None,
        ),
        PilotIntent::Dismiss => cap_dismiss(build, state, settle_ms),
        PilotIntent::Scroll => {
            let (udid, w, h) = {
                let st = state.lock().unwrap();
                match st.current_udid() {
                    Ok(u) => (u, st.sim_width, st.sim_height),
                    Err(e) => {
                        return CapabilityResult::fail(
                            FaultClass::Infra,
                            SessionPhase::Dead,
                            None,
                            "autopilot",
                            json!({ "error": e }),
                            None,
                        )
                    }
                }
            };
            if let Err(e) = HidInput::swipe(&udid, 0.5, 0.72, 0.5, 0.28, w, h) {
                return CapabilityResult::fail(
                    FaultClass::MotorRejected,
                    SessionPhase::Degraded,
                    None,
                    "autopilot",
                    json!({ "error": e.to_string(), "op": "scroll" }),
                    None,
                );
            }
            std::thread::sleep(Duration::from_millis(280));
            let snap = settle_eyes(build, settle_ms);
            CapabilityResult::success(
                phase_of(&snap),
                surface_of(&snap),
                "autopilot",
                json!({ "op": "scroll" }),
                Some(snap),
            )
        }
        PilotIntent::Wait => {
            let snap = settle_eyes(build, settle_ms.max(1500));
            CapabilityResult::success(
                phase_of(&snap),
                surface_of(&snap),
                "autopilot",
                json!({ "op": "wait" }),
                Some(snap),
            )
        }
        PilotIntent::Stop => CapabilityResult::success(
            SessionPhase::Ready,
            None,
            "autopilot",
            json!({ "op": "stop" }),
            None,
        ),
    };
    if result.action_outcome.is_none() {
        let outcome = if let Some(post) = result.observe.as_ref() {
            if !foreground_owned(post, goal) {
                ActionOutcome::WrongSurface
            } else if !expected_epoch.same_target_epoch(&EpochStamp::from_snapshot(post)) {
                ActionOutcome::TargetStale
            } else if post.eyes_unusable || !post.settled {
                ActionOutcome::TransitionInProgress
            } else if !result.ok {
                ActionOutcome::NotDelivered
            } else {
                ActionOutcome::DeliveredAndVerified
            }
        } else if result.ok {
            ActionOutcome::DeliveredAndVerified
        } else {
            ActionOutcome::InfrastructureFault
        };
        result.action_outcome = Some(outcome);
    }
    result
}

fn source_hint(workspace: Option<&Path>, fingerprint: Option<&str>) -> Option<serde_json::Value> {
    let fp = fingerprint?;
    let graph = UxGraph::load(&graph_file(workspace)).ok()?;
    let hint = graph.source_hints_for(fp).into_iter().next()?;
    Some(json!({
        "path": hint.path,
        "confidence": hint.confidence,
        "edits": hint.edits,
    }))
}

fn infer_source_root(workspace: &Path, app: Option<&str>) -> Option<std::path::PathBuf> {
    let app_path = app?;
    let full = if Path::new(app_path).is_absolute() {
        Path::new(app_path).to_path_buf()
    } else {
        workspace.join(app_path)
    };
    let build_dir = full.parent()?;
    if build_dir.file_name()?.to_str()? != "build" {
        return None;
    }
    let project = build_dir.parent()?;
    let name = project.file_name()?.to_str()?;
    let candidate = project.join(name);
    if candidate.is_dir() {
        return Some(candidate);
    }
    None
}

fn source_hint_from_contract(contract: &ligh_core::RepairContract) -> Option<serde_json::Value> {
    Some(json!({
        "path": contract.scope.primary_path,
        "confidence": 0.92,
        "edits": [contract.scope.edit_intent.clone()],
        "avoid_paths": contract.scope.forbidden_globs,
        "diagnosis_code": contract.diagnosis_code,
        "edit_globs": contract.scope.edit_globs,
    }))
}

fn merge_repair_hint(
    contract_hint: Option<serde_json::Value>,
    ux: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    contract_hint.or(ux)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cap_autopilot(
    workspace: Option<&Path>,
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    app: Option<&str>,
    bundle_id: Option<&str>,
    goal: &PilotGoal,
    max_steps: u32,
    settle_ms: u64,
    timeout_ms: u64,
    run_timeout_ms: u64,
    install: bool,
    launch_args: Option<&[String]>,
) -> CapabilityResult {
    let started = Instant::now();
    let run_deadline = started + Duration::from_millis(run_timeout_ms.max(1));

    if goal.required_predicates().is_empty() {
        return CapabilityResult::fail(
            FaultClass::Model,
            SessionPhase::Ready,
            None,
            "autopilot",
            json!({ "error": "goal requires at least one `all` predicate or legacy target" }),
            None,
        );
    }

    // Physical motor: refuse Autopilot when DevDriver cannot gesture (motor v1 lie).
    if ligh_host::physical_ui_active() {
        let ui = ligh_host::physical_ui();
        let ver = ui.as_ref().map(|u| u.driver_version()).unwrap_or(0);
        let caps = ui
            .as_ref()
            .map(|u| u.capabilities())
            .unwrap_or(json!({}));
        let gesture_ok = ver >= 2
            || caps.get("gesture").and_then(|v| v.as_bool()).unwrap_or(false);
        if !gesture_ok {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Ready,
                None,
                "autopilot",
                json!({
                    "error": "physical DevDriver lacks gesture capability — rebuild the app with @mm-labs/ligh-expo >= 0.2 (driver_version 2)",
                    "driver_version": ver,
                    "capabilities": caps,
                }),
                None,
            );
        }
    }

    if let Some(app_path) = app {
        if ligh_host::physical_ui_active() {
            // Physical DevDriver already owns the live app. Never install/relaunch
            // a Simulator .app — Cursor→Expo→phone butter path.
            let _ = app_path;
        } else {
            let launched = run_app(
                build,
                state,
                app_path,
                bundle_id,
                None,
                None,
                settle_ms,
                timeout_ms,
                install,
                launch_args,
            );
            if !launched.ok {
                return CapabilityResult::fail(
                    launched.fault,
                    launched.phase,
                    launched.surface,
                    "autopilot",
                    json!({ "stage": "run_app", "detail": launched.detail }),
                    launched.observe,
                );
            }
        }
    }

    let limits = PilotLimits::default();
    let mut mem = PilotMemory::new();
    let mut history: Vec<PilotStepRecord> = Vec::new();
    let mut trace: Vec<serde_json::Value> = Vec::new();
    let mut trace_sink = EventTrace::open(workspace, state);
    let trace_path = trace_sink
        .as_ref()
        .map(|sink| sink.path.display().to_string());
    let session_meta = {
        let st = state.lock().unwrap();
        json!({
            "event": "operation_started",
            "session_id": st.session_id,
            "boot_epoch": st.boot_epoch,
            "launch_epoch": st.launch_epoch,
            "expected_bundle_id": st.expected_bundle_id,
            "run_deadline_ms": run_timeout_ms,
        })
    };
    push_trace(&mut trace, &mut trace_sink, session_meta);
    let mut seen_fps: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prev_fp: Option<String> = None;
    let mut last_snap = settle_eyes(build, settle_ms);
    if !goal.starting.is_empty()
        && !goal
            .starting
            .iter()
            .all(|predicate| predicate_matches(last_snap.accessibility_tree.nodes(), predicate))
    {
        return CapabilityResult::fail(
            FaultClass::TargetMissing,
            phase_of(&last_snap),
            surface_of(&last_snap),
            "autopilot",
            json!({
                "stage": "starting_predicates",
                "error": "declared starting state does not match fresh observation",
                "goal": goal,
            }),
            Some(last_snap),
        );
    }
    let mut stop_code = "max_steps".to_string();
    let mut reached = false;
    let mut recovery_count = 0u32;
    let mut last_outcome: Option<ActionOutcome> = None;
    /// Speculative decoding: act planned during Pending, fired only after Certified.
    let mut pending_preplan: Option<PilotAct> = None;

    while mem.steps < max_steps && Instant::now() < run_deadline {
        let snap = settle_eyes(build, settle_ms);
        let view = perceive_from_snap(&snap);
        let feel = build_feel(&view, &snap, prev_fp.as_deref(), Some(settle_ms));
        let fp = feel.place.fingerprint.clone();
        if seen_fps.insert(fp.clone()) {
            ux_persist_perceive(workspace, &view);
        }
        last_snap = snap.clone();

        let visible = goal_matches(&snap, goal);
        if visible {
            if let Some(confirmed) = stable_goal(build, goal, settle_ms) {
                last_snap = confirmed;
                reached = true;
                stop_code = "goal_satisfied".into();
                pending_preplan = None;
                break;
            }
            // Acceptance is on screen but not yet stable — wait, do not explore.
            push_trace(
                &mut trace,
                &mut trace_sink,
                json!({
                    "step": mem.steps + 1,
                    "act": {
                        "intent": "wait",
                        "reason": "goal visible — awaiting stability",
                        "stop_code": null,
                    },
                    "fp": fp,
                    "note": "goal_visible_awaiting_stability",
                }),
            );
            mem.steps += 1;
            std::thread::sleep(Duration::from_millis(goal.stability_window_ms.max(80)));
            continue;
        }

        if !foreground_owned(&snap, goal) {
            pending_preplan = None;
            recovery_count += 1;
            let strategy = if recovery_count == 1 {
                RecoveryStage::ReacquireForeground
            } else {
                RecoveryStage::Relaunch
            };
            push_trace(&mut trace, &mut trace_sink, json!({
                "event": "recovery",
                "stage": recovery_count,
                "strategy": strategy.as_str(),
                "epoch": EpochStamp::from_snapshot(&snap),
            }));
            if recovery_count <= 2 {
                if let Some(app_path) = app {
                    let recovery = run_app(
                        build,
                        state,
                        app_path,
                        bundle_id.or(goal.expected_bundle_id.as_deref()),
                        None,
                        None,
                        settle_ms,
                        timeout_ms.min(run_deadline.saturating_duration_since(Instant::now()).as_millis() as u64),
                        false,
                        launch_args,
                    );
                    if recovery.ok {
                        last_snap = recovery.observe.unwrap_or_else(|| settle_eyes(build, settle_ms));
                        continue;
                    }
                }
            }
            stop_code = "wrong_surface".into();
            last_outcome = Some(ActionOutcome::WrongSurface);
            break;
        }

        let act = if let Some(pre) = pending_preplan.take() {
            if ligh_core::act_valid_on_feel(&pre, &feel) {
                push_trace(
                    &mut trace,
                    &mut trace_sink,
                    json!({
                        "event": "speculate_fire_preplanned",
                        "act": pre.trace(),
                        "fp": fp,
                    }),
                );
                pre
            } else {
                push_trace(
                    &mut trace,
                    &mut trace_sink,
                    json!({
                        "event": "speculate_preplan_stale",
                        "act": pre.trace(),
                        "fp": fp,
                    }),
                );
                next_act(goal, &feel, &mem, false, limits)
            }
        } else {
            next_act(goal, &feel, &mem, false, limits)
        };

        if act.is_terminal() {
            stop_code = act.stop_code.clone().unwrap_or_else(|| "stop".into());
            reached = false;
            push_trace(
                &mut trace,
                &mut trace_sink,
                json!({ "step": mem.steps + 1, "act": act.trace(), "fp": fp }),
            );
            break;
        }

        let step_started = Instant::now();
        let epoch = EpochStamp::from_snapshot(&snap);
        let action_spec = act.action_spec(epoch.clone(), mem.steps + 1);
        let remaining_ms = run_deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if remaining_ms == 0 {
            stop_code = "run_deadline".into();
            break;
        }

        // Speculative navigation: predict before fire; certify on settle evidence.
        // Ablation: LIGH_SPECULATE=0 → may_speculate always false (classic settle).
        let pred = ligh_core::predict_after_act_on_feel(goal, &act, &feel);
        let can_spec = ligh_core::may_speculate(&feel, mem.speculation_outstanding()) && !act.is_terminal();

        let r = execute(
            build,
            state,
            &act,
            &epoch,
            goal,
            settle_ms.min(remaining_ms),
            timeout_ms.min(remaining_ms),
        );

        let now_ms = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        };

        let mut spec_verdict = ligh_core::SpecVerdict::Forbidden;
        let (snap_after, goal_progress_spec) = if r.ok && can_spec {
            let budget = if pred.expect_goal {
                settle_ms.max(1_500).min(remaining_ms.max(1))
            } else {
                settle_ms.min(remaining_ms.max(1))
            };
            let ticket = ligh_core::begin_speculate(pred.clone(), now_ms(), budget);
            mem.speculation = Some(ticket);
            // Preplan immediately (same-screen fills) — do not wait for Pending polls;
            // type/tap often Certify on the first settle frame.
            if !pred.expect_goal {
                let mut mem_ahead = mem.clone();
                mem_ahead.speculation = None;
                mem_ahead.mark_outcome(
                    &fp,
                    &act,
                    ActionOutcome::DeliveredAndVerified,
                    &fp,
                );
                let pre = next_act(goal, &feel, &mem_ahead, false, limits);
                if !pre.is_terminal() && ligh_core::act_valid_on_feel(&pre, &feel) {
                    if let Some(ticket) = mem.speculation.as_mut() {
                        ticket.preplanned = Some(pre.clone());
                    }
                    push_trace(
                        &mut trace,
                        &mut trace_sink,
                        json!({
                            "event": "speculate_preplan",
                            "act": pre.trace(),
                            "from_fp": fp,
                            "when": "begin",
                        }),
                    );
                }
            }
            push_trace(
                &mut trace,
                &mut trace_sink,
                json!({
                    "event": "speculate_begin",
                    "pred": pred,
                    "budget_ms": budget,
                }),
            );

            let mut last = r
                .observe
                .clone()
                .unwrap_or_else(|| settle_eyes(build, 80));
            let mut verdict = ligh_core::SpecVerdict::Pending;
            while now_ms()
                < mem
                    .speculation
                    .as_ref()
                    .map(|t| t.deadline_unix_ms)
                    .unwrap_or(0)
            {
                let s = settle_eyes(build, 60);
                let v = perceive_from_snap(&s);
                let f = build_feel(&v, &s, Some(&fp), Some(60));
                let holds = goal_matches(&s, goal);
                if let Some(ticket) = mem.speculation.as_ref() {
                    verdict = ligh_core::certify(ticket, &f, &s, holds, now_ms());
                }
                // Cognitive speculative decoding: plan act₁ while act₀ certifies.
                // Optimistic memory assumes current act commits (same-screen fills).
                if matches!(verdict, ligh_core::SpecVerdict::Pending)
                    && !pred.expect_goal
                    && mem
                        .speculation
                        .as_ref()
                        .map(|t| t.preplanned.is_none())
                        .unwrap_or(false)
                {
                    let mut mem_ahead = mem.clone();
                    mem_ahead.speculation = None;
                    mem_ahead.mark_outcome(
                        &fp,
                        &act,
                        ActionOutcome::DeliveredAndVerified,
                        &f.place.fingerprint,
                    );
                    let pre = next_act(goal, &f, &mem_ahead, holds, limits);
                    if !pre.is_terminal() && ligh_core::act_valid_on_feel(&pre, &f) {
                        if let Some(ticket) = mem.speculation.as_mut() {
                            ticket.preplanned = Some(pre.clone());
                        }
                        push_trace(
                            &mut trace,
                            &mut trace_sink,
                            json!({
                                "event": "speculate_preplan",
                                "act": pre.trace(),
                                "from_fp": fp,
                                "feel_fp": f.place.fingerprint,
                            }),
                        );
                    }
                }
                last = s;
                if matches!(
                    verdict,
                    ligh_core::SpecVerdict::Certified | ligh_core::SpecVerdict::Rejected
                ) {
                    break;
                }
            }
            if let Some(ticket) = mem.speculation.as_mut() {
                ligh_core::apply_verdict(
                    ticket,
                    verdict,
                    match verdict {
                        ligh_core::SpecVerdict::Rejected => Some("prediction_mismatch".into()),
                        _ => None,
                    },
                );
            }
            spec_verdict = verdict;
            let preplanned_out = mem
                .speculation
                .as_ref()
                .and_then(|t| t.preplanned.clone());
            push_trace(
                &mut trace,
                &mut trace_sink,
                json!({
                    "event": "speculate_end",
                    "verdict": verdict,
                    "ticket": mem.speculation,
                }),
            );
            let holds = goal_matches(&last, goal);
            if matches!(verdict, ligh_core::SpecVerdict::Certified) {
                if let Some(pre) = preplanned_out {
                    let v_after = perceive_from_snap(&last);
                    let f_after = build_feel(&v_after, &last, Some(&fp), Some(60));
                    if ligh_core::act_valid_on_feel(&pre, &f_after) {
                        pending_preplan = Some(pre);
                    }
                }
            } else {
                pending_preplan = None;
            }
            (last, holds || matches!(verdict, ligh_core::SpecVerdict::Certified))
        } else {
            pending_preplan = None;
            let snap_after = r
                .observe
                .clone()
                .unwrap_or_else(|| settle_eyes(build, settle_ms));
            let holds = goal_matches(&snap_after, goal);
            (snap_after, holds)
        };

        let view_after = perceive_from_snap(&snap_after);
        let fp_after = view_after.location.fingerprint.clone();
        let changed = fp_after != fp;
        let goal_progress = goal_progress_spec;
        let mut outcome = r.action_outcome.unwrap_or_else(|| {
            if r.ok {
                ActionOutcome::DeliveredAndVerified
            } else {
                ActionOutcome::NotDelivered
            }
        });
        match spec_verdict {
            ligh_core::SpecVerdict::Certified => {
                outcome = ActionOutcome::DeliveredAndVerified;
            }
            ligh_core::SpecVerdict::Rejected if pred.expect_goal || pred.expect_fp_change => {
                if matches!(
                    outcome,
                    ActionOutcome::DeliveredAndVerified | ActionOutcome::TransitionInProgress
                ) {
                    outcome = ActionOutcome::DeliveredNoEffect;
                }
            }
            _ => {}
        }
        if r.ok
            && matches!(act.intent, PilotIntent::Tap | PilotIntent::Dismiss | PilotIntent::Scroll)
            && !changed
            && view_after.since_last.is_empty()
            && !goal_progress
            && !matches!(spec_verdict, ligh_core::SpecVerdict::Certified)
        {
            outcome = ActionOutcome::DeliveredNoEffect;
        }
        if !foreground_owned(&snap_after, goal) {
            outcome = ActionOutcome::WrongSurface;
        }
        last_outcome = Some(outcome);

        let record = PilotStepRecord {
            step: mem.steps + 1,
            action_id: action_spec.action_id.clone(),
            epoch: epoch.clone(),
            intent: act.intent,
            label: act.label.clone(),
            id: act.id.clone(),
            kind: act.kind,
            fp_before: fp.clone(),
            fp_after: fp_after.clone(),
            fired: matches!(
                outcome,
                ActionOutcome::DeliveredAndVerified | ActionOutcome::DeliveredNoEffect
            ),
            changed,
            outcome: Some(outcome),
            goal_progress,
            candidate_keys: feel
                .salience
                .iter()
                .map(|item| item.id.clone().or_else(|| item.label.clone()).unwrap_or_default())
                .collect(),
            events: view_after.since_last.clone(),
            ms: step_started.elapsed().as_millis() as u64,
        };
        push_trace(&mut trace, &mut trace_sink, json!({
            "step": record.step,
            "act": act.trace(),
            "action_spec": action_spec,
            "fp": fp,
            "fp_after": fp_after,
            "epoch": epoch,
            "pre_snapshot_hash": fp,
            "post_snapshot_hash": record.fp_after,
            "fired": record.fired,
            "changed": record.changed,
            "outcome": outcome,
            "goal_progress": goal_progress,
            "speculate": spec_verdict,
            "fault": r.fault,
            "ms": record.ms,
        }));
        history.push(record);

        mem.mark_outcome(&fp, &act, outcome, &fp_after);
        mem.clear_speculation();
        if !outcome.memory_committable() {
            let fail_n = mem.failures.get(&act.memory_key()).copied().unwrap_or(0);
            let strategy = recovery_stage(
                outcome,
                act.intent,
                feel.block.is_some(),
                fail_n,
            );
            push_trace(&mut trace, &mut trace_sink, json!({
                "event": "recovery",
                "strategy": strategy.as_str(),
                "rank": strategy.rank(),
                "failures": fail_n,
                "action_id": format!("action-{}-{}", epoch.launch_epoch, mem.steps),
                "budget_remaining_ms": run_deadline.saturating_duration_since(Instant::now()).as_millis(),
            }));
            match strategy {
                RecoveryStage::WaitStable => {
                    last_snap = settle_eyes(build, settle_ms.max(400));
                }
                RecoveryStage::RefreshAndResolve | RecoveryStage::AlternateMotor => {}
                RecoveryStage::DismissBlockingScope => {
                    let dismissed = cap_dismiss(build, state, settle_ms);
                    last_snap = dismissed.observe.unwrap_or_else(|| settle_eyes(build, settle_ms));
                }
                RecoveryStage::ReacquireForeground | RecoveryStage::Relaunch => {
                    if let Some(app_path) = app {
                        let recovery = run_app(
                            build,
                            state,
                            app_path,
                            bundle_id.or(goal.expected_bundle_id.as_deref()),
                            None,
                            None,
                            settle_ms,
                            timeout_ms.min(
                                run_deadline
                                    .saturating_duration_since(Instant::now())
                                    .as_millis() as u64,
                            ),
                            false,
                            launch_args,
                        );
                        if recovery.ok {
                            last_snap = recovery
                                .observe
                                .unwrap_or_else(|| settle_eyes(build, settle_ms));
                            if strategy == RecoveryStage::Relaunch {
                                mem.failures.clear();
                                mem.tried.clear();
                                mem.scrolls.clear();
                            }
                        }
                    }
                }
                RecoveryStage::StopInfrastructure => {
                    stop_code = "infrastructure_fault".into();
                    break;
                }
            }
        }
        prev_fp = Some(fp);
        last_snap = snap_after;
        if outcome == ActionOutcome::InfrastructureFault {
            stop_code = "infrastructure_fault".into();
            break;
        }
        if outcome == ActionOutcome::TransitionInProgress {
            recovery_count += 1;
            std::thread::sleep(Duration::from_millis(80));
        }
    }
    if Instant::now() >= run_deadline {
        stop_code = "run_deadline".into();
    }

    // Fresh, non-mutating, temporal verification. Acceptance never scrolls or taps
    // the UI merely to make a predicate true.
    if reached || goal_matches(&last_snap, goal) {
        if let Some(confirm) = stable_goal(build, goal, settle_ms) {
            let spec_stats = ligh_core::SpecStats::from_trace(&trace);
            return CapabilityResult::success(
                phase_of(&confirm),
                surface_of(&confirm),
                "autopilot",
                json!({
                    "goal": goal,
                    "reached": true,
                    "steps": mem.steps,
                    "elapsed_ms": started.elapsed().as_millis(),
                    "llm_tokens": 0,
                    "speculate_enabled": ligh_core::speculate_enabled(),
                    "speculate_stats": spec_stats,
                    "trace_path": trace_path,
                    "trace": trace,
                }),
                Some(confirm),
            )
            .with_action_outcome(ActionOutcome::DeliveredAndVerified);
        }
    }

    let view = perceive_from_snap(&last_snap);
    let feel = build_feel(&view, &last_snap, prev_fp.as_deref(), Some(settle_ms));
    let diagnosis = diagnose(goal, &history, &feel);
    let ws = workspace.unwrap_or_else(|| Path::new("."));
    let source_root = infer_source_root(ws, app);
    let contract = emit_repair_contract(&diagnosis, goal, &feel, source_root.as_deref());
    let hint = merge_repair_hint(
        source_hint_from_contract(&contract),
        source_hint(workspace, diagnosis.fingerprint.as_deref()),
    );
    let repair_view = repair_agent_view(&contract);
    let spec_stats = ligh_core::SpecStats::from_trace(&trace);

    let final_fault = match last_outcome {
        Some(ActionOutcome::WrongSurface) => FaultClass::WrongSurface,
        Some(ActionOutcome::TargetStale) => FaultClass::StaleSnapshot,
        Some(ActionOutcome::InfrastructureFault) => FaultClass::Infra,
        Some(ActionOutcome::TransitionInProgress) => FaultClass::TransitionInProgress,
        Some(ActionOutcome::NotDelivered) => FaultClass::MotorRejected,
        Some(ActionOutcome::DeliveredNoEffect) => FaultClass::MotorNoEffect,
        _ if stop_code == "run_deadline" => FaultClass::Timeout,
        _ => FaultClass::TargetMissing,
    };
    let fault_owner = final_fault.owner().as_str();

    CapabilityResult::fail(
        final_fault,
        phase_of(&last_snap),
        surface_of(&last_snap),
        "autopilot",
        json!({
            "goal": goal,
            "reached": false,
            "stop_code": stop_code,
            "steps": mem.steps,
            "elapsed_ms": started.elapsed().as_millis(),
            "llm_tokens": 0,
            "diagnosis": diagnosis,
            "repair_contract": repair_view,
            "fault_owner": fault_owner,
            "source_hint": hint,
            "speculate_enabled": ligh_core::speculate_enabled(),
            "speculate_stats": spec_stats,
            "trace_path": trace_path,
            "trace": trace,
        }),
        Some(last_snap),
    )
}
