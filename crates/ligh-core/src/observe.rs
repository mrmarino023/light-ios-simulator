//! Structured observation snapshot — the agent-facing JSON contract.

use serde::{Deserialize, Serialize};
use serde_json::json;

/// Frame statistics from the GPU compositor / IOSurface stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameMeta {
    #[serde(rename = "w")]
    pub width: u32,
    #[serde(rename = "h")]
    pub height: u32,
    /// Monotonic frame / import id (imports_ok counter).
    pub id: u64,
    /// Approximate FPS since stream start.
    pub fps: f64,
    /// `true` when at least one IOSurface frame has been successfully imported.
    pub imports_ok: bool,
}

/// Screen-level summary for Consumer Agent Vision (observe v2).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SceneMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_title: Option<String>,
    /// Coarse surface: `springboard` | `settings` | `messages_composer` | `transition` | `app` | `unknown`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(default)]
    pub keyboard_visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyboard_frame: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<String>,
}

/// Sensation bus event (post-action / poll).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenseEvent {
    /// Unix time seconds (f64).
    pub t: f64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Full structured observation — returned by `ligh observe` and `lighd observe` RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveSnapshot {
    /// Contract version — bump only on breaking changes; additive fields keep same version.
    #[serde(default = "observe_schema_default")]
    pub schema_version: u32,
    pub udid: String,
    /// Transaction identity. Targets are valid only within this session/launch/screen tuple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub boot_epoch: u64,
    #[serde(default)]
    pub launch_epoch: u64,
    #[serde(default)]
    pub screen_epoch: u64,
    #[serde(default)]
    pub stability_streak: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_score: Option<f64>,
    /// Bundle the current transaction owns and the foreground identity proven from live AX.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_app_label: Option<String>,
    pub booted: bool,
    /// Simulator.app is NOT running when LIGH owns the host (the point).
    pub simulator_app_running: bool,
    pub frame: Option<FrameMeta>,
    /// Active app bundle id if known from session state.
    pub app_bundle_id: Option<String>,
    /// Accessibility tree from headless AXPTranslator (or empty/error).
    pub accessibility_tree: AccessibilityTree,
    /// Screen summary (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneMeta>,
    /// Default LLM view: hittable / interesting nodes (capped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actionable_topk: Vec<serde_json::Value>,
    /// Sensation events since previous observe (daemon) or empty (direct).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SenseEvent>,
    /// `ready` | `empty` | `stale` | `error` | `transition`
    #[serde(default = "default_ax_quality")]
    pub ax_quality: String,
    /// True when this snapshot passed settle (not mid-animation sparse AX).
    #[serde(default)]
    pub settled: bool,
    /// Server-side time to build this snapshot (hot path metric).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observe_ms: Option<f64>,
    /// How the client reached host state: `lighd` (hot) or `direct` (cold).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Control-plane session phase (`booting|ax_warming|ready|acting|settling|degraded|dead`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// True when agents must not plan from this snapshot (empty/transition/error/unsettle).
    #[serde(default)]
    pub eyes_unusable: bool,
    /// Top occlusion layer: `none|keyboard|alert|sheet|transition`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<String>,
    /// Compact screen signature for effect verification (Δ after act).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_sig: Option<String>,
}

fn observe_schema_default() -> u32 {
    OBSERVE_SCHEMA_VERSION
}

fn default_ax_quality() -> String {
    "empty".into()
}

/// Current observe JSON contract version (see `docs/OBSERVE.md`).
pub const OBSERVE_SCHEMA_VERSION: u32 = 2;

/// Default cap for `actionable_topk`.
pub const ACTIONABLE_TOPK: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum AccessibilityTree {
    /// Honest stub: not yet wired up.
    #[serde(rename = "not_implemented")]
    NotImplemented,
    /// No frontmost app / empty tree.
    #[serde(rename = "empty")]
    Empty,
    /// Live dump from AXPTranslator (headless).
    #[serde(rename = "available")]
    Available {
        /// Flat interactive elements (label/identifier + frame in device points).
        nodes: Vec<serde_json::Value>,
        /// Nested root when present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_count: Option<usize>,
        /// Logical point size used to normalize frames (device points).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        point_size: Option<(f64, f64)>,
    },
    /// AX bridge attempted but failed.
    #[serde(rename = "error")]
    Error { message: String },
}

impl AccessibilityTree {
    pub fn from_ax_dump(v: serde_json::Value) -> Self {
        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
        match status {
            "empty" => Self::Empty,
            "available" => {
                let nodes = v
                    .get("elements")
                    .and_then(|e| e.as_array())
                    .cloned()
                    .unwrap_or_default();
                let root = v.get("root").cloned().filter(|r| !r.is_null());
                let element_count = v
                    .get("element_count")
                    .and_then(|c| c.as_u64())
                    .map(|n| n as usize)
                    .or(Some(nodes.len()));
                let point_size = v.get("point_size").and_then(|ps| {
                    let w = ps.get("width")?.as_f64()?;
                    let h = ps.get("height")?.as_f64()?;
                    Some((w, h))
                });
                Self::Available {
                    nodes,
                    root,
                    element_count,
                    point_size,
                }
            }
            _ => Self::Error {
                message: format!("unexpected ax status: {status}"),
            },
        }
    }

    pub fn nodes(&self) -> &[serde_json::Value] {
        match self {
            Self::Available { nodes, .. } => nodes,
            _ => &[],
        }
    }

    pub fn point_size(&self) -> Option<(f64, f64)> {
        match self {
            Self::Available { point_size, .. } => *point_size,
            _ => None,
        }
    }

    /// Center of best element whose label or identifier contains `needle`.
    pub fn find_label(&self, needle: &str) -> Option<(f64, f64)> {
        match self {
            Self::Available {
                nodes, point_size, ..
            } => find_label_center(nodes, needle, *point_size),
            _ => None,
        }
    }

    /// Center of element with exact accessibility `identifier` (or opaque tree `id`).
    pub fn find_id(&self, id: &str) -> Option<(f64, f64)> {
        match self {
            Self::Available {
                nodes, point_size, ..
            } => find_id_center(nodes, id, *point_size),
            _ => None,
        }
    }

