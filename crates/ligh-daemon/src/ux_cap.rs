//! UX Graph capabilities — persist, regress, explore, source hints.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ligh_core::{
    default_compiled_path, default_graph_path, resolve_workspace, CapabilityResult, CompiledFlow,
    ExploreResult, ExploreStep, FaultClass, ObserveSnapshot, SessionPhase, UxGraph,
};
use serde_json::json;

use crate::capabilities::{ensure_ready, phase_of, settle_eyes, surface_of};
use crate::motor::motor_tap;
use crate::qa_cap::{cap_attempt, cap_perceive, perceive_from_snap};
use crate::DaemonState;

fn app_label_from_path(app: &str) -> String {
    Path::new(app)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app")
        .to_string()
}

fn affordance_keys(view: &ligh_core::PerceiveView) -> HashSet<String> {
    view.affordances
        .iter()
        .flat_map(|a| [a.id.clone(), a.label.clone()].into_iter().flatten())
        .collect()
}

fn on_springboard(view: &ligh_core::PerceiveView, app_label: &str) -> bool {
    view.affordances.iter().any(|a| {
        a.label.as_deref() == Some(app_label) || a.id.as_deref() == Some(app_label)
    })
}

/// After run_app, sim scene can report bundle_id while AX tree is still SpringBoard.
fn ensure_app_foreground(
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    app: &str,
    bundle_id: &str,
    entry_id: Option<&str>,
    entry_label: Option<&str>,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let app_label = app_label_from_path(app);
    let _ = timeout_ms;

    for attempt in 1..=5u32 {
        let snap = settle_eyes(build, settle_ms);
        let trust = crate::capabilities::confirm_app_ready(
            &snap,
            bundle_id,
            &app_label,
            entry_label,
            entry_id,
        );
        if trust.ok {
            return CapabilityResult::success(
                trust.phase,
                trust.surface.clone(),
                "ensure_foreground",
                json!({ "attempt": attempt, "via": "app_ready", "detail": trust.detail }),
                trust.observe,
            );
        }

        if only_springboard_with_icon(&snap, &app_label) {
            let udid = match state.lock().unwrap().current_udid() {
                Ok(u) => u,
                Err(e) => {
                    return CapabilityResult::fail(
                        FaultClass::Infra,
                        SessionPhase::Dead,
                        None,
                        "ensure_foreground",
                        json!({ "error": e }),
                        Some(snap),
                    );
                }
            };
            let _ = ligh_sim::Simctl::run(&["launch", &udid, bundle_id]);
            thread::sleep(Duration::from_millis(1200));
            let _ = motor_tap(
                build,
                state,
                Some(app_label.as_str()),
                None,
                settle_ms.min(2000),
                timeout_ms.min(8000),
                None,
                None,
            );
            thread::sleep(Duration::from_millis(1000));
            continue;
        }

        if trust.fault == FaultClass::AppNotForeground {
            let udid = match state.lock().unwrap().current_udid() {
                Ok(u) => u,
                Err(e) => {
                    return CapabilityResult::fail(
                        FaultClass::Infra,
                        SessionPhase::Dead,
                        None,
                        "ensure_foreground",
                        json!({ "error": e }),
                        Some(snap),
                    );
                }
            };
            let _ = ligh_sim::Simctl::run(&["launch", &udid, bundle_id]);
            thread::sleep(Duration::from_millis(1500));
            continue;
        }
    }

    let snap = settle_eyes(build, settle_ms);
    CapabilityResult::fail(
        FaultClass::AppNotForeground,
        phase_of(&snap),
        surface_of(&snap),
        "ensure_foreground",
        json!({
            "reason": "app_not_foreground",
            "app_label": app_label,
            "entry_id": entry_id,
            "entry_label": entry_label,
        }),
        Some(snap),
    )
}

fn only_springboard_with_icon(snap: &ObserveSnapshot, app_label: &str) -> bool {
    crate::capabilities::confirm_app_ready(snap, "", app_label, None, None).fault
        == FaultClass::AppNotForeground
        && snap
            .accessibility_tree
            .nodes()
            .iter()
            .any(|n| {
                n.get("label").and_then(|v| v.as_str()) == Some(app_label)
                    || n.get("identifier").and_then(|v| v.as_str()) == Some(app_label)
            })
}

