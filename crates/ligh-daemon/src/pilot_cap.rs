//! Autopilot capability — host executes a goal end to end, zero LLM tokens.
//!
//! Perceive → Feel IR → generic policy → motor → verify, in a Rust loop. The
//! caller supplies the acceptance target and typed data only; the path is
//! discovered here. On failure the verdict carries a semantic diagnosis plus a
//! source hint, so a code-fixing agent gets evidence instead of a bare timeout.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    build_feel, diagnose, next_act, AffordanceKind, CapabilityResult, FaultClass, ObserveSnapshot,
    PilotAct, PilotGoal, PilotIntent, PilotLimits, PilotMemory, PilotStepRecord, SessionPhase,
    UxGraph,
};
use ligh_host::{AxDump, HidInput};
use serde_json::json;

use crate::capabilities::{phase_of, run_app, settle_eyes, surface_of};
use crate::qa_cap::{cap_dismiss, perceive_from_snap};
use crate::ux_cap::{graph_file, ux_persist_perceive};
use crate::DaemonState;

/// Host-side probe for the acceptance target. Independent of the policy so the
/// planner never has to infer whether the goal is on screen.
fn goal_visible(udid: &str, goal: &PilotGoal) -> bool {
    if let Some(id) = goal.target_id.as_deref() {
        if AxDump::exists_id(udid, id).unwrap_or(false) {
            return true;
        }
    }
    if let Some(label) = goal.target_label.as_deref() {
        if AxDump::exists_label(udid, label).unwrap_or(false) {
            return true;
        }
    }
    false
}

fn execute(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    act: &PilotAct,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    match act.intent {
        PilotIntent::Type => crate::motor::motor_type(
            build,
            state,
            act.text.as_deref().unwrap_or(""),
            act.label.as_deref(),
            act.id.as_deref(),
            settle_ms,
            timeout_ms,
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
    }
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
    install: bool,
    launch_args: Option<&[String]>,
) -> CapabilityResult {
    let started = Instant::now();

    if goal.target_id.is_none() && goal.target_label.is_none() {
        return CapabilityResult::fail(
            FaultClass::Model,
            SessionPhase::Ready,
            None,
            "autopilot",
            json!({ "error": "goal requires target_id or target_label" }),
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

    let udid = match state.lock().unwrap().current_udid() {
        Ok(u) => u,
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
    };

    let limits = PilotLimits::default();
    let mut mem = PilotMemory::new();
    let mut history: Vec<PilotStepRecord> = Vec::new();
    let mut trace: Vec<serde_json::Value> = Vec::new();
    let mut seen_fps: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prev_fp: Option<String> = None;
    let mut last_snap = settle_eyes(build, settle_ms);
    let mut stop_code = "max_steps".to_string();
    let mut reached = false;

    while mem.steps < max_steps {
        let snap = settle_eyes(build, settle_ms);
        let view = perceive_from_snap(&snap);
        let feel = build_feel(&view, &snap, prev_fp.as_deref(), Some(settle_ms));
        let fp = feel.place.fingerprint.clone();
        if seen_fps.insert(fp.clone()) {
            ux_persist_perceive(workspace, &view);
        }
        last_snap = snap;

        let visible = goal_visible(&udid, goal);
        let act = next_act(goal, &feel, &mem, visible, limits);

        if act.is_terminal() {
            stop_code = act.stop_code.clone().unwrap_or_else(|| "stop".into());
            reached = stop_code == "goal_visible";
            trace.push(json!({ "step": mem.steps + 1, "act": act.trace(), "fp": fp }));
            break;
        }

        let step_started = Instant::now();
        let r = execute(build, state, &act, settle_ms, timeout_ms);
        // Primary CTAs commonly publish an intermediate loading frame before their
        // async state change. A normal settle can return on that actionable frame
        // immediately, causing the planner to explore unrelated controls. Poll only
        // the declared acceptance target for a short bounded window — no per-app
        // knowledge and no extra LLM turn.
        let async_goal_visible = if r.ok
            && act.intent == PilotIntent::Tap
            && act.kind == Some(AffordanceKind::PrimaryButton)
        {
            let deadline = Instant::now() + Duration::from_millis(1_500);
            let mut visible = goal_visible(&udid, goal);
            while !visible && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(40));
                visible = goal_visible(&udid, goal);
            }
            visible
        } else {
            false
        };
        let snap_after = if async_goal_visible {
            settle_eyes(build, settle_ms.min(800))
        } else {
            r.observe
                .clone()
                .unwrap_or_else(|| settle_eyes(build, settle_ms))
        };
        let view_after = perceive_from_snap(&snap_after);
        let fp_after = view_after.location.fingerprint.clone();

        let record = PilotStepRecord {
            step: mem.steps + 1,
            intent: act.intent,
            label: act.label.clone(),
            id: act.id.clone(),
            kind: act.kind,
            fp_before: fp.clone(),
            fp_after: fp_after.clone(),
            fired: r.ok,
            changed: fp_after != fp,
            events: view_after.since_last.clone(),
            ms: step_started.elapsed().as_millis() as u64,
        };
        trace.push(json!({
            "step": record.step,
            "act": act.trace(),
            "fp": fp,
            "fp_after": fp_after,
            "fired": record.fired,
            "changed": record.changed,
            "ms": record.ms,
        }));
        history.push(record);

        mem.mark(&fp, &act);
        prev_fp = Some(fp);
        last_snap = snap_after;
    }

    // Confirm the acceptance target the same way a declarative postcondition would.
    if reached || goal_visible(&udid, goal) {
        let confirm = crate::motor::motor_reach(
            build,
            state,
            goal.target_label.as_deref(),
            goal.target_id.as_deref(),
            4,
            settle_ms,
            timeout_ms,
        );
        if confirm.ok {
            return CapabilityResult::success(
                confirm.phase,
                confirm.surface.clone(),
                "autopilot",
                json!({
                    "goal": { "target_id": goal.target_id, "target_label": goal.target_label },
                    "reached": true,
                    "steps": mem.steps,
                    "elapsed_ms": started.elapsed().as_millis(),
                    "llm_tokens": 0,
                    "trace": trace,
                }),
                confirm.observe,
            );
        }
        last_snap = confirm.observe.unwrap_or(last_snap);
    }

    let view = perceive_from_snap(&last_snap);
    let feel = build_feel(&view, &last_snap, prev_fp.as_deref(), Some(settle_ms));
    let diagnosis = diagnose(goal, &history, &feel);
    let hint = source_hint(workspace, diagnosis.fingerprint.as_deref());

    CapabilityResult::fail(
        FaultClass::Model,
        phase_of(&last_snap),
        surface_of(&last_snap),
        "autopilot",
        json!({
            "goal": { "target_id": goal.target_id, "target_label": goal.target_label },
            "reached": false,
            "stop_code": stop_code,
            "steps": mem.steps,
            "elapsed_ms": started.elapsed().as_millis(),
            "llm_tokens": 0,
            "diagnosis": diagnosis,
            "source_hint": hint,
            "trace": trace,
        }),
        Some(last_snap),
    )
}