    pub fn ax_quality(&self) -> &'static str {
        match self {
            Self::Available { nodes, .. } if is_transition_sparse(nodes) => "transition",
            Self::Available { nodes, .. } if !nodes.is_empty() => "ready",
            Self::Available { .. } | Self::Empty => "empty",
            Self::Error { .. } => "error",
            Self::NotImplemented => "empty",
        }
    }
}

impl ObserveSnapshot {
    /// Fill v2 scene / actionable_topk / ax_quality from the AX tree.
    pub fn enrich_v2(&mut self) {
        self.schema_version = OBSERVE_SCHEMA_VERSION;
        let nodes = self.accessibility_tree.nodes();
        self.ax_quality = self.accessibility_tree.ax_quality().into();
        self.settled = self.ax_quality == "ready";
        self.actionable_topk = build_actionable_topk(nodes, ACTIONABLE_TOPK);
        self.scene = Some(build_scene(nodes, self.app_bundle_id.clone()));
        self.screen_sig = Some(crate::qa::screen_fingerprint(nodes));
        let has_udid = !self.udid.is_empty();
        crate::control::stamp_control_fields(self, has_udid);
    }

    /// Whether an agent should act on this snapshot (not empty/transition).
    pub fn is_actionable_eyes(&self) -> bool {
        self.ax_quality == "ready" && !self.actionable_topk.is_empty() && !self.eyes_unusable
    }
}

/// Status-bar / Spotlight / chrome that must not drive agent policy.
/// Label of the app that owns the AX tree, when it is not SpringBoard itself.
/// Decisive signal that a real app is foreground, whatever its content looks like.
pub fn foreground_app_label(nodes: &[serde_json::Value]) -> Option<String> {
    nodes.iter().find_map(|n| {
        let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if !role.contains("Application") {
            return None;
        }
        let label = n
            .get("label")
            .and_then(|v| v.as_str())
            .or_else(|| n.get("text").and_then(|v| v.as_str()))?;
        if label.is_empty() || label.eq_ignore_ascii_case("springboard") {
            return None;
        }
        Some(label.to_string())
    })
}