pub(crate) fn graph_file(workspace: Option<&Path>) -> PathBuf {
    if let Ok(p) = std::env::var("LIGH_UXGRAPH_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    default_graph_path(&resolve_workspace(workspace))
}

fn load_graph(path: &Path) -> UxGraph {
    UxGraph::load(path).unwrap_or_else(|_| UxGraph::new())
}

fn save_graph(graph: &mut UxGraph, path: &Path) -> Result<(), String> {
    graph.save(path).map_err(|e| e.to_string())
}

pub(crate) fn ux_persist_perceive(workspace: Option<&Path>, view: &ligh_core::PerceiveView) {
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    g.record_perceive(view);
    let _ = save_graph(&mut g, &path);
}

pub(crate) fn ux_persist_attempt(
    workspace: Option<&Path>,
    pre: &ligh_core::PerceiveView,
    verdict: &ligh_core::AttemptVerdict,
    label: Option<&str>,
    id: Option<&str>,
    text: Option<&str>,
) {
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    g.record_attempt(pre, verdict, label, id, text);
    let _ = save_graph(&mut g, &path);
}

pub(crate) fn cap_ux_status(workspace: Option<&Path>) -> CapabilityResult {
    let path = graph_file(workspace);
    let g = load_graph(&path);
    CapabilityResult::success(
        SessionPhase::Ready,
        None,
        "ux_status",
        json!({
            "path": path.display().to_string(),
            "summary": g.summary(),
            "stats": g.stats,
            "baselines": g.baselines.keys().collect::<Vec<_>>(),
            "active_baseline": g.active_baseline,
        }),
        None,
    )
}

pub(crate) fn cap_ux_baseline(
    workspace: Option<&Path>,
    name: &str,
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
) -> CapabilityResult {
    let ready = ensure_ready(build, state, settle_ms, 3);
    if !ready.ok {
        return ready;
    }
    let snap = ready
        .observe
        .clone()
        .unwrap_or_else(|| settle_eyes(build, settle_ms));
    let view = perceive_from_snap(&snap);
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    g.record_perceive(&view);
    g.set_baseline(name);
    if let Err(e) = save_graph(&mut g, &path) {
        return CapabilityResult::fail(
            FaultClass::Infra,
            phase_of(&snap),
            surface_of(&snap),
            "ux_baseline",
            json!({ "error": e }),
            Some(snap),
        );
    }
    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "ux_baseline",
        json!({
            "baseline": name,
            "fingerprints": g.baselines.get(name).map(|b| b.fingerprints.clone()),
            "path": path.display().to_string(),
        }),
        Some(snap),
    )
}

pub(crate) fn cap_ux_regress(
    workspace: Option<&Path>,
    baseline: &str,
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
) -> CapabilityResult {
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    if !g.baselines.contains_key(baseline) {
        return CapabilityResult::fail(
            FaultClass::Model,
            SessionPhase::Ready,
            None,
            "ux_regress",
            json!({
                "error": format!("unknown baseline: {baseline}"),
                "known": g.baselines.keys().collect::<Vec<_>>(),
            }),
            None,
        );
    }
    let p = cap_perceive(build, state, settle_ms, workspace);
    if !p.ok {
        return p;
    }
    let snap = p.observe.clone().unwrap_or_else(|| settle_eyes(build, settle_ms));
    let view = perceive_from_snap(&snap);
    g.record_perceive(&view);
    let diff = g.regress(baseline, &view);
    let pass = diff.regress_pass;
    let _ = save_graph(&mut g, &path);
    if pass {
        CapabilityResult::success(
            phase_of(&snap),
            surface_of(&snap),
            "ux_regress",
            json!({ "diff": diff, "perceive": view, "path": path.display().to_string() }),
            Some(snap),
        )
    } else {
        CapabilityResult::fail(
            FaultClass::Model,
            phase_of(&snap),
            surface_of(&snap),
            "ux_regress",
            json!({ "diff": diff, "perceive": view, "path": path.display().to_string() }),
            Some(snap),
        )
    }
}

