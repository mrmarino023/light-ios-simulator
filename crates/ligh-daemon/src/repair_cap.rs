//! TRAIL `cap_repair_job` — prove / apply / certify over AppJob + RepairContract.
//!
//! LLM fixer stays outside the daemon. This capability owns motor prove/certify
//! and scoped patch apply so agents get one RPC instead of a long Python harness.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ligh_core::{
    build_feel, contract_with_trace, emit_repair_contract, fix_plan_for_mode, repair_agent_view,
    repair_mode_from_trace, CapabilityResult, EditPlan, FaultClass, ObserveSnapshot, PilotDiagnosis,
    PilotGoal, RepairJobPhase, RepairMode, TraceFailure, REPAIR_JOB_WALL_MS,
};
use serde_json::{json, Value};

use crate::capabilities::app_job;
use crate::qa_cap::perceive_from_snap;
use crate::DaemonState;

fn ax_identities(obs: &Option<ObserveSnapshot>) -> Vec<String> {
    let Some(obs) = obs else {
        return Vec::new();
    };
    let mut keys: Vec<String> = Vec::new();
    for n in obs.accessibility_tree.nodes() {
        if let Some(id) = n.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            keys.push(id.to_string());
        }
        if let Some(label) = n
            .get("label")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            keys.push(label.to_string());
        }
    }
    keys.sort();
    keys.dedup();
    keys.truncate(32);
    keys
}

fn normalize_steps(steps: &[Value]) -> Vec<Value> {
    steps
        .iter()
        .map(|s| {
            let mut o = s.clone();
            if let Some(obj) = o.as_object_mut() {
                if !obj.contains_key("op") {
                    if let Some(action) = obj.get("action").and_then(|v| v.as_str()) {
                        obj.insert("op".into(), json!(action));
                    }
                }
            }
            o
        })
        .collect()
}

fn fault_str(fault: FaultClass) -> &'static str {
    match fault {
        FaultClass::TargetMissing => "target_missing",
        FaultClass::MotorNoEffect => "motor_no_effect",
        FaultClass::MotorRejected => "motor_rejected",
        FaultClass::Blocked => "blocked",
        FaultClass::EyesUnusable => "eyes_unusable",
        FaultClass::Timeout => "timeout",
        _ => "exercise_failed",
    }
}

fn mode_code(mode: RepairMode) -> &'static str {
    match mode {
        RepairMode::TabChromeMissing => "tab_chrome_missing",
        RepairMode::StateGateStuck => "control_fired_no_transition",
        RepairMode::BlockedOverlay => "blocked_overlay",
        RepairMode::AcceptanceNotInAx => "acceptance_not_in_ax",
        RepairMode::TypeNeverCommitted => "type_never_committed",
        RepairMode::MotorRejected => "motor_rejected",
        RepairMode::TargetNeverVisible => "target_never_visible",
        RepairMode::EyesUnusable => "eyes_unusable",
        RepairMode::Unknown => "unknown",
    }
}

fn write_patch(workspace: &Path, patch: &EditPlan) -> Result<(), String> {
    let rel = patch.path.replace('\\', "/");
    if rel.contains("..") {
        return Err("patch path escapes workspace".into());
    }
    let abs = if Path::new(&rel).is_absolute() {
        PathBuf::from(&rel)
    } else {
        workspace.join(&rel)
    };
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&abs, &patch.content).map_err(|e| e.to_string())?;
    Ok(())
}

