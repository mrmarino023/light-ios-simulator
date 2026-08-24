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
    build_feel, diagnose, next_act, recovery_stage, ActionOutcome, AffordanceKind, CapabilityResult,
    EpochStamp, FaultClass, GoalPredicate, MotorTypeStrategy, ObserveSnapshot, PilotAct, PilotGoal,
    PilotIntent, PilotLimits, PilotMemory, PilotStepRecord, RecoveryStage, SessionPhase, UxGraph,
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


fn predicate_matches(nodes: &[serde_json::Value], predicate: &GoalPredicate) -> bool {
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
        id_matches && label_matches && value_matches && enabled_matches && focused_matches
    })
}

fn foreground_owned(snap: &ObserveSnapshot, goal: &PilotGoal) -> bool {
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
    !required.is_empty()
        && required.iter().all(|p| predicate_matches(nodes, p))
        && goal.none.iter().all(|p| !predicate_matches(nodes, p))
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

    if let Some(app_path) = app {
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
                break;
            }
        }

        if !foreground_owned(&snap, goal) {
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

        let act = next_act(goal, &feel, &mem, false, limits);

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
        let r = execute(
            build,
            state,
            &act,
            &epoch,
            goal,
            settle_ms.min(remaining_ms),
            timeout_ms.min(remaining_ms),
        );
        // Primary CTAs commonly publish an intermediate loading frame before their
        // async state change. A normal settle can return on that actionable frame
        // immediately, causing the planner to explore unrelated controls. Poll only
        // the declared acceptance target for a short bounded window — no per-app
        // knowledge and no extra LLM turn.
        let async_goal_visible = if r.ok
            && act.intent == PilotIntent::Tap
            && act.kind == Some(AffordanceKind::PrimaryButton)
        {
            stable_goal(build, goal, settle_ms.min(1_500))
        } else {
            None
        };
        let snap_after = if let Some(accepted) = async_goal_visible.clone() {
            accepted
        } else {
            r.observe
                .clone()
                .unwrap_or_else(|| settle_eyes(build, settle_ms))
        };
        let view_after = perceive_from_snap(&snap_after);
        let fp_after = view_after.location.fingerprint.clone();
        let changed = fp_after != fp;
        let goal_progress = async_goal_visible.is_some() || goal_matches(&snap_after, goal);
        let mut outcome = r.action_outcome.unwrap_or_else(|| {
            if r.ok {
                ActionOutcome::DeliveredAndVerified
            } else {
                ActionOutcome::NotDelivered
            }
        });
        if r.ok
            && matches!(act.intent, PilotIntent::Tap | PilotIntent::Dismiss | PilotIntent::Scroll)
            && !changed
            && view_after.since_last.is_empty()
            && !goal_progress
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
            "fault": r.fault,
            "ms": record.ms,
        }));
        history.push(record);

        mem.mark_outcome(&fp, &act, outcome, &fp_after);
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
    let hint = source_hint(workspace, diagnosis.fingerprint.as_deref());

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
            "fault_owner": fault_owner,
            "source_hint": hint,
            "trace_path": trace_path,
            "trace": trace,
        }),
        Some(last_snap),
    )
}
