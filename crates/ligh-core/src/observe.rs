//! Structured observation snapshot — the agent-facing JSON contract.

use serde::{Deserialize, Serialize};

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

/// Full structured observation — returned by `ligh observe` and `lighd observe` RPC.
///
/// Deliberately minimal MVP observation contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveSnapshot {
    /// Contract version — bump only on breaking changes; additive fields keep same version.
    #[serde(default = "observe_schema_v1")]
    pub schema_version: u32,
    pub udid: String,
    pub booted: bool,
    /// Simulator.app is NOT running when LIGH owns the host (the point).
    pub simulator_app_running: bool,
    pub frame: Option<FrameMeta>,
    /// Active app bundle id if known from session state.
    pub app_bundle_id: Option<String>,
    /// Accessibility tree from headless AXPTranslator (or empty/error).
    pub accessibility_tree: AccessibilityTree,
    /// Server-side time to build this snapshot (hot path metric).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observe_ms: Option<f64>,
    /// How the client reached host state: `lighd` (hot) or `direct` (cold).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn observe_schema_v1() -> u32 {
    OBSERVE_SCHEMA_VERSION
}

/// Current observe JSON contract version (see `docs/OBSERVE.md`).
pub const OBSERVE_SCHEMA_VERSION: u32 = 1;

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

    /// Center of best element whose label or identifier contains `needle`.
    /// Prefers text/search fields, then top-most match (search bars beat list rows).
    pub fn find_label(&self, needle: &str) -> Option<(f64, f64)> {
        match self {
            Self::Available {
                nodes, point_size, ..
            } => find_label_center(nodes, needle, *point_size),
            _ => None,
        }
    }
}

fn is_editable_role(role: &str) -> bool {
    let r = role.to_ascii_lowercase();
    r.contains("searchfield") || r.contains("textfield") || r.contains("textarea")
}

/// Lower is better. Prefer fields/buttons over static copy (avoids matching
/// "Nessun risultato per Bluetooth" when waiting for a Bluetooth row).
/// Lower is better. Prefer fields only for Cerca/Search; otherwise prefer buttons
/// so waiting for "Generali" hits the list row, not the search field value.
fn role_rank(role: &str, prefer_editable: bool) -> u8 {
    let r = role.to_ascii_lowercase();
    if is_editable_role(&r) {
        if prefer_editable {
            0
        } else {
            3 // demote fields when looking for list rows / icons
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
        5 // never prefer app/window chrome for agent taps
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

/// Nav back chrome like "< Impostazioni" must not match needle "Impostazioni".
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

fn node_center(
    el: &serde_json::Value,
    point_size: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    let frame = el.get("frame")?;
    let x = frame.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = frame.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let w = frame.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let h = frame.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let (pw, ph) = point_size.unwrap_or((393.0, 852.0));
    if pw <= 0.0 || ph <= 0.0 {
        return None;
    }
    // Full-screen / near-full frames (app root, dimming views) produce false 0.5,0.5 taps.
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
    // Settings search bar is often an empty-label AXTextField — include it for Cerca/Search.
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
            // Home app icons are larger than Settings nav "Impostazioni" chrome.
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
    let nodes = dump.get("elements").and_then(|e| e.as_array())?;
    let point_size = dump.get("point_size").and_then(|ps| {
        Some((ps.get("width")?.as_f64()?, ps.get("height")?.as_f64()?))
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
}
