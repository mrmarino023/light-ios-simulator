//! Autopilot policy — generic goal-directed UI planner over Feel IR.
//!
//! App-agnostic by construction: every decision reads affordance *kind* plus live
//! Feel IR state, never app-specific labels or a recorded step list. The task
//! supplies only the acceptance target (`PilotGoal`) and typed data (`PilotParam`);
//! the path is discovered at runtime, one settled frame at a time.
//!
//! Pure module: no simulator, no IO. The daemon executes the returned acts.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::control::{ActionOutcome, ActionSpec, EpochStamp, TargetIdentity};
use crate::feel::{FeelIR, FeelPhase, SalienceItem};
use crate::qa::AffordanceKind;
use crate::uxgraph::is_destructive_label;

pub const AUTOPILOT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalPredicate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PilotSlot {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
}

/// Acceptance target plus typed data. Carries no path and no app-specific steps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    /// Data the flow may need, bound to fields by kind (never by field name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<PilotParam>,
    /// Declarative acceptance contract. Legacy target_id/target_label compile into `all`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<GoalPredicate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub none: Vec<GoalPredicate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starting: Vec<GoalPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_bundle_id: Option<String>,
    #[serde(default = "default_stable_observations")]
    pub stable_observations: u32,
    #[serde(default = "default_stability_window_ms")]
    pub stability_window_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<PilotSlot>,
    #[serde(default)]
    pub allow_destructive: bool,
}

impl GoalSpec {
    pub fn target_name(&self) -> String {
        self.target_id
            .clone()
            .or_else(|| self.target_label.clone())
            .unwrap_or_else(|| "<unset>".into())
    }

    pub fn required_predicates(&self) -> Vec<GoalPredicate> {
        let mut predicates = self.all.clone();
        if let Some(id) = &self.target_id {
            predicates.push(GoalPredicate {
                id: Some(id.clone()),
                ..Default::default()
            });
        } else if let Some(label) = &self.target_label {
            predicates.push(GoalPredicate {
                label: Some(label.clone()),
                ..Default::default()
            });
        }
        predicates
    }

    pub fn effective_params(&self) -> Vec<PilotParam> {
        if self.slots.is_empty() {
            return self.params.clone();
        }
        self.slots
            .iter()
            .map(|slot| PilotParam {
                value: slot.value.clone(),
                secure: slot.secure,
            })
            .collect()
    }
}

/// Compatibility name for existing RPC/CLI clients. The contract is GoalSpec v2.
pub type PilotGoal = GoalSpec;

fn default_stable_observations() -> u32 {
    2
}

fn default_stability_window_ms() -> u64 {
    150
}

/// One datum for the flow. `secure` binds it to a secure text field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PilotParam {
    pub value: String,
    #[serde(default)]
    pub secure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PilotIntent {
    Type,
    Tap,
    Dismiss,
    Scroll,
    Back,
    Wait,
    Stop,
}

/// Verified type motor. Each failed type must escalate to the next strategy;
/// the same ActionSpec is never fired twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotorTypeStrategy {
    /// HID-tap the typeable node, then type. AX press is not trusted as first responder.
    #[default]
    FocusHid,
    /// Tap, wait for keyboard/focus, type even if AX does not reflect focus.
    TapThenHid,
    /// Tap, clear existing value, retype.
    ClearRetype,
    /// Tap slightly inside the field body (not the labeled container center).
    CoordOffsetHid,
}

impl MotorTypeStrategy {
    pub const ALL: [Self; 4] = [
        Self::FocusHid,
        Self::TapThenHid,
        Self::ClearRetype,
        Self::CoordOffsetHid,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FocusHid => "focus_hid",
            Self::TapThenHid => "tap_then_hid",
            Self::ClearRetype => "clear_retype",
            Self::CoordOffsetHid => "coord_offset_hid",
        }
    }

    pub fn from_attempt(attempt: u32) -> Option<Self> {
        Self::ALL.get(attempt as usize).copied()
    }
}

/// Layered recovery. Labels are not enough: the host must execute the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStage {
    WaitStable,
    RefreshAndResolve,
    AlternateMotor,
    DismissBlockingScope,
    ReacquireForeground,
    Relaunch,
    StopInfrastructure,
}

impl RecoveryStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WaitStable => "wait_stable_scene",
            Self::RefreshAndResolve => "refresh_ax_and_resolve",
            Self::AlternateMotor => "alternate_verified_motor",
            Self::DismissBlockingScope => "dismiss_blocking_scope",
            Self::ReacquireForeground => "reacquire_foreground",
            Self::Relaunch => "relaunch_and_invalidate_epoch",
            Self::StopInfrastructure => "stop_infrastructure_fault",
        }
    }

    /// Later stages are strictly more severe. Used to prove escalation is monotonic.
    pub fn rank(self) -> u8 {
        match self {
            Self::WaitStable => 0,
            Self::RefreshAndResolve => 1,
            Self::AlternateMotor => 2,
            Self::DismissBlockingScope => 3,
            Self::ReacquireForeground => 4,
            Self::Relaunch => 5,
            Self::StopInfrastructure => 6,
        }
    }
}

/// Map a verified-failed outcome onto the next recovery stage.
/// `consecutive_failures` is the failure count for this field/control *after*
/// the current mark (1 = first failure).
pub fn recovery_stage(
    outcome: ActionOutcome,
    intent: PilotIntent,
    blocked: bool,
    consecutive_failures: u32,
) -> RecoveryStage {
    match outcome {
        ActionOutcome::DeliveredAndVerified => RecoveryStage::RefreshAndResolve,
        ActionOutcome::TransitionInProgress => RecoveryStage::WaitStable,
        ActionOutcome::TargetStale => RecoveryStage::RefreshAndResolve,
        ActionOutcome::WrongSurface if consecutive_failures <= 1 => {
            RecoveryStage::ReacquireForeground
        }
        ActionOutcome::WrongSurface => RecoveryStage::Relaunch,
        ActionOutcome::InfrastructureFault => RecoveryStage::StopInfrastructure,
        ActionOutcome::NotDelivered | ActionOutcome::DeliveredNoEffect => {
            if blocked && intent != PilotIntent::Dismiss {
                return RecoveryStage::DismissBlockingScope;
            }
            if intent == PilotIntent::Type {
                if MotorTypeStrategy::from_attempt(consecutive_failures).is_some() {
                    return RecoveryStage::AlternateMotor;
                }
                if consecutive_failures == MotorTypeStrategy::ALL.len() as u32 {
                    return RecoveryStage::DismissBlockingScope;
                }
                return RecoveryStage::Relaunch;
            }
            if consecutive_failures <= 2 {
                RecoveryStage::AlternateMotor
            } else if consecutive_failures == 3 {
                RecoveryStage::DismissBlockingScope
            } else {
                RecoveryStage::Relaunch
            }
        }
    }
}

