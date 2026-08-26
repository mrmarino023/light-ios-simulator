//! Core types and configuration for LIGH.

pub mod autopilot;
pub mod config;
pub mod control;
pub mod device;
pub mod error;
pub mod feel;
pub mod observe;
pub mod profile;
pub mod qa;
pub mod repair;
pub mod rpc;
pub mod scene;
pub mod speculate;
pub mod state;
pub mod uxgraph;

pub use autopilot::{
    act_valid_on_feel, diagnose, next_act, recovery_stage, CandidateGenerator, GoalPredicate,
    GoalSpec, MotorTypeStrategy, PilotAct, PilotDiagnosis, PilotGoal, PilotIntent, PilotLimits,
    PilotMemory, PilotParam, PilotSlot, PilotStepRecord, RecoveryStage, SearchPolicy,
    AUTOPILOT_SCHEMA_VERSION,
};
pub use config::LighConfig;
pub use control::{
    eyes_unusable, overlay_from_snapshot, phase_from_snapshot, stamp_control_fields,
    ActionOutcome, ActionSpec, CapabilityResult, EpochStamp, FaultClass, FaultDomain, Overlay, SessionPhase,
    TargetIdentity,
};
pub use device::DevicePreset;
pub use error::{LighError, Result};
pub use observe::{
    build_actionable_topk, build_scene, detect_surface, diff_sense_events, eyes_ready,
    find_hittable_id_in_dump, find_hittable_label_in_dump, find_id_center, find_id_in_dump,
    find_label_center, find_label_in_dump, find_onscreen_id_in_dump, foreground_app_label,
    identity_suggests_tab_label, is_chrome_node, is_editable_role, is_tab_bar_node,
    is_transition_sparse, label_is_noisy, node_matches_identifier, node_matches_identity_needle,
    node_viewport_hittable,
    rank_candidates, tab_chrome_alias_matches, AccessibilityTree, FrameMeta, ObserveSnapshot,
    SceneMeta, SenseEvent, ACTIONABLE_TOPK, OBSERVE_SCHEMA_VERSION,
};
pub use feel::{
    build_feel, feel_agent_view, suggest_act, FeelBlock, FeelDelta, FeelIR, FeelMeta, FeelPhase,
    FeelPlace, FeelSuggestedAct, SalienceItem, WorldElement, WorldModel, FEEL_SCHEMA_VERSION,
};
pub use scene::{
    atoms_from_nodes, build_scene_digest, regionize, scene_agent_view, DynamicClass, MotorOp,
    RegionKind, RegionRole, SceneAtom, SceneDigest, SceneDynamic, SceneMotor, ScenePhase,
    ScenePlace, SceneRegion, SceneScroll, SceneTrust, SCENE_SCHEMA_VERSION,
};
pub use speculate::{
    apply_verdict, begin_speculate, certify, may_speculate, predict_after_act,
    predict_after_act_on_feel, speculate_enabled, SpecLevel, SpecPhase, SpecPrediction, SpecStats,
    SpecTicket, SpecVerdict, SPECULATE_SCHEMA_VERSION,
};
pub use qa::{
    build_perceive, evaluate_attempt, fingerprint_of, infer_affordances, parse_expectation,
    screen_fingerprint, Affordance, AffordanceKind, AttemptEvidence, AttemptVerdict, BlockingView,
    Expectation, Hypothesis, LocationView, PerceiveView,
};
pub use uxgraph::{
    default_compiled_path, default_graph_path, resolve_workspace, CompiledFlow, ExploreResult,
    ExploreStep, GraphDiff, GraphSummary, ScreenChange, SourceHint, UxBaseline, UxGraph,
    UxGraphStats, UxScreenNode, UxTransitionEdge, UXGRAPH_SCHEMA_VERSION,
};
pub use profile::{FeatureRequirements, resolve_disabled_jobs, slim_labels};
pub use repair::{
    classify_trace_effect, contract_with_trace, emit_repair_contract, fix_plan_for_mode,
    path_allowed_by_contract, repair_agent_view, repair_mode_from_trace, repair_scope, world_evidence,
    EditPlan, PatchSource, RepairContract, RepairJobPhase, RepairJobResult, RepairMode, RepairScope,
    TraceFailure, WorldEvidence, REPAIR_JOB_WALL_MS, REPAIR_SCHEMA_VERSION, TRAIL_PROVE_LOCALIZE_MS,
};
pub use rpc::{
    default_sock_path, ensure_daemon, sibling_lighd, DaemonClient, DaemonRequest, DaemonResponse,
};
pub use state::SessionState;

#[cfg(test)]
mod tests {
    use super::DevicePreset;

    #[test]
    fn parses_device_presets() {
        assert_eq!(
            "iphone-15-pro".parse::<DevicePreset>().unwrap(),
            DevicePreset::Iphone15Pro
        );
    }

    #[test]
    fn iphone_15_pro_hid_points_from_framebuffer() {
        let (w, h) = DevicePreset::Iphone15Pro.hid_size_from_framebuffer(1179, 2556);
        assert!((w - 393.0).abs() < 0.01);
        assert!((h - 852.0).abs() < 0.01);
    }
}