pub(crate) fn cap_ux_hint(
    workspace: Option<&Path>,
    fingerprint: &str,
    source_path: &str,
) -> CapabilityResult {
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    g.record_source_hint(fingerprint, source_path);
    if let Err(e) = save_graph(&mut g, &path) {
        return CapabilityResult::fail(
            FaultClass::Infra,
            SessionPhase::Ready,
            None,
            "ux_hint",
            json!({ "error": e }),
            None,
        );
    }
    let hints = g.source_hints_for(fingerprint);
    CapabilityResult::success(
        SessionPhase::Ready,
        None,
        "ux_hint",
        json!({
            "fingerprint": fingerprint,
            "source_path": source_path,
            "hints": hints,
            "path": path.display().to_string(),
        }),
        None,
    )
}

pub(crate) fn cap_ux_explore(
    workspace: Option<&Path>,
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    max_steps: u32,
    max_depth: u32,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    let nodes_before = g.nodes.len();
    g.stats.explores += 1;

    let mut trace = Vec::new();
    let mut tried: HashSet<String> = HashSet::new();
    let mut known_fps: HashSet<String> = g.nodes.keys().cloned().collect();
    let mut depth = 0u32;

    let p0 = cap_perceive(build, state, settle_ms, workspace);
    if !p0.ok {
        return p0;
    }
    let mut snap = p0.observe.clone().unwrap_or_else(|| settle_eyes(build, settle_ms));
    let mut current = perceive_from_snap(&snap);
    known_fps.insert(current.location.fingerprint.clone());

    for step in 1..=max_steps {
        if depth >= max_depth {
            break;
        }
        let Some((aff, key)) = g.plan_explore_tap(&current, &tried) else {
            break;
        };
        tried.insert(key);
        let label = aff.label.as_deref();
        let id = aff.id.as_deref();
        let from_fp = current.location.fingerprint.clone();

        let attempt = cap_attempt(
            build,
            state,
            "tap",
            label,
            id,
            None,
            None,
            None,
            settle_ms,
            timeout_ms,
            workspace,
        );
        snap = attempt
            .observe
            .clone()
            .unwrap_or_else(|| settle_eyes(build, settle_ms));
        current = perceive_from_snap(&snap);
        let to_fp = current.location.fingerprint.clone();
        let new_screen = !known_fps.contains(&to_fp);
        known_fps.insert(to_fp.clone());

        let intent_met = attempt
            .detail
            .as_ref()
            .and_then(|d| d.get("verdict"))
            .and_then(|v| v.get("intent_met"))
            .and_then(|v| v.as_bool())
            .unwrap_or(attempt.ok);

        trace.push(ExploreStep {
            step,
            action: "tap".into(),
            target_label: aff.label.clone(),
            target_id: aff.id.clone(),
            from_fp: from_fp.clone(),
            to_fp: to_fp.clone(),
            intent_met,
            new_screen,
        });

        if to_fp != from_fp {
            depth += 1;
        }

        if !attempt.ok && !intent_met {
            break;
        }
    }

    if let Err(e) = save_graph(&mut g, &path) {
        return CapabilityResult::fail(
            FaultClass::Infra,
            phase_of(&snap),
            surface_of(&snap),
            "ux_explore",
            json!({ "error": e }),
            Some(snap),
        );
    }

    let result = ExploreResult {
        steps_taken: trace.len() as u32,
        screens_discovered: g.count_new_screens_after(nodes_before),
        transitions_recorded: trace.len() as u32,
        trace,
        graph_summary: g.summary(),
    };

    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "ux_explore",
        json!({ "explore": result, "path": path.display().to_string() }),
        Some(snap),
    )
}

pub(crate) fn cap_ux_compile_flow(
    workspace: Option<&Path>,
    goal_id: &str,
) -> CapabilityResult {
    let ws = resolve_workspace(workspace);
    let graph_path = graph_file(workspace);
    let g = load_graph(&graph_path);
    match g.compile_flow(goal_id) {
        Ok(flow) => {
            let out = default_compiled_path(&ws, goal_id);
            if let Err(e) = flow.save(&out) {
                return CapabilityResult::fail(
                    FaultClass::Infra,
                    SessionPhase::Ready,
                    None,
                    "ux_compile_flow",
                    json!({ "error": e.to_string() }),
                    None,
                );
            }
            CapabilityResult::success(
                SessionPhase::Ready,
                None,
                "ux_compile_flow",
                json!({
                    "goal_id": goal_id,
                    "steps": flow.steps.len(),
                    "confidence": flow.confidence,
                    "source_fps": flow.source_fps,
                    "path": out.display().to_string(),
                    "flow": flow,
                }),
                None,
            )
        }
        Err(e) => CapabilityResult::fail(
            FaultClass::Model,
            SessionPhase::Ready,
            None,
            "ux_compile_flow",
            json!({ "error": e, "graph_edges": g.edges.len(), "graph_nodes": g.nodes.len() }),
            None,
        ),
    }
}