impl PilotIntent {
    pub fn as_str(self) -> &'static str {
        match self {
            PilotIntent::Type => "type",
            PilotIntent::Tap => "tap",
            PilotIntent::Dismiss => "dismiss",
            PilotIntent::Scroll => "scroll",
            PilotIntent::Back => "back",
            PilotIntent::Wait => "wait",
            PilotIntent::Stop => "stop",
        }
    }
}

/// One host act chosen by the policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PilotAct {
    pub intent: PilotIntent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Text to enter. Redacted in traces when `secure`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<AffordanceKind>,
    pub reason: String,
    /// Dedupe handle, unique per (screen, target, motor strategy).
    pub key: String,
    /// Terminal reason when `intent == Stop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_code: Option<String>,
    /// Type motor. Absent for non-type intents. Included in `key` so a failed
    /// type cannot replay the same ActionSpec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motor_strategy: Option<MotorTypeStrategy>,
}

impl PilotAct {
    pub fn is_terminal(&self) -> bool {
        self.intent == PilotIntent::Stop
    }

    pub fn target_name(&self) -> String {
        self.id
            .clone()
            .or_else(|| self.label.clone())
            .unwrap_or_else(|| "<unnamed>".into())
    }

    /// Stable memory identity: type strategies share one fill key so a failed
    /// type escalates instead of looking like a new field.
    pub fn memory_key(&self) -> String {
        if self.intent == PilotIntent::Type {
            type_fill_key(self)
        } else {
            self.key.clone()
        }
    }

    /// Trace-safe view: secure text never leaves the host as plaintext.
    pub fn trace(&self) -> serde_json::Value {
        serde_json::json!({
            "intent": self.intent.as_str(),
            "label": self.label,
            "id": self.id,
            "kind": self.kind,
            "text": self.text.as_ref().map(|t| if self.secure { "***".to_string() } else { t.clone() }),
            "slot": self.slot_name,
            "reason": self.reason,
            "stop_code": self.stop_code,
            "motor_strategy": self.motor_strategy.map(|s| s.as_str()),
        })
    }

    pub fn action_spec(&self, epoch: EpochStamp, step: u32) -> ActionSpec {
        ActionSpec {
            action_id: format!("action-{}-{step}", epoch.launch_epoch),
            epoch,
            target: if self.id.is_some() || self.label.is_some() {
                Some(TargetIdentity {
                    stable_key: self.key.clone(),
                    identifier: self.id.clone(),
                    label: self.label.clone(),
                    role: self.kind.map(|kind| format!("{kind:?}")),
                    frame_bucket: None,
                })
            } else {
                None
            },
            operation: serde_json::json!({
                "intent": self.intent,
                "text": self.text.as_ref().map(|text| if self.secure { "***" } else { text.as_str() }),
                "slot": self.slot_name,
                "motor_strategy": self.motor_strategy.map(|s| s.as_str()),
            }),
            preconditions: vec![
                serde_json::json!({"epoch_matches": true}),
                serde_json::json!({"foreground_owned": true}),
            ],
            postconditions: vec![serde_json::json!({
                "delivery_and_relevant_effect_verified": true
            })],
        }
    }
}

/// What the pilot has already done, so it never repeats a dead act.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PilotMemory {
    pub tried: HashSet<String>,
    pub filled: HashSet<String>,
    /// Parameters consumed across screens. Fingerprints are deliberately not
    /// used for fields: focus, validation and keyboard changes can alter a
    /// fingerprint without creating a new form.
    pub plain_params_used: usize,
    pub secure_params_used: usize,
    pub scrolls: HashMap<String, u32>,
    pub backs: u32,
    pub steps: u32,
    #[serde(default)]
    pub failures: HashMap<String, u32>,
    #[serde(default)]
    pub transitions: Vec<PilotTransition>,
    #[serde(default)]
    pub slots_used: HashSet<String>,
    #[serde(default)]
    pub screen_actions: HashMap<String, u32>,
    #[serde(default)]
    pub recoveries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PilotTransition {
    pub state: String,
    pub action_key: String,
    pub outcome: ActionOutcome,
    pub next_state: String,
}

