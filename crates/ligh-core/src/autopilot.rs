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

use crate::feel::{FeelIR, FeelPhase, SalienceItem};
use crate::qa::AffordanceKind;
use crate::uxgraph::is_destructive_label;

pub const AUTOPILOT_SCHEMA_VERSION: u32 = 1;

/// Acceptance target plus typed data. Carries no path and no app-specific steps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PilotGoal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    /// Data the flow may need, bound to fields by kind (never by field name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<PilotParam>,
}

impl PilotGoal {
    pub fn target_name(&self) -> String {
        self.target_id
            .clone()
            .or_else(|| self.target_label.clone())
            .unwrap_or_else(|| "<unset>".into())
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<AffordanceKind>,
    pub reason: String,
    /// Dedupe handle, unique per (screen, target).
    pub key: String,
    /// Terminal reason when `intent == Stop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_code: Option<String>,
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

    /// Trace-safe view: secure text never leaves the host as plaintext.
    pub fn trace(&self) -> serde_json::Value {
        serde_json::json!({
            "intent": self.intent.as_str(),
            "label": self.label,
            "id": self.id,
            "kind": self.kind,
            "text": self.text.as_ref().map(|t| if self.secure { "***".to_string() } else { t.clone() }),
            "reason": self.reason,
            "stop_code": self.stop_code,
        })
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
}

impl PilotMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an executed act so the policy advances instead of looping.
    pub fn mark(&mut self, fp: &str, act: &PilotAct) {
        self.steps += 1;
        match act.intent {
            PilotIntent::Type => {
                self.filled.insert(act.key.clone());
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
}

impl Default for PilotLimits {
    fn default() -> Self {
        Self {
            max_scrolls_per_screen: 3,
            max_backs: 2,
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
        kind: None,
        reason,
        key,
        stop_code: None,
    }
}

fn stop(code: &str, reason: impl Into<String>) -> PilotAct {
    let mut a = act(PilotIntent::Stop, format!("stop|{code}"), reason.into());
    a.stop_code = Some(code.to_string());
    a
}

/// Next field to fill, paired with the next unused param of the matching class.
fn next_type(goal: &PilotGoal, feel: &FeelIR, mem: &PilotMemory) -> Option<PilotAct> {
    if goal.params.is_empty() {
        return None;
    }
    let fields: Vec<&SalienceItem> = feel.salience.iter().filter(|i| is_field(i.kind)).collect();
    if fields.is_empty() {
        return None;
    }

    let target = fields
        .iter()
        .find(|i| !mem.filled.contains(&field_key(i)))?;
    let secure = target.kind == AffordanceKind::SecureField;
    let skip = if secure {
        mem.secure_params_used
    } else {
        mem.plain_params_used
    };
    let param = match goal.params.iter().filter(|p| p.secure == secure).nth(skip) {
        Some(p) => p,
        // AX can report a secure field as a plain text field, so fall back to form
        // order. Never for a search field: that is where typing noise does damage.
        None if target.kind != AffordanceKind::SearchField => goal
            .params
            .get(mem.secure_params_used + mem.plain_params_used)?,
        None => return None,
    };

    let mut a = act(
        PilotIntent::Type,
        field_key(target),
        format!("fill {:?} (param {} of class)", target.kind, skip + 1),
    );
    a.label = target.label.clone();
    a.id = target.id.clone();
    a.text = Some(param.value.clone());
    a.secure = secure;
    a.kind = Some(target.kind);
    Some(a)
}

/// Highest-salience control not yet tried on this screen.
fn next_tap(feel: &FeelIR, mem: &PilotMemory, fp: &str) -> Option<PilotAct> {
    for item in &feel.salience {
        if !is_tappable(item.kind) {
            continue;
        }
        if item.label.is_none() && item.id.is_none() {
            continue;
        }
        if is_destructive_label(item.label.as_deref().unwrap_or("")) {
            continue;
        }
        let key = tap_key(fp, item);
        if mem.tried.contains(&key) {
            continue;
        }
        let mut a = act(
            PilotIntent::Tap,
            key,
            format!("salience rank {} ({:?})", item.rank, item.kind),
        );
        a.label = item.label.clone();
        a.id = item.id.clone();
        a.kind = Some(item.kind);
        return Some(a);
    }
    None
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

    let fp = feel.place.fingerprint.as_str();

    if let Some(a) = next_type(goal, feel, mem) {
        return a;
    }

    // Fields are done; a live keyboard can cover the controls that submit them.
    let keyboard_up = feel.feel.keyboard
        || feel
            .block
            .as_ref()
            .map(|b| b.kind == "keyboard")
            .unwrap_or(false);
    if keyboard_up {
        return act(
            PilotIntent::Dismiss,
            format!("{fp}|dismiss|keyboard"),
            "fields filled — clear keyboard to expose controls".into(),
        );
    }

    if let Some(a) = next_tap(feel, mem, fp) {
        return a;
    }

    // A sheet or alert is often the flow itself, so its own controls are tapped
    // above. Only escape the overlay once nothing inside it is left to try.
    if let Some(block) = &feel.block {
        if block.kind != "keyboard" {
            let key = format!("{fp}|dismiss|{}", block.kind);
            if !mem.tried.contains(&key) {
                return act(
                    PilotIntent::Dismiss,
                    key,
                    format!("{} exhausted — dismiss to get back to the flow", block.kind),
                );
            }
        }
    }

    if mem.scrolls.get(fp).copied().unwrap_or(0) < limits.max_scrolls_per_screen {
        return act(
            PilotIntent::Scroll,
            format!("{fp}|scroll"),
            "controls exhausted — reveal more of the screen".into(),
        );
    }

    let back = feel
        .salience
        .iter()
        .find(|s| s.kind == AffordanceKind::NavBack);
    if let Some(item) = back {
        if mem.backs < limits.max_backs {
            let mut a = act(
                PilotIntent::Back,
                format!("{fp}|back"),
                "screen exhausted — step back out".into(),
            );
            a.label = item.label.clone();
            a.id = item.id.clone();
            a.kind = Some(item.kind);
            return a;
        }
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
    use crate::feel::{FeelBlock, FeelDelta, FeelMeta, FeelPlace};

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
    fn diagnoses_inert_primary_button() {
        let history = vec![
            PilotStepRecord {
                step: 1,
                intent: PilotIntent::Type,
                label: None,
                id: Some("userField".into()),
                kind: Some(AffordanceKind::TextField),
                fp_before: "fp_login".into(),
                fp_after: "fp_login".into(),
                fired: true,
                changed: false,
                events: vec!["type:alice".into()],
                ms: 300,
            },
            PilotStepRecord {
                step: 2,
                intent: PilotIntent::Tap,
                label: Some("Login".into()),
                id: Some("loginButton".into()),
                kind: Some(AffordanceKind::PrimaryButton),
                fp_before: "fp_login".into(),
                fp_after: "fp_login".into(),
                fired: true,
                changed: false,
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
            intent: PilotIntent::Tap,
            label: Some("Browse".into()),
            id: None,
            kind: Some(AffordanceKind::Button),
            fp_before: "fp_a".into(),
            fp_after: "fp_b".into(),
            fired: true,
            changed: true,
            events: vec!["nav".into()],
            ms: 200,
        }];
        let d = diagnose(&creds(), &history, &login_screen(false));
        assert_eq!(d.code, "target_never_visible");
    }
}