pub fn is_chrome_node(n: &serde_json::Value) -> bool {
    let id = n
        .get("identifier")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if id == "spotlight-pill" || id.contains("spotlight") {
        return true;
    }
    let lab = n
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let val = n
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Home Spotlight search affordance (IT/EN) — not Settings search.
    if (lab == "cerca" || lab == "search") && val.contains("pagina") {
        return true;
    }
    if lab.starts_with("carica batteria")
        || lab.starts_with("battery")
        || lab == "cellulare"
        || lab == "cellular"
        || lab == "wifi"
        || lab == "wi-fi"
    {
        // Status items are often tiny and top-of-screen; treat as chrome when short height.
        let h = n
            .get("frame")
            .and_then(|f| f.get("height"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let y = n
            .get("frame")
            .and_then(|f| f.get("y"))
            .and_then(|v| v.as_f64())
            .unwrap_or(999.0);
        if y < 60.0 || h < 28.0 {
            return true;
        }
    }
    // Clock-only status
    if lab.len() <= 5 && lab.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.') {
        let y = n
            .get("frame")
            .and_then(|f| f.get("y"))
            .and_then(|v| v.as_f64())
            .unwrap_or(999.0);
        if y < 50.0 {
            return true;
        }
    }
    false
}

/// Tab bar chrome is first-class: tab items are how real apps switch surfaces.
/// Status-bar / Spotlight remain chrome; tab bars do not.
pub fn is_tab_bar_node(n: &serde_json::Value) -> bool {
    let traits = n
        .get("traits")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if traits.contains("tabbar") {
        return true;
    }
    let role = n
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if role.contains("tabbar") || role.contains("tabbutton") || role.contains("tab bar") {
        return true;
    }
    let lab = n
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    lab == "tab bar" || lab.contains("tabbar")
}

fn normalize_tab_token(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// XCTest often names tab buttons `tab_home` while AXPTranslator hit-test
/// reports the SF Symbol (`house.fill`) plus the visible label (`Home`).
pub fn tab_chrome_alias_matches(needle: &str, label: Option<&str>, is_tab_chrome: bool) -> bool {
    if !is_tab_chrome {
        return false;
    }
    let Some(rest) = needle.strip_prefix("tab_") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    let Some(label) = label.filter(|s| !s.is_empty()) else {
        return false;
    };
    normalize_tab_token(label) == normalize_tab_token(rest)
}

/// True when a goal identity names this tab (e.g. `tab_notes`, `notes_title` → "Notes").
pub fn identity_suggests_tab_label(identity: &str, label: &str) -> bool {
    if tab_chrome_alias_matches(identity, Some(label), true) {
        return true;
    }
    let first = identity
        .strip_prefix("tab_")
        .unwrap_or(identity)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    let want = normalize_tab_token(first);
    let have = normalize_tab_token(label);
    !want.is_empty() && want == have
}

/// Exact accessibility identifier, opaque tree id, or tab-chrome `tab_*` alias.
pub fn node_matches_identifier(n: &serde_json::Value, needle: &str) -> bool {
    if n.get("identifier").and_then(|v| v.as_str()) == Some(needle)
        || n.get("id").and_then(|v| v.as_str()) == Some(needle)
    {
        return true;
    }
    tab_chrome_alias_matches(
        needle,
        n.get("label").and_then(|v| v.as_str()),
        is_tab_bar_node(n),
    )
}

/// Mid-navigation AX: only status chrome or almost nothing.
pub fn is_transition_sparse(nodes: &[serde_json::Value]) -> bool {
    if nodes.is_empty() {
        return false; // empty is empty, not transition
    }
    let useful: Vec<_> = nodes.iter().filter(|n| !is_chrome_node(n)).collect();
    if useful.len() >= 4 {
        return false;
    }
    // 1–3 non-chrome nodes that are only status-like → transition
    useful.iter().all(|n| {
        let lab = n.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let role = n
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        lab.is_empty()
            || role.contains("static")
            || is_chrome_node(n)
            || lab.chars().all(|c| c.is_ascii_digit() || c == ':' || c == ' ')
    }) || useful.len() < 3 && nodes.iter().filter(|n| !is_chrome_node(n)).count() < 3
}

/// Detect high-level surface for agent policy (not bundle_id — AX labels).
pub fn detect_surface(nodes: &[serde_json::Value]) -> &'static str {
    if nodes.is_empty() {
        return "transition";
    }
    let labs: Vec<String> = nodes
        .iter()
        .filter(|n| !is_chrome_node(n))
        .filter_map(|n| n.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let lower: Vec<String> = labs.iter().map(|s| s.to_ascii_lowercase()).collect();
    let has = |xs: &[&str]| xs.iter().any(|x| lower.iter().any(|l| l == x || l.contains(x)));

    if has(&["a:", "to:"]) && has(&["messaggio", "message"]) {
        return "messages_composer";
    }
    if has(&["nuovo messaggio", "new message"]) {
        return "messages_composer";
    }
    // Inside Settings: search field or settings rows — NOT SpringBoard Impostazioni icon alone.
    if has(&["generali", "general", "bluetooth"])
        || (has(&["cerca", "search"])
            && nodes.iter().any(|n| {
                let id = n.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
                is_editable_role(n.get("role").and_then(|v| v.as_str()).unwrap_or(""))
                    && id != "spotlight-pill"
                    && !is_chrome_node(n)
            }))
    {
        return "settings";
    }

    let is_home_icon = |l: &str| {
        matches!(
            l,
            "messaggi"
                | "messages"
                | "impostazioni"
                | "settings"
                | "safari"
                | "foto"
                | "photos"
                | "mappe"
                | "maps"
                | "calendario"
                | "calendar"
                | "wallet"
                | "salute"
                | "health"
                | "news"
                | "fitness"
                | "watch"
                | "contatti"
                | "contacts"
                | "file"
                | "files"
                | "cartella utility"
                | "utilities"
                | "utility"
                | "fotocamera"
                | "camera"
                | "orologio"
                | "clock"
                | "app store"
                | "musica"
                | "music"
                | "mail"
                | "telefono"
                | "phone"
        )
    };
    let app_icons = lower.iter().filter(|l| is_home_icon(l)).count();
    if app_icons >= 3 {
        return "springboard";
    }
    // Strong pair signal on slim simulators (Fitness + Watch, no spotlight pill).
    let has = |name: &str| lower.iter().any(|l| l == name);
    if has("fitness") && has("watch") {
        return "springboard";
    }
    if has("safari") && (has("messaggi") || has("messages")) && app_icons >= 2 {
        return "springboard";
    }
    // Home grid: many hittable icon-sized buttons in the upper screen. Weak signal —
    // a list-of-buttons app looks identical, so require that no foreground app owns
    // the tree and that the buttons carry no accessibility identifier (home icons
    // never do, app content usually does).
    if foreground_app_label(nodes).is_none() {
        let grid_icons = nodes
            .iter()
            .filter(|n| {
                let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let has_ident = n
                    .get("identifier")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                role.contains("Button")
                    && !has_ident
                    && n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(false)
                    && n.get("frame")
                        .and_then(|f| f.get("y"))
                        .and_then(|y| y.as_f64())
                        .map(|y| y < 650.0)
                        .unwrap_or(false)
            })
            .count();
        if grid_icons >= 6 {
            return "springboard";
        }
    }

    if is_transition_sparse(nodes) {
        return "transition";
    }
    "app"
}

/// Snapshot is settled enough for the agent to act.
pub fn eyes_ready(ax_quality: &str, actionable_len: usize) -> bool {
    ax_quality == "ready" && actionable_len > 0
}

/// Build sensation events by comparing consecutive node fingerprints.
pub fn diff_sense_events(
    prev: Option<&[serde_json::Value]>,
    curr: &[serde_json::Value],
    now: f64,
) -> Vec<SenseEvent> {
    let mut out = Vec::new();
    if curr.is_empty() {
        if prev.map(|p| !p.is_empty()).unwrap_or(false) {
            out.push(SenseEvent {
                t: now,
                kind: "ax_empty".into(),
                payload: None,
            });
        }
        return out;
    }
    let Some(prev) = prev else {
        return out;
    };

    let prev_focus = focused_id(prev);
    let curr_focus = focused_id(curr);
    if prev_focus != curr_focus {
        out.push(SenseEvent {
            t: now,
            kind: "focus_changed".into(),
            payload: Some(serde_json::json!({ "from": prev_focus, "to": curr_focus })),
        });
    }

    let prev_vals = value_map(prev);
    let curr_vals = value_map(curr);
    for (id, (label, val)) in &curr_vals {
        match prev_vals.get(id) {
            Some((_, old)) if old != val => {
                out.push(SenseEvent {
                    t: now,
                    kind: "value_changed".into(),
                    payload: Some(serde_json::json!({
                        "id": id,
                        "label": label,
                        "from": old,
                        "to": val,
                    })),
                });
            }
            None if !val.is_empty() => {
                out.push(SenseEvent {
                    t: now,
                    kind: "value_changed".into(),
                    payload: Some(serde_json::json!({
                        "id": id,
                        "label": label,
                        "from": "",
                        "to": val,
                    })),
                });
            }
            _ => {}
        }
    }

    let prev_kb = keyboard_visible(prev);
    let curr_kb = keyboard_visible(curr);
    if !prev_kb && curr_kb {
        out.push(SenseEvent {
            t: now,
            kind: "keyboard_shown".into(),
            payload: None,
        });
    }

    let prev_alerts = alert_labels(prev);
    let curr_alerts = alert_labels(curr);
    for a in &curr_alerts {
        if !prev_alerts.iter().any(|p| p == a) {
            out.push(SenseEvent {
                t: now,
                kind: "alert_appeared".into(),
                payload: Some(serde_json::json!({ "label": a })),
            });
        }
    }

    let prev_title = screen_title(prev);
    let curr_title = screen_title(curr);
    if prev_title != curr_title && curr_title.is_some() {
        out.push(SenseEvent {
            t: now,
            kind: "navigated".into(),
            payload: Some(serde_json::json!({ "from": prev_title, "to": curr_title })),
        });
    }

    out
}

fn focused_id(nodes: &[serde_json::Value]) -> Option<String> {
    nodes.iter().find_map(|n| {
        if n.get("focused").and_then(|v| v.as_bool()).unwrap_or(false) {
            n.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn value_map(nodes: &[serde_json::Value]) -> std::collections::HashMap<String, (String, String)> {
    let mut m = std::collections::HashMap::new();
    for n in nodes {
        let id = n
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if !is_editable_role(role) {
            continue;
        }
        let label = n
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let val = n
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        m.insert(id, (label, val));
    }
    m
}

fn keyboard_visible(nodes: &[serde_json::Value]) -> bool {
    nodes.iter().any(|n| {
        let role = n
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        role.contains("keyboard")
            || n.get("traits")
                .and_then(|v| v.as_str())
                .map(|t| t.contains("keyboard"))
                .unwrap_or(false)
    })
}

fn alert_labels(nodes: &[serde_json::Value]) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| {
            let role = n
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            role.contains("alert") || role.contains("sheet") || role.contains("dialog")
        })
        .filter_map(|n| {
            n.get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        })
        .collect()
}

fn screen_title(nodes: &[serde_json::Value]) -> Option<String> {
    let mut best: Option<(f64, String)> = None;
    for n in nodes {
        let role = n
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let lab = n
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if lab.is_empty() || lab.len() > 48 {
            continue;
        }
        let y = n
            .get("frame")
            .and_then(|f| f.get("y"))
            .and_then(|v| v.as_f64())
            .unwrap_or(9999.0);
        let score = if role.contains("heading") {
            y - 1000.0
        } else if role.contains("static") && y < 120.0 {
            y
        } else {
            continue;
        };
        match &best {
            None => best = Some((score, lab.to_string())),
            Some((s, _)) if score < *s => best = Some((score, lab.to_string())),
            _ => {}
        }
    }
    best.map(|(_, t)| t)
}

pub fn build_scene(nodes: &[serde_json::Value], bundle_id: Option<String>) -> SceneMeta {
    let kb = keyboard_visible(nodes);
    let kb_frame = nodes.iter().find_map(|n| {
        let role = n
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if role.contains("keyboard") {
            n.get("frame").cloned()
        } else {
            None
        }
    });
    SceneMeta {
        bundle_id,
        screen_title: screen_title(nodes),
        surface: Some(detect_surface(nodes).into()),
        keyboard_visible: kb,
        keyboard_frame: kb_frame,
        alerts: alert_labels(nodes),
        sheets: nodes
            .iter()
            .filter(|n| {
                n.get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains("sheet")
            })
            .filter_map(|n| n.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect(),
    }
}

fn actionable_score(n: &serde_json::Value) -> i32 {
    if is_chrome_node(n) && !is_tab_bar_node(n) {
        return -1000;
    }
    let role = n
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let hittable = n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true);
    let enabled = n.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let focused = n.get("focused").and_then(|v| v.as_bool()).unwrap_or(false);
    let label = n.get("label").and_then(|v| v.as_str()).unwrap_or("");
    let has_label = !label.is_empty();
    let has_id = n
        .get("identifier")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        || n.get("id")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
    if !hittable || !enabled {
        return -100;
    }
    // Mega VoiceOver soup / full-screen groups — poison for the planner.
    if label_is_noisy(label) {
        return -1000;
    }
    if role.contains("group") && frame_is_near_fullscreen(n) && !is_tab_bar_node(n) {
        return -1000;
    }
    let mut s = 0;
    if focused {
        s += 50;
    }
    if is_tab_bar_node(n) {
        s += 35;
        return s;
    }
    if is_editable_role(&role) {
        s += 40;
    } else if role.contains("button") || role.contains("cell") || role.contains("link") {
        s += 30;
    } else if role.contains("switch") || role.contains("slider") {
        s += 25;
    } else if has_label || has_id {
        s += 10;
    } else {
        return -50;
    }
    if role.contains("application") || role.contains("window") {
        s -= 80;
    }
    // Prefer short, precise labels.
    if label.len() > 40 {
        s -= ((label.len() - 40) / 15) as i32;
    }
    s
}

/// Labels that dump the whole screen into one node (RN accessibility merge).
pub fn label_is_noisy(label: &str) -> bool {
    if label.len() > 96 {
        return true;
    }
    // Concatenated tab bar into one string, or many list titles jammed together.
    let tab_marks = label.matches(", tab,").count();
    if tab_marks >= 2 {
        return true;
    }
    if label.matches(" Tab").count() >= 3 {
        return true;
    }
    false
}

fn frame_is_near_fullscreen(n: &serde_json::Value) -> bool {
    let Some(f) = n.get("frame") else {
        return false;
    };
    let w = f.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let h = f.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
    w >= 300.0 && h >= 500.0
}

fn shorten_label(label: &str) -> String {
    let t = label.trim();
    if t.len() <= 64 {
        return t.to_string();
    }
    format!("{}…", t.chars().take(61).collect::<String>())
}

/// Filter + rank interactive nodes for the LLM default view.
pub fn build_actionable_topk(nodes: &[serde_json::Value], k: usize) -> Vec<serde_json::Value> {
    let mut scored: Vec<(i32, &serde_json::Value)> = nodes
        .iter()
        .map(|n| (actionable_score(n), n))
        .filter(|(s, _)| *s >= 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(k)
        .map(|(_, n)| {
            let mut slim = serde_json::Map::new();
            for key in [
                "id",
                "role",
                "traits",
                "text",
                "label",
                "value",
                "placeholder",
                "focused",
                "selected",
                "enabled",
                "hittable",
                "visible",
                "frame",
                "center_norm",
                "parent_id",
                "identifier",
            ] {
                if let Some(v) = n.get(key) {
                    if key == "label" {
                        if let Some(s) = v.as_str() {
                            slim.insert(key.to_string(), json!(shorten_label(s)));
                            continue;
                        }
                    }
                    slim.insert(key.to_string(), v.clone());
                }
            }
            serde_json::Value::Object(slim)
        })
        .collect()
}

fn name_similarity(wanted: &str, candidate: &str) -> f32 {
    if wanted.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    let w = wanted.to_ascii_lowercase();
    let c = candidate.to_ascii_lowercase();
    if w == c {
        return 1.0;
    }
    if c.contains(&w) || w.contains(&c) {
        return 0.82;
    }
    let common = w.chars().zip(c.chars()).take_while(|(a, b)| a == b).count();
    if common >= 3 {
        return 0.45 + (common as f32 * 0.05);
    }
    0.0
}

/// Rank AX nodes similar to a wanted id/label — agent fault evidence (no screenshots).
pub fn rank_candidates(
    nodes: &[serde_json::Value],
    wanted_id: Option<&str>,
    wanted_label: Option<&str>,
    k: usize,
) -> Vec<serde_json::Value> {
    let mut scored: Vec<(f32, &serde_json::Value)> = nodes
        .iter()
        .filter_map(|n| {
            let ident = n
                .get("identifier")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let label = n.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let mut score = 0.0f32;
            if let Some(w) = wanted_id {
                score = score.max(name_similarity(w, ident));
            }
            if let Some(w) = wanted_label {
                score = score.max(name_similarity(w, label) * 0.95);
            }
            if score < 0.35 {
                return None;
            }
            if role.to_ascii_lowercase().contains("button")
                || role.to_ascii_lowercase().contains("textfield")
                || role.to_ascii_lowercase().contains("secure")
            {
                score += 0.05;
            }
            Some((score, n))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(k)
        .map(|(score, n)| {
            json!({
                "score": (score * 100.0).round() / 100.0,
                "id": n.get("identifier").or_else(|| n.get("id")),
                "label": n.get("label").or_else(|| n.get("text")),
                "role": n.get("role"),
                "focused": n.get("focused"),
                "hittable": n.get("hittable"),
            })
        })
        .collect()
}

/// Center of element with exact accessibility `identifier` (preferred) or opaque tree `id`.
/// When several nodes share the identifier, prefer an on-screen editable field over a
/// labeled container that inherited the same id.
pub fn find_id_center(
    nodes: &[serde_json::Value],
    id: &str,
    point_size: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    let mut hits: Vec<&serde_json::Value> = nodes
        .iter()
        .filter(|n| node_matches_identifier(n, id))
        .collect();
    if hits.is_empty() {
        return None;
    }
    hits.sort_by(|a, b| typeable_rank(a).cmp(&typeable_rank(b)));
    node_center(hits[0], point_size)
}

fn typeable_rank(n: &serde_json::Value) -> (u8, u8, u8) {
    let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let editable = is_editable_role(role)
        || n.get("traits")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.contains("editable"));
    let focused = n.get("focused").and_then(|v| v.as_bool()).unwrap_or(false);
    let on_screen = n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true)
        && n.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
    (
        u8::from(!editable),
        u8::from(!focused),
        u8::from(!on_screen),
    )
}

pub fn find_id_in_dump(dump: &serde_json::Value, id: &str) -> Option<(f64, f64)> {
    let nodes = dump
        .get("elements")
        .and_then(|e| e.as_array())
        .or_else(|| dump.get("nodes").and_then(|e| e.as_array()))?;
    let point_size = dump_point_size(dump);
    find_id_center(nodes, id, point_size)
}

fn dump_point_size(dump: &serde_json::Value) -> Option<(f64, f64)> {
    dump.get("point_size").and_then(|ps| {
        if let Some(arr) = ps.as_array() {
            Some((arr.first()?.as_f64()?, arr.get(1)?.as_f64()?))
        } else {
            Some((ps.get("width")?.as_f64()?, ps.get("height")?.as_f64()?))
        }
    })
}

/// True when node is enabled, marked hittable/visible, and centered in the viewport band.
pub fn node_viewport_hittable(n: &serde_json::Value) -> bool {
    let hittable = n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true);
    let enabled = n.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let visible = n.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
    if !hittable || !enabled || !visible {
        return false;
    }
    if let Some(cn) = n.get("center_norm") {
        if let Some(y) = cn.get("y").and_then(|v| v.as_f64()) {
            return (0.06..=0.94).contains(&y);
        }
    }
    true
}

/// Like [`find_id_in_dump`] but requires the element to be viewport-hittable (scroll target).
pub fn find_hittable_id_in_dump(dump: &serde_json::Value, id: &str) -> Option<(f64, f64)> {
    let nodes = dump
        .get("elements")
        .and_then(|e| e.as_array())
        .or_else(|| dump.get("nodes").and_then(|e| e.as_array()))?;
    let point_size = dump_point_size(dump);
    let el = nodes.iter().find(|n| {
        node_matches_identifier(n, id) && node_viewport_hittable(n)
    })?;
    node_center(el, point_size)
}

pub fn find_onscreen_id_in_dump(dump: &serde_json::Value, id: &str) -> Option<(f64, f64)> {
    if let Some(hit) = find_hittable_id_in_dump(dump, id) {
        return Some(hit);
    }
    let nodes = dump
        .get("elements")
        .and_then(|e| e.as_array())
        .or_else(|| dump.get("nodes").and_then(|e| e.as_array()))?;
    let point_size = dump_point_size(dump);
    let el = nodes.iter().find(|n| {
        node_matches_identifier(n, id)
            && n
                .get("center_norm")
                .and_then(|c| c.get("y").and_then(|v| v.as_f64()))
                .is_some_and(|y| (0.04..=0.96).contains(&y))
    })?;
    node_center(el, point_size)
}

pub fn find_hittable_label_in_dump(dump: &serde_json::Value, label: &str) -> Option<(f64, f64)> {
    let needle = label.to_ascii_lowercase();
    let nodes = dump
        .get("elements")
        .and_then(|e| e.as_array())
        .or_else(|| dump.get("nodes").and_then(|e| e.as_array()))?;
    let point_size = dump_point_size(dump);
    let el = nodes.iter().find(|n| {
        node_viewport_hittable(n) && node_matches_label(n, &needle)
    })?;
    node_center(el, point_size)
}

pub fn is_editable_role(role: &str) -> bool {
    let r = role.to_ascii_lowercase();
    r.contains("searchfield") || r.contains("textfield") || r.contains("textarea")
}

fn role_rank(role: &str, prefer_editable: bool) -> u8 {
    let r = role.to_ascii_lowercase();
    if is_editable_role(&r) {
        if prefer_editable {
            0
        } else {
            3
        }
    } else if r.contains("button")
        || r.contains("cell")
        || r.contains("switch")
        || r.contains("link")
    {
        1
    } else if r.contains("slider") {
        2
    } else if r.contains("application") || r.contains("window") {
        5
    } else if r.contains("static") || r.contains("image") || r.contains("heading") {
        4
    } else {
        3
    }
}

fn label_text(n: &serde_json::Value) -> String {
    n.get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_back_chrome_label(lab: &str) -> bool {
    let t = lab.trim();
    t.starts_with('<')
        || t.starts_with('‹')
        || t.starts_with('←')
        || t.starts_with("back ")
}

fn node_matches_label(n: &serde_json::Value, needle: &str) -> bool {
    let lab = label_text(n);
    if !lab.is_empty() {
        if is_back_chrome_label(&lab) {
            return false;
        }
        if lab.contains(needle) {
            return true;
        }
    }
    n.get("identifier")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase().contains(needle))
        .unwrap_or(false)
        || n.get("value")
            .and_then(|v| v.as_str())
            .map(|s| {
                let v = s.to_ascii_lowercase();
                !is_back_chrome_label(&v) && v.contains(needle)
            })
            .unwrap_or(false)
}

fn label_exactness(n: &serde_json::Value, needle: &str) -> u8 {
    let lab = label_text(n);
    if lab == needle {
        0
    } else if lab.starts_with(needle) || lab.ends_with(needle) {
        1
    } else {
        2
    }
}

fn node_area(el: &serde_json::Value) -> f64 {
    let frame = match el.get("frame") {
        Some(f) => f,
        None => return 0.0,
    };
    let w = frame.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let h = frame.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
    w * h
}

fn node_center(el: &serde_json::Value, point_size: Option<(f64, f64)>) -> Option<(f64, f64)> {
    if let Some(cn) = el.get("center_norm") {
        let x = cn.get("x").and_then(|v| v.as_f64())?;
        let y = cn.get("y").and_then(|v| v.as_f64())?;
        return Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
    }
    let frame = el.get("frame")?;
    let x = frame.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = frame.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let w = frame.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let h = frame.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let (pw, ph) = point_size.unwrap_or((393.0, 852.0));
    if pw <= 0.0 || ph <= 0.0 {
        return None;
    }
    if w >= pw * 0.9 && h >= ph * 0.5 {
        return None;
    }
    Some((
        ((x + w * 0.5) / pw).clamp(0.0, 1.0),
        ((y + h * 0.5) / ph).clamp(0.0, 1.0),
    ))
}

fn is_settings_search_row(n: &serde_json::Value) -> bool {
    let id = n
        .get("identifier")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    id.contains("settings.search") || id == "spotlight-pill"
}

pub fn find_label_center(
    nodes: &[serde_json::Value],
    label: &str,
    point_size: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    let needle = label.to_ascii_lowercase();
    let search_query = needle == "cerca" || needle == "search";
    let mut hits: Vec<&serde_json::Value> = nodes
        .iter()
        .filter(|n| node_matches_label(n, &needle))
        .filter(|n| !(search_query && is_settings_search_row(n)))
        .collect();
    if search_query {
        for n in nodes {
            let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
            if !is_editable_role(role) || is_settings_search_row(n) {
                continue;
            }
            let lab = n.get("label").and_then(|v| v.as_str()).unwrap_or("");
            if lab.is_empty() && !hits.iter().any(|h| std::ptr::eq(*h, n)) {
                hits.push(n);
            }
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort_by(|a, b| {
        let ra = a.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let rb = b.get("role").and_then(|v| v.as_str()).unwrap_or("");
        role_rank(ra, search_query)
            .cmp(&role_rank(rb, search_query))
            .then_with(|| label_exactness(a, &needle).cmp(&label_exactness(b, &needle)))
            .then_with(|| {
                let aa = node_area(a);
                let ab = node_area(b);
                ab.partial_cmp(&aa)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        let ya = a
                            .get("frame")
                            .and_then(|f| f.get("y"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(f64::MAX);
                        let yb = b
                            .get("frame")
                            .and_then(|f| f.get("y"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(f64::MAX);
                        ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
                    })
            })
    });

    for best in hits {
        let best_role = best.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let lab = best
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if lab.contains("nessun risultato")
            || lab.contains("no result")
            || lab.contains("nessun elemento")
        {
            continue;
        }
        let tall = best
            .get("frame")
            .and_then(|f| f.get("height"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            >= 36.0;
        let rank = role_rank(best_role, search_query);
        if rank >= 5 {
            continue;
        }
        if rank >= 4 && !tall {
            continue;
        }
        if search_query && !is_editable_role(best_role) {
            continue;
        }
        if let Some(pt) = node_center(best, point_size) {
            return Some(pt);
        }
    }
    None
}

pub fn find_label_in_dump(dump: &serde_json::Value, label: &str) -> Option<(f64, f64)> {
    let nodes = dump
        .get("elements")
        .and_then(|e| e.as_array())
        .or_else(|| dump.get("nodes").and_then(|e| e.as_array()))?;
    let point_size = dump.get("point_size").and_then(|ps| {
        if let Some(arr) = ps.as_array() {
            Some((arr.first()?.as_f64()?, arr.get(1)?.as_f64()?))
        } else {
            Some((ps.get("width")?.as_f64()?, ps.get("height")?.as_f64()?))
        }
    });
    find_label_center(nodes, label, point_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefers_search_field_over_list_row() {
        let nodes = vec![
            json!({
                "role": "AXButton",
                "label": "Cerca",
                "identifier": "com.apple.settings.search",
                "frame": {"x": 0.0, "y": 400.0, "width": 390.0, "height": 44.0}
            }),
            json!({
                "role": "AXSearchField",
                "label": "Cerca",
                "frame": {"x": 16.0, "y": 100.0, "width": 360.0, "height": 36.0}
            }),
        ];
        let (x, y) = find_label_center(&nodes, "Cerca", Some((393.0, 852.0))).unwrap();
        assert!((y - (100.0 + 18.0) / 852.0).abs() < 0.01, "y={y}");
        assert!((x - (16.0 + 180.0) / 393.0).abs() < 0.01, "x={x}");
    }

    #[test]
    fn ignores_settings_search_row_and_spotlight_pill() {
        let nodes = vec![
            json!({
                "role": "AXButton",
                "label": "Cerca",
                "identifier": "com.apple.settings.search",
                "frame": {"x": 0.0, "y": 400.0, "width": 390.0, "height": 44.0}
            }),
            json!({
                "role": "AXSlider",
                "label": "Cerca",
                "identifier": "spotlight-pill",
                "frame": {"x": 80.0, "y": 780.0, "width": 230.0, "height": 36.0}
            }),
            json!({
                "role": "AXTextField",
                "label": "",
                "frame": {"x": 16.0, "y": 56.0, "width": 360.0, "height": 36.0}
            }),
        ];
        let (x, y) = find_label_center(&nodes, "Cerca", Some((393.0, 852.0))).unwrap();
        assert!((y - (56.0 + 18.0) / 852.0).abs() < 0.01, "y={y}");
        assert!((x - (16.0 + 180.0) / 393.0).abs() < 0.01, "x={x}");
    }

    #[test]
    fn ignores_static_text_false_positive() {
        let nodes = vec![json!({
            "role": "AXStaticText",
            "label": "Nessun risultato per “Bluetooth”",
            "frame": {"x": 0.0, "y": 200.0, "width": 390.0, "height": 20.0}
        })];
        assert!(find_label_center(&nodes, "Bluetooth", Some((393.0, 852.0))).is_none());
    }

    #[test]
    fn rejects_nav_back_chrome() {
        let nodes = vec![
            json!({
                "role": "AXButton",
                "label": "< Impostazioni",
                "frame": {"x": 8.0, "y": 50.0, "width": 120.0, "height": 44.0}
            }),
            json!({
                "role": "AXButton",
                "label": "Impostazioni",
                "identifier": "Impostazioni",
                "frame": {"x": 285.0, "y": 372.0, "width": 94.0, "height": 86.0}
            }),
        ];
        let (x, y) = find_label_center(&nodes, "Impostazioni", Some((393.0, 852.0))).unwrap();
        assert!((y - (372.0 + 43.0) / 852.0).abs() < 0.02, "y={y}");
        assert!((x - (285.0 + 47.0) / 393.0).abs() < 0.02, "x={x}");
    }

    #[test]
    fn allows_tall_static_search_hit() {
        let nodes = vec![json!({
            "role": "AXStaticText",
            "label": "Generali",
            "frame": {"x": 16.0, "y": 180.0, "width": 360.0, "height": 44.0}
        })];
        let (x, y) = find_label_center(&nodes, "Generali", Some((393.0, 852.0))).unwrap();
        assert!((y - (180.0 + 22.0) / 852.0).abs() < 0.01, "y={y}");
        assert!((x - (16.0 + 180.0) / 393.0).abs() < 0.01, "x={x}");
    }

    #[test]
    fn find_id_prefers_accessibility_identifier() {
        let nodes = vec![
            json!({"id":"n4304014d","identifier":"NameField","role":"AXTextField","frame":{"x":16.0,"y":470.0,"width":361.0,"height":34.0}}),
            json!({"id":"n8108c5bd","identifier":"GoNext","role":"AXButton","frame":{"x":156.0,"y":524.0,"width":81.0,"height":34.0}}),
        ];
        let (x, y) = find_id_center(&nodes, "GoNext", Some((393.0, 852.0))).unwrap();
        assert!((x - (156.0 + 40.5) / 393.0).abs() < 0.01, "x={x}");
        assert!((y - (524.0 + 17.0) / 852.0).abs() < 0.01, "y={y}");
        // opaque tree id still works
        assert!(find_id_center(&nodes, "n4304014d", Some((393.0, 852.0))).is_some());
    }

    #[test]
    fn find_id_prefers_editable_node_over_labeled_container() {
        let nodes = vec![
            json!({
                "id": "n-wrap",
                "identifier": "login_email_field",
                "role": "AXGroup",
                "hittable": true,
                "frame": {"x": 20.0, "y": 300.0, "width": 350.0, "height": 60.0}
            }),
            json!({
                "id": "n-field",
                "identifier": "login_email_field",
                "role": "AXTextField",
                "focused": true,
                "hittable": true,
                "frame": {"x": 40.0, "y": 312.0, "width": 310.0, "height": 36.0}
            }),
        ];
        let (x, y) = find_id_center(&nodes, "login_email_field", Some((393.0, 852.0))).unwrap();
        assert!((x - (40.0 + 155.0) / 393.0).abs() < 0.01, "x={x}");
        assert!((y - (312.0 + 18.0) / 852.0).abs() < 0.01, "y={y}");
    }

    #[test]
    fn actionable_topk_prefers_buttons() {
        let nodes = vec![
            json!({"id":"n1","role":"AXApplication","label":"SpringBoard","hittable":true,"enabled":true}),
            json!({"id":"n2","role":"AXButton","label":"Messaggi","hittable":true,"enabled":true}),
        ];
        let top = build_actionable_topk(&nodes, 5);
        assert_eq!(top[0]["id"], "n2");
    }

    #[test]
    fn tab_bar_items_are_actionable_not_chrome() {
        let nodes = vec![
            json!({
                "id": "n-tabbar",
                "role": "AXGroup",
                "label": "Tab Bar",
                "hittable": true,
                "enabled": true,
                "traits": "tabbar"
            }),
            json!({
                "id": "n-home",
                "identifier": "tab_home",
                "role": "AXTabButton",
                "label": "Home",
                "hittable": true,
                "enabled": true,
                "traits": "tabbar"
            }),
            json!({
                "id": "n-card",
                "identifier": "home_product_card_1",
                "role": "AXButton",
                "label": "Love",
                "hittable": true,
                "enabled": true
            }),
        ];
        assert!(is_tab_bar_node(&nodes[0]));
        assert!(is_tab_bar_node(&nodes[1]));
        let top = build_actionable_topk(&nodes, 5);
        assert!(
            top.iter().any(|n| n["identifier"] == "tab_home"),
            "tab_home must survive top-k: {top:?}"
        );
    }

    #[test]
    fn tab_prefix_id_matches_recovered_tab_label() {
        let n = json!({
            "identifier": "house.fill",
            "label": "Home",
            "role": "AXRadioButton",
            "traits": "button,tabbar",
            "hittable": true,
            "enabled": true
        });
        assert!(is_tab_bar_node(&n));
        assert!(node_matches_identifier(&n, "tab_home"));
        assert!(!node_matches_identifier(&n, "tab_notes"));
        assert!(!node_matches_identifier(
            &json!({
                "identifier": "home_product_card_1",
                "label": "Home",
                "role": "AXButton",
                "hittable": true,
                "enabled": true
            }),
            "tab_home"
        ));
        assert!(identity_suggests_tab_label("notes_title", "Notes"));
        assert!(identity_suggests_tab_label("tab_notes", "Notes"));
        assert!(!identity_suggests_tab_label("homeTitle", "Home"));
    }

    #[test]
    fn chrome_filters_spotlight() {
        let n = json!({
            "label": "Cerca",
            "identifier": "spotlight-pill",
            "value": "Pagina 1 di 2",
            "hittable": true,
            "enabled": true,
            "role": "AXButton",
        });
        assert!(is_chrome_node(&n));
        let top = build_actionable_topk(&[n], 5);
        assert!(top.is_empty());
    }

    #[test]
    fn surface_springboard_not_settings() {
        let nodes = vec![
            json!({"label":"Messaggi","role":"AXButton","hittable":true,"enabled":true}),
            json!({"label":"Impostazioni","role":"AXButton","hittable":true,"enabled":true}),
            json!({"label":"Safari","role":"AXButton","hittable":true,"enabled":true}),
            json!({"label":"Cerca","identifier":"spotlight-pill","value":"Pagina 1 di 2","role":"AXButton","hittable":true,"enabled":true}),
        ];
        assert_eq!(detect_surface(&nodes), "springboard");
    }

    #[test]
    fn surface_springboard_fitness_watch_no_spotlight() {
        let nodes = vec![
            json!({"label":"Fitness","role":"AXButton","hittable":true,"enabled":true,"frame":{"x":29,"y":78,"width":64,"height":86}}),
            json!({"label":"Watch","role":"AXButton","hittable":true,"enabled":true,"frame":{"x":120,"y":78,"width":64,"height":86}}),
            json!({"label":"Contatti","role":"AXButton","hittable":true,"enabled":true,"frame":{"x":210,"y":78,"width":64,"height":86}}),
            json!({"label":"Safari","role":"AXButton","hittable":true,"enabled":true,"frame":{"x":300,"y":78,"width":64,"height":86}}),
            json!({"label":"Messaggi","role":"AXButton","hittable":true,"enabled":true,"frame":{"x":29,"y":180,"width":64,"height":86}}),
            json!({"label":"OnboardingDemo","role":"AXButton","hittable":true,"enabled":true,"frame":{"x":120,"y":180,"width":64,"height":86}}),
        ];
        assert_eq!(detect_surface(&nodes), "springboard");
    }

    #[test]
    fn button_dense_app_is_not_springboard() {
        let mut nodes = vec![json!({
            "label": "LighFeed",
            "role": "AXApplication",
            "hittable": true,
            "enabled": true
        })];
        nodes.extend((1..=16).map(|i| {
            json!({
                "label": format!("Post {i}"),
                "identifier": format!("post-{i}"),
                "role": "AXButton",
                "hittable": true,
                "enabled": true,
                "frame": {"x":40,"y":120 + i * 30,"width":80,"height":24}
            })
        }));
        assert_eq!(foreground_app_label(&nodes).as_deref(), Some("LighFeed"));
        assert_eq!(detect_surface(&nodes), "app");
    }

    #[test]
    fn surface_settings_inside() {
        let nodes = vec![
            json!({"label":"Cerca","role":"AXSearchField","hittable":true,"enabled":true}),
            json!({"label":"Generali","role":"AXStaticText","frame":{"x":0,"y":180,"width":300,"height":44},"hittable":true,"enabled":true}),
        ];
        assert_eq!(detect_surface(&nodes), "settings");
    }

    #[test]
    fn surface_messages_composer() {
        let nodes = vec![
            json!({"label":"A:","role":"AXTextField","hittable":true,"enabled":true}),
            json!({"label":"Messaggio","role":"AXTextView","hittable":true,"enabled":true}),
            json!({"label":"Invia","role":"AXButton","hittable":true,"enabled":true}),
        ];
        assert_eq!(detect_surface(&nodes), "messages_composer");
    }

    #[test]
    fn actionable_rejects_mega_labels_and_fullscreen_groups() {
        let mega = "Symposia Into the Wild Foo Bar TabEventsHome, tab, 1 of 5 TabMatchingEvents, tab, 2 of 5 TabRecording, tab, 3 of 5 TabChat, tab, 4 of 5 TabProfile, tab, 5 of 5";
        let nodes = vec![
            json!({
                "id": "n-mega",
                "role": "Group",
                "label": mega,
                "hittable": true,
                "enabled": true,
                "frame": {"x": 0, "y": 0, "width": 390, "height": 844}
            }),
            json!({
                "id": "n-tab",
                "role": "Button",
                "label": "TabEventsHome, tab, 1 of 5",
                "hittable": true,
                "enabled": true,
                "frame": {"x": 0, "y": 764, "width": 78, "height": 60}
            }),
            json!({
                "id": "n-row",
                "role": "Button",
                "label": "Into the Wild",
                "hittable": true,
                "enabled": true,
                "frame": {"x": 25, "y": 120, "width": 348, "height": 93}
            }),
        ];
        let top = build_actionable_topk(&nodes, 10);
        assert!(top.iter().all(|n| n.get("id") != Some(&json!("n-mega"))));
        assert!(top.iter().any(|n| n.get("label") == Some(&json!("Into the Wild"))));
        assert!(top.iter().any(|n| {
            n.get("label")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("TabEventsHome"))
        }));
    }
}
