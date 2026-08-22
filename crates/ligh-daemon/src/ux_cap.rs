//! UX Graph capabilities — persist, regress, explore, source hints.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ligh_core::{
    default_graph_path, resolve_workspace, CapabilityResult, ExploreResult, ExploreStep,
    FaultClass, ObserveSnapshot, SessionPhase, UxGraph,
};
use serde_json::json;

use crate::capabilities::{ensure_ready, phase_of, settle_eyes, surface_of};
use crate::qa_cap::{cap_attempt, cap_perceive, perceive_from_snap};
use crate::DaemonState;

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
) {
    let path = graph_file(workspace);
    let mut g = load_graph(&path);
    g.record_attempt(pre, verdict, label, id);
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
    build: &dyn Fn() -> ObserveSnapshot>,
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
            from_fp,
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