pub(crate) fn cap_ux_execute_compiled(
    workspace: Option<&Path>,
    goal_id: &str,
    app: &str,
    bundle_id: Option<&str>,
    build: &dyn Fn() -> ObserveSnapshot,
    state: &Arc<Mutex<DaemonState>>,
    settle_ms: u64,
    timeout_ms: u64,
) -> CapabilityResult {
    use crate::capabilities::{app_job, run_motor_step};

    let ws = resolve_workspace(workspace);
    let compiled_path = default_compiled_path(&ws, goal_id);
    let flow = match CompiledFlow::load(&compiled_path) {
        Ok(f) => f,
        Err(e) => {
            return CapabilityResult::fail(
                FaultClass::Infra,
                SessionPhase::Ready,
                None,
                "ux_execute_compiled",
                json!({ "error": e.to_string(), "path": compiled_path.display().to_string() }),
                None,
            );
        }
    };

    let ready = ensure_ready(build, state, settle_ms, 4);
    if !ready.ok {
        return ready;
    }

    let run = app_job(
        build,
        state,
        app,
        bundle_id,
        &[],
        settle_ms,
        timeout_ms,
        true,
        None,
    );
    if !run.ok {
        return CapabilityResult::fail(
            run.fault,
            run.phase,
            run.surface,
            "ux_execute_compiled",
            json!({ "stage": "run_app", "detail": run.detail }),
            run.observe,
        );
    }

    let first = flow.steps.first();
    let entry_id = first.and_then(|s| s.get("id")).and_then(|v| v.as_str());
    let entry_label = first.and_then(|s| s.get("label")).and_then(|v| v.as_str());
    let bid = bundle_id.unwrap_or("");
    if !bid.is_empty() {
        let fg = ensure_app_foreground(
            build,
            state,
            app,
            bid,
            entry_id,
            entry_label,
            settle_ms,
            timeout_ms,
        );
        if !fg.ok {
            return CapabilityResult::fail(
                fg.fault,
                fg.phase,
                fg.surface,
                "ux_execute_compiled",
                json!({ "stage": "foreground", "detail": fg.detail }),
                fg.observe,
            );
        }
    }

    let mut trace = Vec::new();
    for (i, step) in flow.steps.iter().enumerate() {
        let r = run_motor_step(build, state, step, settle_ms, timeout_ms);
        trace.push(json!({ "step": i, "op": step.get("op"), "ok": r.ok, "fault": format!("{:?}", r.fault) }));
        if !r.ok {
            return CapabilityResult::fail(
                r.fault,
                r.phase,
                r.surface,
                "ux_execute_compiled",
                json!({
                    "stage": "motor",
                    "step_index": i,
                    "step": step,
                    "trace": trace,
                    "compiled_path": compiled_path.display().to_string(),
                }),
                r.observe,
            );
        }
    }

    let snap = settle_eyes(build, settle_ms);
    let view = perceive_from_snap(&snap);
    let found = view.affordances.iter().any(|a| a.id.as_deref() == Some(goal_id))
        || view
            .affordances
            .iter()
            .any(|a| a.label.as_deref() == Some(goal_id));
    if !found {
        return CapabilityResult::fail(
            FaultClass::TargetMissing,
            phase_of(&snap),
            surface_of(&snap),
            "ux_execute_compiled",
            json!({
                "stage": "verify",
                "goal_id": goal_id,
                "trace": trace,
                "perceive": view,
            }),
            Some(snap),
        );
    }

    CapabilityResult::success(
        phase_of(&snap),
        surface_of(&snap),
        "ux_execute_compiled",
        json!({
            "goal_id": goal_id,
            "steps_executed": flow.steps.len(),
            "confidence": flow.confidence,
            "compiled_path": compiled_path.display().to_string(),
            "trace": trace,
            "verified": true,
            "llm_tokens": 0,
        }),
        Some(snap),
    )
}