/// Prove exercise failure → RepairContract; optional patch apply; optional certify re-run.
pub(crate) fn cap_repair_job(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    app: &str,
    bundle_id: Option<&str>,
    exercise: &[Value],
    workspace: Option<&Path>,
    settle_ms: u64,
    timeout_ms: u64,
    install: bool,
    launch_args: Option<&[String]>,
    patch: Option<&EditPlan>,
    certify: bool,
) -> CapabilityResult {
    let t0 = Instant::now();
    let steps = normalize_steps(exercise);
    let root = workspace.unwrap_or_else(|| Path::new("."));

    if let Some(p) = patch {
        if let Err(e) = write_patch(root, p) {
            return CapabilityResult::fail(
                FaultClass::Infra,
                ligh_core::SessionPhase::Ready,
                None,
                "repair_job",
                json!({
                    "phase": "patch",
                    "error": e,
                    "wall_ms": t0.elapsed().as_millis() as u64,
                }),
                None,
            );
        }
    }

    let do_install = install || patch.is_some() || certify;
    let job = app_job(
        build,
        state,
        app,
        bundle_id,
        &steps,
        settle_ms,
        timeout_ms,
        do_install,
        launch_args,
    );

    let wall_ms = t0.elapsed().as_millis() as u64;
    if wall_ms > REPAIR_JOB_WALL_MS {
        return CapabilityResult::fail(
            FaultClass::Timeout,
            job.phase,
            job.surface.clone(),
            "repair_job",
            json!({
                "phase": RepairJobPhase::Timeout,
                "wall_ms": wall_ms,
                "detail": job.detail,
            }),
            job.observe,
        );
    }

    if job.ok {
        return CapabilityResult::success(
            job.phase,
            job.surface,
            "repair_job",
            json!({
                "capability": "repair_job",
                "ok": true,
                "reached": true,
                "verified": certify || patch.is_some(),
                "phase": if certify || patch.is_some() {
                    RepairJobPhase::Done
                } else {
                    RepairJobPhase::TraceExercise
                },
                "wall_ms": wall_ms,
            }),
            job.observe,
        );
    }

    let detail = job.detail.clone().unwrap_or_else(|| json!({}));
    let step_i = detail.get("step").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let op = detail
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or("tap")
        .to_string();
    let expected = steps
        .get(step_i)
        .and_then(|s| {
            s.get("id")
                .and_then(|v| v.as_str())
                .or_else(|| s.get("label").and_then(|v| v.as_str()))
        })
        .unwrap_or("")
        .to_string();
    let observed = ax_identities(&job.observe);
    let fault = fault_str(job.fault);
    let tf = TraceFailure {
        step: (step_i as u32).max(1),
        action: op,
        expected_identity: expected.clone(),
        observed_identities: observed,
        fault: fault.to_string(),
        scene_before: None,
        scene_after: None,
        label: Some(expected.clone()),
    };
    let mode = repair_mode_from_trace(&tf.fault, &tf.expected_identity);

    let mut goal = PilotGoal::default();
    if !expected.is_empty() {
        goal.target_id = Some(expected.clone());
        goal.all.push(ligh_core::GoalPredicate {
            id: Some(expected.clone()),
            ..Default::default()
        });
    }

    let diagnosis = PilotDiagnosis {
        code: mode_code(mode).into(),
        message: format!(
            "exercise step {} failed ({fault}) for '{expected}'",
            tf.step
        ),
        fingerprint: None,
        control: Some(expected.clone()),
    };

    let (repair_view, fixer_input) = if let Some(ref snap) = job.observe {
        let view = perceive_from_snap(snap);
        let feel = build_feel(&view, snap, None, Some(settle_ms));
        let mut contract = emit_repair_contract(&diagnosis, &goal, &feel, Some(root));
        contract.mode = mode;
        contract = contract_with_trace(contract, tf.clone());
        let plan = fix_plan_for_mode(&contract);
        contract.scope.edit_intent = plan.clone();
        (
            repair_agent_view(&contract),
            json!({
                "fix_plan": plan,
                "primary_path": contract.scope.primary_path,
            }),
        )
    } else {
        (
            json!({
                "mode": mode,
                "diagnosis_code": diagnosis.code,
                "invariant": diagnosis.message,
            }),
            json!({
                "fix_plan": format!("Fix exercise failure for '{expected}'"),
                "primary_path": Value::Null,
            }),
        )
    };

    let _ = bundle_id; // reserved for future ownership checks

    CapabilityResult::fail(
        job.fault,
        job.phase,
        job.surface,
        "repair_job",
        json!({
            "capability": "repair_job",
            "ok": false,
            "reached": false,
            "verified": false,
            "phase": RepairJobPhase::Localize,
            "mode": mode,
            "wall_ms": wall_ms,
            "trace_failure": tf,
            "repair_contract": repair_view,
            "fixer_input": fixer_input,
            "detail": detail,
        }),
        job.observe,
    )
}