impl PilotMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compatibility helper for tests and external users: records verified success.
    pub fn mark(&mut self, fp: &str, act: &PilotAct) {
        self.mark_outcome(fp, act, ActionOutcome::DeliveredAndVerified, fp);
    }

    /// Commit durable planner memory only after delivery and relevant UI effect
    /// were verified. Rejected/no-effect actions remain available to recovery.
    pub fn mark_outcome(
        &mut self,
        fp: &str,
        act: &PilotAct,
        outcome: ActionOutcome,
        next_fp: &str,
    ) {
        self.steps += 1;
        *self.screen_actions.entry(fp.to_string()).or_insert(0) += 1;
        self.transitions.push(PilotTransition {
            state: fp.to_string(),
            action_key: act.key.clone(),
            outcome,
            next_state: next_fp.to_string(),
        });
        if !outcome.memory_committable() {
            let fail_key = act.memory_key();
            *self.failures.entry(fail_key).or_insert(0) += 1;
            self.recoveries = self.recoveries.saturating_add(1);
            return;
        }
        match act.intent {
            PilotIntent::Type => {
                self.filled.insert(type_fill_key(act));
                if let Some(slot) = &act.slot_name {
                    self.slots_used.insert(slot.clone());
                }
                if act.secure {
                    self.secure_params_used += 1;
                } else {
                    self.plain_params_used += 1;
                }
            }
            PilotIntent::Tap | PilotIntent::Dismiss => {
                self.tried.insert(act.key.clone());
            }
            PilotIntent::Scroll => {
                *self.scrolls.entry(fp.to_string()).or_insert(0) += 1;
            }
            PilotIntent::Back => {
                self.backs += 1;
                // Leaving a screen invalidates its scroll budget, not its tried set.
                self.scrolls.remove(fp);
            }
            PilotIntent::Wait | PilotIntent::Stop => {}
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PilotLimits {
    pub max_scrolls_per_screen: u32,
    pub max_backs: u32,
    pub max_actions_per_screen: u32,
    pub max_recoveries: u32,
}

impl Default for PilotLimits {
    fn default() -> Self {
        Self {
            max_scrolls_per_screen: 3,
            max_backs: 2,
            max_actions_per_screen: 12,
            max_recoveries: 6,
        }
    }
}

fn is_field(kind: AffordanceKind) -> bool {
    matches!(
        kind,
        AffordanceKind::TextField | AffordanceKind::SecureField | AffordanceKind::SearchField
    )
}

/// Kinds the pilot may tap while hunting for the goal. `NavBack` is excluded:
/// backing out is a separate, budgeted recovery step.
fn is_tappable(kind: AffordanceKind) -> bool {
    matches!(
        kind,
        AffordanceKind::PrimaryButton
            | AffordanceKind::Button
            | AffordanceKind::Cell
            | AffordanceKind::Link
            | AffordanceKind::Switch
    )
}

fn item_handle(item: &SalienceItem) -> String {
    item.id
        .clone()
        .or_else(|| item.label.clone())
        .unwrap_or_else(|| format!("rank{}", item.rank))
}

fn field_key(item: &SalienceItem) -> String {
    format!("field|{}", item_handle(item))
}

fn type_fill_key(act: &PilotAct) -> String {
    if let Some(strategy) = act.motor_strategy {
        let suffix = format!("|{}", strategy.as_str());
        if let Some(base) = act.key.strip_suffix(&suffix) {
            return base.to_string();
        }
    }
    format!(
        "field|{}",
        act.id
            .clone()
            .or_else(|| act.label.clone())
            .unwrap_or_else(|| "unnamed".into())
    )
}

/// Prefer a WorldModel element that is actually editable and on-screen.
/// Identifier on a labeled container is rebound to a nearby typeable node.
fn bind_typeable(feel: &FeelIR, item: &SalienceItem) -> (Option<String>, Option<String>) {
    let els = &feel.world.elements;
    if els.is_empty() {
        return (item.id.clone(), item.label.clone());
    }
    if let Some(el) = els.iter().find(|e| {
        item.id
            .as_ref()
            .map(|id| e.identifier.as_ref() == Some(id))
            .unwrap_or(false)
            && e.editable
            && e.on_screen
    }) {
        return (
            el.identifier.clone().or(item.id.clone()),
            el.label.clone().or(item.label.clone()),
        );
    }
    if let Some(container) = els.iter().find(|e| {
        item.id
            .as_ref()
            .map(|id| e.identifier.as_ref() == Some(id))
            .unwrap_or(false)
    }) {
        if let Some(near) = els.iter().find(|e| {
            e.editable
                && e.on_screen
                && (e.frame_bucket == container.frame_bucket
                    || (item.label.is_some() && e.label == item.label))
        }) {
            return (
                near.identifier.clone().or(item.id.clone()),
                near.label.clone().or(item.label.clone()),
            );
        }
    }
    (item.id.clone(), item.label.clone())
}

fn tap_key(fp: &str, item: &SalienceItem) -> String {
    format!("{fp}|tap|{:?}|{}", item.kind, item_handle(item))
}

fn act(intent: PilotIntent, key: String, reason: String) -> PilotAct {
    PilotAct {
        intent,
        label: None,
        id: None,
        text: None,
        secure: false,
        slot_name: None,
        kind: None,
        reason,
        key,
        stop_code: None,
        motor_strategy: None,
    }
}

fn stop(code: &str, reason: impl Into<String>) -> PilotAct {
    let mut a = act(PilotIntent::Stop, format!("stop|{code}"), reason.into());
    a.stop_code = Some(code.to_string());
    a
}

fn goal_identity_present(goal: &PilotGoal, feel: &FeelIR) -> bool {
    let predicates = goal.required_predicates();
    if predicates.is_empty() {
        return false;
    }
    predicates.iter().all(|predicate| {
        feel.world.elements.iter().any(|el| {
            let id_ok = predicate.id.as_deref().map_or(true, |id| {
                el.identifier.as_deref() == Some(id)
                    || crate::observe::tab_chrome_alias_matches(
                        id,
                        el.label.as_deref(),
                        el.tab_chrome,
                    )
            });
            let label_ok = predicate.label.as_deref().map_or(true, |lab| {
                el.label
                    .as_deref()
                    .map(|l| l == lab || l.contains(lab))
                    .unwrap_or(false)
            });
            id_ok && label_ok
        }) || feel.salience.iter().any(|s| {
            let id_ok = predicate.id.as_deref().map_or(true, |id| s.id.as_deref() == Some(id));
            let label_ok = predicate.label.as_deref().map_or(true, |lab| {
                s.label
                    .as_deref()
                    .map(|l| l == lab || l.contains(lab))
                    .unwrap_or(false)
            });
            id_ok && label_ok
        })
    })
}

/// After a verified navigation, do not tap catalog noise hoping a missing
/// acceptance identity will materialize. Fields and an untried primary CTA
/// can still be the path; random cells cannot.
fn should_stop_acceptance_absent(goal: &PilotGoal, feel: &FeelIR, mem: &PilotMemory) -> bool {
    if goal_identity_present(goal, feel) {
        return false;
    }
    if next_type(goal, feel, mem).is_some() {
        return false;
    }
    let fp = feel.place.fingerprint.as_str();
    let untried_primary = feel.salience.iter().any(|s| {
        s.kind == AffordanceKind::PrimaryButton
            && !mem.tried.contains(&tap_key(fp, s))
            && mem.failures.get(&tap_key(fp, s)).copied().unwrap_or(0) < 2
    });
    if untried_primary {
        return false;
    }
    mem.transitions.iter().any(|t| {
        t.outcome == ActionOutcome::DeliveredAndVerified && t.state != t.next_state
    })
}

/// Next field to fill, paired with the next unused param of the matching class.
fn next_type(goal: &PilotGoal, feel: &FeelIR, mem: &PilotMemory) -> Option<PilotAct> {
    let params = goal.effective_params();
    if params.is_empty() {
        return None;
    }
    let fields: Vec<&SalienceItem> = feel.salience.iter().filter(|i| is_field(i.kind)).collect();
    if fields.is_empty() {
        return None;
    }

    let target = fields.iter().find(|i| {
        let base = field_key(i);
        !mem.filled.contains(&base)
            && MotorTypeStrategy::from_attempt(mem.failures.get(&base).copied().unwrap_or(0))
                .is_some()
    })?;
    let base = field_key(target);
    let strategy = MotorTypeStrategy::from_attempt(
        mem.failures.get(&base).copied().unwrap_or(0),
    )?;
    let (bound_id, bound_label) = bind_typeable(feel, target);
    let secure = target.kind == AffordanceKind::SecureField;
    let skip = if secure {
        mem.secure_params_used
    } else {
        mem.plain_params_used
    };
    let bound_slot = if goal.slots.is_empty() {
        None
    } else {
        let target_text = format!(
            "{} {}",
            target.id.as_deref().unwrap_or(""),
            target.label.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        goal.slots
            .iter()
            .filter(|slot| !mem.slots_used.contains(&slot.name))
            .filter(|slot| slot.secure == secure || target.kind != AffordanceKind::SecureField)
            .max_by_key(|slot| {
                let mut score = 0;
                if target_text.contains(&slot.name.to_ascii_lowercase()) {
                    score += 4;
                }
                if slot
                    .constraints
                    .iter()
                    .any(|c| target_text.contains(&c.to_ascii_lowercase()))
                {
                    score += 3;
                }
                if slot
                    .kind_hint
                    .as_deref()
                    .map(|hint| format!("{:?}", target.kind).to_ascii_lowercase().contains(&hint.to_ascii_lowercase()))
                    .unwrap_or(false)
                {
                    score += 2;
                }
                score
            })
    };
    let param = match bound_slot.map(|slot| PilotParam {
        value: slot.value.clone(),
        secure: slot.secure,
    }).or_else(|| params.iter().filter(|p| p.secure == secure).nth(skip).cloned()) {
        Some(p) => p,
        // AX can report a secure field as a plain text field, so fall back to form
        // order. Never for a search field: that is where typing noise does damage.
        None if target.kind != AffordanceKind::SearchField => {
            params.get(mem.secure_params_used + mem.plain_params_used)?.clone()
        }
        None => return None,
    };

    let mut a = act(
        PilotIntent::Type,
        format!("{}|{}", base, strategy.as_str()),
        format!(
            "fill {:?} (param {} of class) via {}",
            target.kind,
            skip + 1,
            strategy.as_str()
        ),
    );
    a.label = bound_label.or(target.label.clone());
    a.id = bound_id.or(target.id.clone());
    a.text = Some(param.value);
    a.secure = secure;
    a.slot_name = bound_slot.map(|slot| slot.name.clone());
    a.kind = Some(target.kind);
    a.motor_strategy = Some(strategy);
    Some(a)
}

#[derive(Debug, Clone)]
struct PilotCandidate {
    act: PilotAct,
    score: f64,
}

/// Pure action enumeration. It has no app labels or recorded paths and emits
/// only actions whose preconditions hold in the current WorldModel frame.
pub struct CandidateGenerator;

impl CandidateGenerator {
    fn generate(
        goal: &PilotGoal,
        feel: &FeelIR,
        mem: &PilotMemory,
        limits: PilotLimits,
    ) -> Vec<PilotCandidate> {
        let fp = feel.place.fingerprint.as_str();
        let mut out = Vec::new();
        if mem.screen_actions.get(fp).copied().unwrap_or(0) >= limits.max_actions_per_screen {
            if mem.backs < limits.max_backs {
                if let Some(item) = feel.salience.iter().find(|s| s.kind == AffordanceKind::NavBack) {
                    let mut back = act(
                        PilotIntent::Back,
                        format!("{fp}|back"),
                        "screen action budget exhausted — bounded backtrack".into(),
                    );
                    back.label = item.label.clone();
                    back.id = item.id.clone();
                    back.kind = Some(item.kind);
                    out.push(PilotCandidate { act: back, score: 10.0 });
                }
            }
            return out;
        }
        if let Some(a) = next_type(goal, feel, mem) {
            out.push(PilotCandidate { act: a, score: 300.0 });
            return out;
        }
        let keyboard_up = feel.feel.keyboard
            || feel.block.as_ref().map(|b| b.kind == "keyboard").unwrap_or(false);
        if keyboard_up {
            out.push(PilotCandidate {
                act: act(
                    PilotIntent::Dismiss,
                    format!("{fp}|dismiss|keyboard"),
                    "fields filled — clear keyboard to expose controls".into(),
                ),
                score: 250.0,
            });
        }
        for item in &feel.salience {
            if !is_tappable(item.kind)
                || (item.label.is_none() && item.id.is_none())
                || (!goal.allow_destructive
                    && is_destructive_label(item.label.as_deref().unwrap_or("")))
            {
                continue;
            }
            let key = tap_key(fp, item);
            if mem.tried.contains(&key) || mem.failures.get(&key).copied().unwrap_or(0) >= 2 {
                continue;
            }
            let mut candidate = act(
                PilotIntent::Tap,
                key,
                format!("best-first candidate rank {} ({:?})", item.rank, item.kind),
            );
            candidate.label = item.label.clone();
            candidate.id = item.id.clone();
            candidate.kind = Some(item.kind);
            out.push(PilotCandidate {
                act: candidate,
                score: item.weight,
            });
        }
        if let Some(block) = &feel.block {
            if block.kind != "keyboard" {
                let key = format!("{fp}|dismiss|{}", block.kind);
                if !mem.tried.contains(&key) && mem.failures.get(&key).copied().unwrap_or(0) < 2 {
                    out.push(PilotCandidate {
                        act: act(
                            PilotIntent::Dismiss,
                            key,
                            format!("{} exhausted — dismiss blocking scope", block.kind),
                        ),
                        score: 40.0,
                    });
                }
            }
        }
        if mem.scrolls.get(fp).copied().unwrap_or(0) < limits.max_scrolls_per_screen {
            out.push(PilotCandidate {
                act: act(
                    PilotIntent::Scroll,
                    format!("{fp}|scroll"),
                    "bounded search: reveal unobserved controls".into(),
                ),
                score: 20.0,
            });
        }
        if mem.backs < limits.max_backs {
            if let Some(item) = feel.salience.iter().find(|s| s.kind == AffordanceKind::NavBack) {
                let mut a = act(
                    PilotIntent::Back,
                    format!("{fp}|back"),
                    "bounded search: backtrack exhausted state".into(),
                );
                a.label = item.label.clone();
                a.id = item.id.clone();
                a.kind = Some(item.kind);
                out.push(PilotCandidate { act: a, score: 10.0 });
            }
        }
        out
    }
}

/// Bounded best-first selection over live candidates and observed transition
/// outcomes. Failed edges are penalized; verified transitions are never replayed.
pub struct SearchPolicy;

impl SearchPolicy {
    fn select(mut candidates: Vec<PilotCandidate>, mem: &PilotMemory) -> Option<PilotAct> {
        for candidate in &mut candidates {
            candidate.score -=
                30.0 * mem.failures.get(&candidate.act.key).copied().unwrap_or(0) as f64;
        }
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.into_iter().next().map(|c| c.act)
    }
}

/// Choose the next host act. `goal_visible` comes from the host's own AX probe,
/// so the policy never has to guess whether the acceptance target is on screen.
pub fn next_act(
    goal: &PilotGoal,
    feel: &FeelIR,
    mem: &PilotMemory,
    goal_visible: bool,
    limits: PilotLimits,
) -> PilotAct {
    if goal_visible {
        return stop(
            "goal_visible",
            format!("acceptance target {} present", goal.target_name()),
        );
    }
    if mem.recoveries >= limits.max_recoveries {
        return stop(
            "recovery_exhausted",
            "layered recovery budget exhausted without verified progress",
        );
    }

    match feel.feel.phase {
        FeelPhase::EyesUnusable => {
            return act(
                PilotIntent::Wait,
                format!("{}|wait", feel.place.fingerprint),
                "eyes unusable — re-settle before acting".into(),
            )
        }
        FeelPhase::Transition => {
            return act(
                PilotIntent::Wait,
                format!("{}|wait", feel.place.fingerprint),
                "scene in transition".into(),
            )
        }
        _ => {}
    }

    if should_stop_acceptance_absent(goal, feel, mem) {
        return stop(
            "acceptance_not_in_ax",
            format!(
                "navigated to a stable screen that cannot expose {} — the identity is absent from AX",
                goal.target_name()
            ),
        );
    }

    if let Some(a) = SearchPolicy::select(
        CandidateGenerator::generate(goal, feel, mem, limits),
        mem,
    ) {
        return a;
    }

    stop(
        "exhausted",
        format!(
            "no untried control leads to {} from this screen",
            goal.target_name()
        ),
    )
}

/// One executed act plus the observed effect. Feeds diagnosis, not the LLM prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PilotStepRecord {
    pub step: u32,
    #[serde(default)]
    pub action_id: String,
    #[serde(default)]
    pub epoch: EpochStamp,
    pub intent: PilotIntent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<AffordanceKind>,
    pub fp_before: String,
    pub fp_after: String,
    /// Motor accepted and delivered the input.
    pub fired: bool,
    /// Screen fingerprint changed after the act.
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ActionOutcome>,
    #[serde(default)]
    pub goal_progress: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    pub ms: u64,
}

impl PilotStepRecord {
    fn inert(&self) -> bool {
        self.intent == PilotIntent::Tap && self.fired && !self.changed && self.events.is_empty()
    }
}

/// Why the goal was not reached, in terms a code-fixing agent can act on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PilotDiagnosis {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
}

/// Turn the executed trace into one actionable verdict.
pub fn diagnose(goal: &PilotGoal, history: &[PilotStepRecord], feel: &FeelIR) -> PilotDiagnosis {
    let inert_primary = history
        .iter()
        .rev()
        .find(|r| r.inert() && r.kind == Some(AffordanceKind::PrimaryButton));
    let inert_any = history.iter().rev().find(|r| r.inert());

    if let Some(r) = inert_primary.or(inert_any) {
        let control =
            r.id.clone()
                .or_else(|| r.label.clone())
                .unwrap_or_else(|| "<unnamed>".into());
        return PilotDiagnosis {
            code: "control_fired_no_transition".into(),
            message: format!(
                "control '{control}' was hit and the app accepted the input, but the screen did not \
                 change and no state event fired. The navigation out of this screen is gated by \
                 state that never becomes true — inspect what '{control}' sets versus what the view \
                 switches on."
            ),
            fingerprint: Some(r.fp_before.clone()),
            control: Some(control),
        };
    }

    if let Some(block) = &feel.block {
        return PilotDiagnosis {
            code: "blocked_overlay".into(),
            message: format!(
                "the scene stayed blocked by {} and the flow could not proceed",
                block.kind
            ),
            fingerprint: Some(feel.place.fingerprint.clone()),
            control: None,
        };
    }

    if feel.feel.phase == FeelPhase::EyesUnusable {
        return PilotDiagnosis {
            code: "eyes_unusable".into(),
            message: "accessibility was never usable on this screen — perception, not app logic"
                .into(),
            fingerprint: Some(feel.place.fingerprint.clone()),
            control: None,
        };
    }

    if history
        .iter()
        .any(|r| r.changed && r.fired && r.outcome == Some(ActionOutcome::DeliveredAndVerified))
        && !history.iter().any(|r| r.goal_progress)
        && !goal_identity_present(goal, feel)
    {
        return PilotDiagnosis {
            code: "acceptance_not_in_ax".into(),
            message: format!(
                "the host navigated to a stable screen but {} is not in the accessibility tree. \
                 Further content taps cannot create that identity — inspect tab/chrome AX or the \
                 declared acceptance target.",
                goal.target_name()
            ),
            fingerprint: Some(feel.place.fingerprint.clone()),
            control: Some(goal.target_name()),
        };
    }

    if history
        .iter()
        .any(|r| r.intent == PilotIntent::Type && r.outcome == Some(ActionOutcome::DeliveredNoEffect))
        && !history.iter().any(|r| {
            r.intent == PilotIntent::Type && r.outcome == Some(ActionOutcome::DeliveredAndVerified)
        })
    {
        let control = history
            .iter()
            .rev()
            .find(|r| r.intent == PilotIntent::Type)
            .and_then(|r| r.id.clone().or_else(|| r.label.clone()));
        return PilotDiagnosis {
            code: "type_never_committed".into(),
            message: "text was delivered to a resolved field but the value never committed \
                 (AX value hash unchanged). The typeable node was not first responder, or the \
                 field does not accept input."
                .into(),
            fingerprint: Some(feel.place.fingerprint.clone()),
            control,
        };
    }

    if history
        .iter()
        .any(|r| r.intent == PilotIntent::Tap && !r.fired)
    {
        return PilotDiagnosis {
            code: "motor_rejected".into(),
            message: "a control was present but could not be hit (not hittable or off-screen)"
                .into(),
            fingerprint: Some(feel.place.fingerprint.clone()),
            control: None,
        };
    }

    PilotDiagnosis {
        code: "target_never_visible".into(),
        message: format!(
            "{} never appeared after exploring every reachable control — the screen that should \
             expose it is either not built or not reachable",
            goal.target_name()
        ),
        fingerprint: Some(feel.place.fingerprint.clone()),
        control: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feel::{FeelBlock, FeelDelta, FeelMeta, FeelPlace, WorldElement};

    fn item(rank: u32, kind: AffordanceKind, label: &str, id: Option<&str>) -> SalienceItem {
        SalienceItem {
            rank,
            kind,
            label: Some(label.into()),
            id: id.map(|s| s.into()),
            weight: 100.0 - rank as f64,
        }
    }

    fn feel_with(salience: Vec<SalienceItem>, keyboard: bool, block: Option<&str>) -> FeelIR {
        FeelIR {
            schema: 1,
            place: FeelPlace {
                fingerprint: "fp_login".into(),
                surface: Some("app".into()),
                title: Some("Sign in".into()),
                bundle_id: Some("me.demo".into()),
            },
            salience,
            block: block.map(|k| FeelBlock {
                kind: k.into(),
                detail: None,
            }),
            delta: FeelDelta::default(),
            feel: FeelMeta {
                phase: if block.is_some() && block != Some("keyboard") {
                    FeelPhase::Blocked
                } else {
                    FeelPhase::Settled
                },
                keyboard,
                ready: true,
            },
            world: Default::default(),
        }
    }

    fn login_screen(keyboard: bool) -> FeelIR {
        feel_with(
            vec![
                item(
                    1,
                    AffordanceKind::PrimaryButton,
                    "Login",
                    Some("loginButton"),
                ),
                item(2, AffordanceKind::TextField, "Username", Some("userField")),
                item(
                    3,
                    AffordanceKind::SecureField,
                    "Password",
                    Some("passField"),
                ),
            ],
            keyboard,
            if keyboard { Some("keyboard") } else { None },
        )
    }

    fn creds() -> PilotGoal {
        PilotGoal {
            target_id: Some("homeTitle".into()),
            target_label: None,
            params: vec![
                PilotParam {
                    value: "alice".into(),
                    secure: false,
                },
                PilotParam {
                    value: "secret".into(),
                    secure: true,
                },
            ],
            ..Default::default()
        }
    }

    /// Full generic login: fields by kind, keyboard cleared, then the CTA.
    #[test]
    fn drives_login_without_app_specific_knowledge() {
        let goal = creds();
        let mut mem = PilotMemory::new();
        let limits = PilotLimits::default();

        let a1 = next_act(&goal, &login_screen(false), &mem, false, limits);
        assert_eq!(a1.intent, PilotIntent::Type);
        assert_eq!(a1.id.as_deref(), Some("userField"));
        assert_eq!(a1.text.as_deref(), Some("alice"));
        assert!(!a1.secure);
        mem.mark("fp_login", &a1);

        let a2 = next_act(&goal, &login_screen(true), &mem, false, limits);
        assert_eq!(a2.intent, PilotIntent::Type);
        assert_eq!(a2.id.as_deref(), Some("passField"));
        assert_eq!(a2.text.as_deref(), Some("secret"));
        assert!(a2.secure);
        mem.mark("fp_login", &a2);

        let a3 = next_act(&goal, &login_screen(true), &mem, false, limits);
        assert_eq!(a3.intent, PilotIntent::Dismiss);
        mem.mark("fp_login", &a3);

        let a4 = next_act(&goal, &login_screen(false), &mem, false, limits);
        assert_eq!(a4.intent, PilotIntent::Tap);
        assert_eq!(a4.id.as_deref(), Some("loginButton"));
    }

    #[test]
    fn secure_text_is_redacted_in_trace() {
        let goal = creds();
        let mut mem = PilotMemory::new();
        let a1 = next_act(
            &goal,
            &login_screen(false),
            &mem,
            false,
            PilotLimits::default(),
        );
        mem.mark("fp_login", &a1);
        let a2 = next_act(
            &goal,
            &login_screen(true),
            &mem,
            false,
            PilotLimits::default(),
        );
        assert_eq!(a2.trace()["text"], "***");
        assert_eq!(a1.trace()["text"], "alice");
    }

    #[test]
    fn goal_visible_stops_immediately() {
        let a = next_act(
            &creds(),
            &login_screen(false),
            &PilotMemory::new(),
            true,
            PilotLimits::default(),
        );
        assert!(a.is_terminal());
        assert_eq!(a.stop_code.as_deref(), Some("goal_visible"));
    }

    /// A sheet is usually the flow, not an obstacle: act inside it first.
    #[test]
    fn sheet_controls_are_used_before_dismissing_it() {
        let goal = PilotGoal {
            target_id: Some("ModalConfirmed".into()),
            ..Default::default()
        };
        let feel = feel_with(
            vec![
                item(
                    1,
                    AffordanceKind::Button,
                    "ConfirmAction",
                    Some("ConfirmAction"),
                ),
                item(
                    2,
                    AffordanceKind::Button,
                    "CancelSheet",
                    Some("CancelSheet"),
                ),
            ],
            false,
            Some("sheet"),
        );
        let a = next_act(
            &goal,
            &feel,
            &PilotMemory::new(),
            false,
            PilotLimits::default(),
        );
        assert_eq!(a.intent, PilotIntent::Tap);
        assert_eq!(a.id.as_deref(), Some("ConfirmAction"));
    }

    /// An overlay with nothing left to try is escaped, once.
    #[test]
    fn opaque_overlay_is_dismissed_after_controls_run_out() {
        let goal = PilotGoal {
            target_id: Some("homeTitle".into()),
            ..Default::default()
        };
        let feel = feel_with(vec![], false, Some("alert"));
        let mut mem = PilotMemory::new();
        let a = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert_eq!(a.intent, PilotIntent::Dismiss);
        mem.mark("fp_login", &a);

        let again = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert_ne!(again.intent, PilotIntent::Dismiss);
    }

    #[test]
    fn never_taps_destructive_controls() {
        let feel = feel_with(
            vec![
                item(1, AffordanceKind::PrimaryButton, "Delete account", None),
                item(2, AffordanceKind::Button, "Continue", Some("go")),
            ],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("homeTitle".into()),
            ..Default::default()
        };
        let a = next_act(
            &goal,
            &feel,
            &PilotMemory::new(),
            false,
            PilotLimits::default(),
        );
        assert_eq!(a.intent, PilotIntent::Tap);
        assert_eq!(a.id.as_deref(), Some("go"));
    }

    /// Real iOS reports SwiftUI SecureField as a plain AXTextField. Form order must
    /// still fill both fields, or the app rejects the submit.
    #[test]
    fn fills_both_fields_when_secure_field_is_misclassified() {
        let goal = creds();
        let mut mem = PilotMemory::new();
        let flat = feel_with(
            vec![
                item(1, AffordanceKind::TextField, "", Some("usernameTextField")),
                item(
                    2,
                    AffordanceKind::PrimaryButton,
                    "Login",
                    Some("loginButton"),
                ),
                item(
                    3,
                    AffordanceKind::TextField,
                    "",
                    Some("passwordSecureField"),
                ),
            ],
            false,
            None,
        );

        let a1 = next_act(&goal, &flat, &mem, false, PilotLimits::default());
        assert_eq!(a1.id.as_deref(), Some("usernameTextField"));
        assert_eq!(a1.text.as_deref(), Some("alice"));
        mem.mark("fp_login", &a1);

        let a2 = next_act(&goal, &flat, &mem, false, PilotLimits::default());
        assert_eq!(a2.intent, PilotIntent::Type);
        assert_eq!(a2.id.as_deref(), Some("passwordSecureField"));
        assert_eq!(a2.text.as_deref(), Some("secret"));
        mem.mark("fp_login", &a2);

        let a3 = next_act(&goal, &flat, &mem, false, PilotLimits::default());
        assert_eq!(a3.intent, PilotIntent::Tap);
        assert_eq!(a3.id.as_deref(), Some("loginButton"));
    }

    /// No param of a field's class means the field is left untouched.
    #[test]
    fn search_field_is_not_filled_with_noise() {
        let feel = feel_with(
            vec![
                item(1, AffordanceKind::SearchField, "Search", Some("search")),
                item(2, AffordanceKind::Button, "Browse", Some("browse")),
            ],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("homeTitle".into()),
            target_label: None,
            params: vec![PilotParam {
                value: "secret".into(),
                secure: true,
            }],
            ..Default::default()
        };
        let a = next_act(
            &goal,
            &feel,
            &PilotMemory::new(),
            false,
            PilotLimits::default(),
        );
        assert_eq!(a.intent, PilotIntent::Tap);
        assert_eq!(a.id.as_deref(), Some("browse"));
    }

    #[test]
    fn failed_type_does_not_consume_slot_or_field() {
        let feel = login_screen(false);
        let goal = creds();
        let mut mem = PilotMemory::new();
        let first = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert_eq!(first.intent, PilotIntent::Type);
        assert_eq!(first.motor_strategy, Some(MotorTypeStrategy::FocusHid));
        mem.mark_outcome(
            "fp_login",
            &first,
            ActionOutcome::NotDelivered,
            "fp_login",
        );
        assert_eq!(mem.plain_params_used, 0);
        assert!(mem.filled.is_empty());
        let retry = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.text, first.text);
        assert_eq!(retry.motor_strategy, Some(MotorTypeStrategy::TapThenHid));
        assert_ne!(retry.key, first.key);
        assert_ne!(
            first.action_spec(EpochStamp::default(), 1).operation,
            retry.action_spec(EpochStamp::default(), 2).operation
        );
    }

    #[test]
    fn failed_type_does_not_replay_identical_action_spec() {
        let feel = login_screen(false);
        let goal = creds();
        let mut mem = PilotMemory::new();
        let mut specs = Vec::new();
        for _ in 0..MotorTypeStrategy::ALL.len() {
            let action = next_act(&goal, &feel, &mem, false, PilotLimits::default());
            assert_eq!(action.intent, PilotIntent::Type);
            let spec = action.action_spec(EpochStamp::default(), mem.steps + 1);
            assert!(
                !specs.iter().any(|prev: &ActionSpec| {
                    prev.operation == spec.operation && prev.target == spec.target
                }),
                "replayed identical type ActionSpec: {:?}",
                spec.operation
            );
            specs.push(spec);
            mem.mark_outcome(
                "fp_login",
                &action,
                ActionOutcome::DeliveredNoEffect,
                "fp_login",
            );
        }
    }

    #[test]
    fn recovery_escalation_is_monotonic_for_failed_type() {
        let mut last = 0u8;
        for failures in 1..=6 {
            let stage = recovery_stage(
                ActionOutcome::DeliveredNoEffect,
                PilotIntent::Type,
                false,
                failures,
            );
            assert!(
                stage.rank() >= last,
                "recovery went backwards at failures={failures}: {} < {last}",
                stage.rank()
            );
            last = stage.rank();
        }
        assert_eq!(
            recovery_stage(ActionOutcome::DeliveredNoEffect, PilotIntent::Type, false, 1),
            RecoveryStage::AlternateMotor
        );
        assert_eq!(
            recovery_stage(ActionOutcome::DeliveredNoEffect, PilotIntent::Type, false, 4),
            RecoveryStage::DismissBlockingScope
        );
        assert_eq!(
            recovery_stage(ActionOutcome::DeliveredNoEffect, PilotIntent::Type, false, 5),
            RecoveryStage::Relaunch
        );
    }

    #[test]
    fn recovery_budget_guarantees_bounded_termination() {
        let feel = login_screen(false);
        let goal = creds();
        let limits = PilotLimits {
            max_recoveries: 3,
            ..Default::default()
        };
        let mut mem = PilotMemory::new();
        for _ in 0..3 {
            let action = next_act(&goal, &feel, &mem, false, limits);
            mem.mark_outcome(
                "fp_login",
                &action,
                ActionOutcome::NotDelivered,
                "fp_login",
            );
        }
        let terminal = next_act(&goal, &feel, &mem, false, limits);
        assert!(terminal.is_terminal());
        assert_eq!(terminal.stop_code.as_deref(), Some("recovery_exhausted"));
    }

    #[test]
    fn destructive_action_requires_goal_authorization() {
        let feel = feel_with(
            vec![
                item(1, AffordanceKind::Button, "Delete account", Some("delete")),
                item(2, AffordanceKind::Button, "Continue", Some("continue")),
            ],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("done".into()),
            ..Default::default()
        };
        let action = next_act(
            &goal,
            &feel,
            &PilotMemory::new(),
            false,
            PilotLimits::default(),
        );
        assert_eq!(action.id.as_deref(), Some("continue"));
    }

    #[test]
    fn exhausts_taps_then_scrolls_then_stops() {
        let feel = feel_with(
            vec![item(1, AffordanceKind::Button, "Only", Some("only"))],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("homeTitle".into()),
            ..Default::default()
        };
        let limits = PilotLimits {
            max_scrolls_per_screen: 2,
            max_backs: 0,
            ..Default::default()
        };
        let mut mem = PilotMemory::new();

        let a1 = next_act(&goal, &feel, &mem, false, limits);
        assert_eq!(a1.intent, PilotIntent::Tap);
        mem.mark("fp_login", &a1);

        for _ in 0..2 {
            let s = next_act(&goal, &feel, &mem, false, limits);
            assert_eq!(s.intent, PilotIntent::Scroll);
            mem.mark("fp_login", &s);
        }

        let last = next_act(&goal, &feel, &mem, false, limits);
        assert!(last.is_terminal());
        assert_eq!(last.stop_code.as_deref(), Some("exhausted"));
    }

    #[test]
    fn still_taps_list_cells_before_any_navigation() {
        let feel = feel_with(
            vec![item(1, AffordanceKind::Cell, "Post", Some("post_1"))],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("PostDetail".into()),
            ..Default::default()
        };
        let a = next_act(
            &goal,
            &feel,
            &PilotMemory::new(),
            false,
            PilotLimits::default(),
        );
        assert_eq!(a.intent, PilotIntent::Tap);
        assert_eq!(a.id.as_deref(), Some("post_1"));
    }

    #[test]
    fn still_taps_primary_cta_when_goal_is_absent() {
        let feel = feel_with(
            vec![item(
                1,
                AffordanceKind::PrimaryButton,
                "SIGN IN",
                Some("login_button"),
            )],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("tab_home".into()),
            ..Default::default()
        };
        let a = next_act(
            &goal,
            &feel,
            &PilotMemory::new(),
            false,
            PilotLimits::default(),
        );
        assert_eq!(a.intent, PilotIntent::Tap);
        assert_eq!(a.id.as_deref(), Some("login_button"));
    }

    #[test]
    fn stops_when_acceptance_absent_after_verified_navigation() {
        let feel = feel_with(
            vec![item(1, AffordanceKind::Button, "Love", Some("card_1"))],
            false,
            None,
        );
        let goal = PilotGoal {
            target_id: Some("tab_home".into()),
            ..Default::default()
        };
        let mut mem = PilotMemory::new();
        mem.transitions.push(PilotTransition {
            state: "fp_login".into(),
            action_key: "tap|login".into(),
            outcome: ActionOutcome::DeliveredAndVerified,
            next_state: "fp_home".into(),
        });
        let a = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert!(a.is_terminal());
        assert_eq!(a.stop_code.as_deref(), Some("acceptance_not_in_ax"));
    }

    #[test]
    fn does_not_stop_when_acceptance_identity_is_in_world() {
        let mut feel = feel_with(
            vec![item(1, AffordanceKind::Button, "Love", Some("card_1"))],
            false,
            None,
        );
        feel.world.elements.push(WorldElement {
            stable_key: "id:tab_home".into(),
            ax_path: "/0".into(),
            kind: AffordanceKind::Button,
            identifier: Some("tab_home".into()),
            label: Some("Home".into()),
            role: Some("AXTabButton".into()),
            frame_bucket: None,
            value_hash: None,
            enabled: true,
            focused: false,
            editable: false,
            on_screen: true,
            overlay_scope: None,
            tab_chrome: true,
        });
        feel.world.has_tab_bar = true;
        let goal = PilotGoal {
            target_id: Some("tab_home".into()),
            ..Default::default()
        };
        let mut mem = PilotMemory::new();
        mem.transitions.push(PilotTransition {
            state: "fp_login".into(),
            action_key: "tap|login".into(),
            outcome: ActionOutcome::DeliveredAndVerified,
            next_state: "fp_home".into(),
        });
        let a = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert!(!a.is_terminal());
    }

    #[test]
    fn does_not_stop_when_tab_prefix_aliases_to_tab_label() {
        let mut feel = feel_with(
            vec![item(1, AffordanceKind::Button, "Love", Some("card_1"))],
            false,
            None,
        );
        feel.world.elements.push(WorldElement {
            stable_key: "id:house.fill".into(),
            ax_path: "/0".into(),
            kind: AffordanceKind::Button,
            identifier: Some("house.fill".into()),
            label: Some("Home".into()),
            role: Some("AXRadioButton".into()),
            frame_bucket: None,
            value_hash: None,
            enabled: true,
            focused: false,
            editable: false,
            on_screen: true,
            overlay_scope: None,
            tab_chrome: true,
        });
        feel.world.has_tab_bar = true;
        let goal = PilotGoal {
            target_id: Some("tab_home".into()),
            ..Default::default()
        };
        let mut mem = PilotMemory::new();
        mem.transitions.push(PilotTransition {
            state: "fp_login".into(),
            action_key: "tap|login".into(),
            outcome: ActionOutcome::DeliveredAndVerified,
            next_state: "fp_home".into(),
        });
        let a = next_act(&goal, &feel, &mem, false, PilotLimits::default());
        assert!(!a.is_terminal(), "tab_home should alias to tab labeled Home");
    }

    #[test]
    fn diagnoses_inert_primary_button() {
        let history = vec![
            PilotStepRecord {
                step: 1,
                action_id: "a1".into(),
                epoch: Default::default(),
                intent: PilotIntent::Type,
                label: None,
                id: Some("userField".into()),
                kind: Some(AffordanceKind::TextField),
                fp_before: "fp_login".into(),
                fp_after: "fp_login".into(),
                fired: true,
                changed: false,
                outcome: Some(ActionOutcome::DeliveredAndVerified),
                goal_progress: false,
                candidate_keys: vec![],
                events: vec!["type:alice".into()],
                ms: 300,
            },
            PilotStepRecord {
                step: 2,
                action_id: "a2".into(),
                epoch: Default::default(),
                intent: PilotIntent::Tap,
                label: Some("Login".into()),
                id: Some("loginButton".into()),
                kind: Some(AffordanceKind::PrimaryButton),
                fp_before: "fp_login".into(),
                fp_after: "fp_login".into(),
                fired: true,
                changed: false,
                outcome: Some(ActionOutcome::DeliveredNoEffect),
                goal_progress: false,
                candidate_keys: vec![],
                events: vec![],
                ms: 250,
            },
        ];
        let d = diagnose(&creds(), &history, &login_screen(false));
        assert_eq!(d.code, "control_fired_no_transition");
        assert_eq!(d.control.as_deref(), Some("loginButton"));
        assert_eq!(d.fingerprint.as_deref(), Some("fp_login"));
    }

    #[test]
    fn diagnoses_missing_target_when_nothing_was_inert() {
        let history = vec![PilotStepRecord {
            step: 1,
            action_id: "a1".into(),
            epoch: Default::default(),
            intent: PilotIntent::Tap,
            label: Some("Browse".into()),
            id: None,
            kind: Some(AffordanceKind::Button),
            fp_before: "fp_a".into(),
            fp_after: "fp_b".into(),
            fired: true,
            changed: true,
            outcome: Some(ActionOutcome::DeliveredAndVerified),
            goal_progress: true,
            candidate_keys: vec![],
            events: vec!["nav".into()],
            ms: 200,
        }];
        let d = diagnose(&creds(), &history, &login_screen(false));
        assert_eq!(d.code, "target_never_visible");
    }
}
