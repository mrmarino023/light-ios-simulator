//! Scene IR — hyper-computational UI transform.
//!
//! Host-owned projection from AX atoms → topological regions with measured ε.
//! Fail-closed: kind ≠ flat only when residual ≤ ε(kind) and membership is a
//! partition. No app-specific templates. No vision.

use serde::{Deserialize, Serialize};

use crate::feel::{FeelIR, FeelPhase, SalienceItem, WorldModel};
use crate::observe::ObserveSnapshot;
use crate::qa::AffordanceKind;

pub const SCENE_SCHEMA_VERSION: u32 = 1;

/// Closed vocabulary — every UI maps here or to `flat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    Point,
    Row,
    Col,
    Grid,
    StripH,
    StripV,
    Radial,
    ChromeBand,
    Overlay,
    Flat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionRole {
    Nav,
    Content,
    Chrome,
    Overlay,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePhase {
    Settled,
    Transition,
    Blocked,
    EyesUnusable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicClass {
    Enter,
    Exit,
    Move,
    Settle,
    Reflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotorOp {
    Tap,
    Type,
    ScrollX,
    ScrollY,
    Back,
    Tab,
    Dismiss,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneAtom {
    pub identity: String,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<AffordanceKind>,
    pub on_screen: bool,
    pub tab_chrome: bool,
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneScroll {
    pub axis: String,
    pub can_neg: bool,
    pub can_pos: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneRegion {
    pub id: String,
    pub kind: RegionKind,
    /// Fit residual (lower = tighter). Flat uses +∞ sentinel as null on wire via skip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epsilon: Option<f64>,
    /// 0 ⇒ agent must ignore `kind` (treat as flat).
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_hint: Option<RegionRole>,
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members_overflow: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<SceneScroll>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDynamic {
    pub on: String,
    pub class: DynamicClass,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTrust {
    pub partition_ok: bool,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_epsilon: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSalience {
    pub identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub w: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneMotor {
    pub allowed: Vec<MotorOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenePlace {
    pub fp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
}

/// Agent-facing digest — enum-only, budget-capped, fail-closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDigest {
    pub schema: u32,
    pub place: ScenePlace,
    pub phase: ScenePhase,
    pub trust: SceneTrust,
    pub regions: Vec<SceneRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamics: Vec<SceneDynamic>,
    pub salience: Vec<SceneSalience>,
    pub motor: SceneMotor,
}

const MEMBER_CAP: usize = 12;
const REGION_CAP: usize = 16;
const SALIENCE_CAP: usize = 8;

/// ε thresholds — kind accepted only if residual ≤ these.
fn epsilon_limit(kind: RegionKind) -> f64 {
    match kind {
        RegionKind::Row | RegionKind::Col => 0.18,
        RegionKind::Grid => 0.22,
        RegionKind::Radial => 0.20,
        RegionKind::ChromeBand => 0.25,
        RegionKind::StripH | RegionKind::StripV => 0.22,
        RegionKind::Point | RegionKind::Overlay | RegionKind::Flat => f64::INFINITY,
    }
}

fn node_frame(n: &serde_json::Value) -> Option<(f64, f64, f64, f64)> {
    let f = n.get("frame")?.as_object()?;
    let x = f.get("x")?.as_f64()?;
    let y = f.get("y")?.as_f64()?;
    let w = f
        .get("width")
        .or_else(|| f.get("w"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .max(1.0);
    let h = f
        .get("height")
        .or_else(|| f.get("h"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .max(1.0);
    Some((x + w * 0.5, y + h * 0.5, w, h))
}

fn node_identity(n: &serde_json::Value, index: usize) -> String {
    if let Some(id) = n
        .get("identifier")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return id.to_string();
    }
    if let Some(id) = n
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return id.to_string();
    }
    if let Some(label) = n
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return label.to_string();
    }
    format!("anon:{index}")
}

/// Extract on-screen atoms with geometry. Nodes without frames are skipped
/// (cannot participate in measured topology — fail-closed).
pub fn atoms_from_nodes(nodes: &[serde_json::Value]) -> Vec<SceneAtom> {
    let mut out = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let hittable = n.get("hittable").and_then(|v| v.as_bool()).unwrap_or(true);
        let visible = n.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
        if !hittable || !visible {
            continue;
        }
        let Some((cx, cy, w, h)) = node_frame(n) else {
            continue;
        };
        let tab = crate::observe::is_tab_bar_node(n);
        let role = n.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let editable = role.to_ascii_lowercase().contains("textfield")
            || role.to_ascii_lowercase().contains("secure");
        out.push(SceneAtom {
            identity: node_identity(n, i),
            cx,
            cy,
            w,
            h,
            kind: None,
            on_screen: true,
            tab_chrome: tab,
            editable,
        });
    }
    out
}

#[derive(Clone)]
struct FitResult {
    kind: RegionKind,
    residual: f64,
}

fn cv(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    if mean.abs() < 1e-6 {
        return f64::INFINITY;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt() / mean.abs()
}

fn fit_row(atoms: &[&SceneAtom]) -> FitResult {
    if atoms.len() < 2 {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    let mut sorted: Vec<_> = atoms.to_vec();
    sorted.sort_by(|a, b| a.cx.partial_cmp(&b.cx).unwrap_or(std::cmp::Ordering::Equal));
    let ys: Vec<f64> = sorted.iter().map(|a| a.cy).collect();
    let y_mean = ys.iter().sum::<f64>() / ys.len() as f64;
    let y_span = ys
        .iter()
        .map(|y| (y - y_mean).abs())
        .fold(0.0_f64, f64::max);
    let med_h = {
        let mut hs: Vec<f64> = sorted.iter().map(|a| a.h).collect();
        hs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        hs[hs.len() / 2]
    };
    let align = if med_h > 1.0 { y_span / med_h } else { y_span };
    let gaps: Vec<f64> = sorted.windows(2).map(|w| (w[1].cx - w[0].cx).abs()).collect();
    let residual = (align + cv(&gaps)) * 0.5;
    FitResult {
        kind: RegionKind::Row,
        residual,
    }
}

fn fit_col(atoms: &[&SceneAtom]) -> FitResult {
    if atoms.len() < 2 {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    let mut sorted: Vec<_> = atoms.to_vec();
    sorted.sort_by(|a, b| a.cy.partial_cmp(&b.cy).unwrap_or(std::cmp::Ordering::Equal));
    let xs: Vec<f64> = sorted.iter().map(|a| a.cx).collect();
    let x_mean = xs.iter().sum::<f64>() / xs.len() as f64;
    let x_span = xs
        .iter()
        .map(|x| (x - x_mean).abs())
        .fold(0.0_f64, f64::max);
    let med_w = {
        let mut ws: Vec<f64> = sorted.iter().map(|a| a.w).collect();
        ws.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        ws[ws.len() / 2]
    };
    let align = if med_w > 1.0 { x_span / med_w } else { x_span };
    let gaps: Vec<f64> = sorted.windows(2).map(|w| (w[1].cy - w[0].cy).abs()).collect();
    let residual = (align + cv(&gaps)) * 0.5;
    FitResult {
        kind: RegionKind::Col,
        residual,
    }
}

fn fit_grid(atoms: &[&SceneAtom]) -> FitResult {
    let n = atoms.len();
    if n < 4 {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    // Try small factorizations near sqrt(n).
    let root = (n as f64).sqrt().round() as usize;
    let mut best = f64::INFINITY;
    for rows in root.saturating_sub(1)..=root + 1 {
        if rows == 0 || n % rows != 0 {
            continue;
        }
        let cols = n / rows;
        if cols < 2 || rows < 2 {
            continue;
        }
        let mut sorted: Vec<_> = atoms.to_vec();
        sorted.sort_by(|a, b| {
            a.cy.partial_cmp(&b.cy)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cx.partial_cmp(&b.cx).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut xs: Vec<f64> = sorted.iter().map(|a| a.cx).collect();
        let mut ys: Vec<f64> = sorted.iter().map(|a| a.cy).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // Unique-ish pitches via every cols-th / rows-th is fragile; use CV of all gaps.
        let x_gaps: Vec<f64> = (0..cols.saturating_sub(1))
            .filter_map(|i| {
                let a = xs.get(i)?;
                let b = xs.get(i + 1)?;
                Some((b - a).abs())
            })
            .collect();
        let y_gaps: Vec<f64> = (0..rows.saturating_sub(1))
            .filter_map(|i| {
                let a = ys.get(i)?;
                let b = ys.get(i + 1)?;
                Some((b - a).abs())
            })
            .collect();
        let r = (cv(&x_gaps) + cv(&y_gaps)) * 0.5;
        if r < best {
            best = r;
        }
    }
    FitResult {
        kind: RegionKind::Grid,
        residual: best,
    }
}

fn fit_radial(atoms: &[&SceneAtom]) -> FitResult {
    if atoms.len() < 3 {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    let cx = atoms.iter().map(|a| a.cx).sum::<f64>() / atoms.len() as f64;
    let cy = atoms.iter().map(|a| a.cy).sum::<f64>() / atoms.len() as f64;
    let radii: Vec<f64> = atoms
        .iter()
        .map(|a| ((a.cx - cx).hypot(a.cy - cy)).max(1.0))
        .collect();
    let r_mean = radii.iter().sum::<f64>() / radii.len() as f64;
    let r_cv = cv(&radii);
    // Prefer similar circle sizes among members.
    let sizes: Vec<f64> = atoms.iter().map(|a| (a.w + a.h) * 0.5).collect();
    let size_cv = cv(&sizes);
    // Angular coverage: large gaps hurt.
    let mut angles: Vec<f64> = atoms
        .iter()
        .map(|a| (a.cy - cy).atan2(a.cx - cx))
        .collect();
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut gaps = Vec::new();
    for w in angles.windows(2) {
        gaps.push(w[1] - w[0]);
    }
    if let (Some(first), Some(last)) = (angles.first(), angles.last()) {
        gaps.push(std::f64::consts::TAU - (last - first));
    }
    let gap_cv = cv(&gaps);
    let residual = (r_cv + size_cv * 0.5 + gap_cv * 0.35) / (1.0 + (r_mean / 100.0).min(1.0));
    FitResult {
        kind: RegionKind::Radial,
        residual,
    }
}

fn fit_chrome_band(atoms: &[&SceneAtom], screen_h: f64) -> FitResult {
    if atoms.len() < 2 || screen_h < 1.0 {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    let y_mean = atoms.iter().map(|a| a.cy).sum::<f64>() / atoms.len() as f64;
    let near_bottom = y_mean > screen_h * 0.78;
    let near_top = y_mean < screen_h * 0.18;
    if !near_bottom && !near_top {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    let row = fit_row(atoms);
    let tabbish = atoms.iter().filter(|a| a.tab_chrome).count();
    let boost = if tabbish > 0 { 0.85 } else { 1.0 };
    FitResult {
        kind: RegionKind::ChromeBand,
        residual: row.residual * boost,
    }
}

fn best_fit(atoms: &[&SceneAtom], screen_h: f64) -> FitResult {
    if atoms.is_empty() {
        return FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        };
    }
    if atoms.len() == 1 {
        return FitResult {
            kind: RegionKind::Point,
            residual: 0.0,
        };
    }
    let row = fit_row(atoms);
    let col = fit_col(atoms);
    let grid = fit_grid(atoms);
    let radial = fit_radial(atoms);
    let chrome = fit_chrome_band(atoms, screen_h);
    let linear_best = row.residual.min(col.residual);

    let mut best = FitResult {
        kind: RegionKind::Flat,
        residual: f64::INFINITY,
    };
    for c in [&row, &col, &chrome, &radial, &grid] {
        if c.kind == RegionKind::Grid && c.residual + 0.04 >= linear_best {
            continue;
        }
        if c.residual < best.residual {
            best = FitResult {
                kind: c.kind,
                residual: c.residual,
            };
        }
    }
    let tab_ratio =
        atoms.iter().filter(|a| a.tab_chrome).count() as f64 / atoms.len() as f64;
    if tab_ratio >= 0.5 && chrome.residual <= epsilon_limit(RegionKind::ChromeBand) {
        best = FitResult {
            kind: chrome.kind,
            residual: chrome.residual,
        };
    }

    if best.kind != RegionKind::Flat && best.residual <= epsilon_limit(best.kind) {
        best
    } else {
        FitResult {
            kind: RegionKind::Flat,
            residual: f64::INFINITY,
        }
    }
}

/// Union-Find clustering on spatial proximity.
fn cluster_indices(atoms: &[SceneAtom]) -> Vec<Vec<usize>> {
    let n = atoms.len();
    if n == 0 {
        return vec![];
    }
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank = vec![0u8; n];
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    fn unite(parent: &mut [usize], rank: &mut [u8], a: usize, b: usize) {
        let mut a = find(parent, a);
        let mut b = find(parent, b);
        if a == b {
            return;
        }
        if rank[a] < rank[b] {
            std::mem::swap(&mut a, &mut b);
        }
        parent[b] = a;
        if rank[a] == rank[b] {
            rank[a] += 1;
        }
    }

    let mut sizes: Vec<f64> = atoms.iter().map(|a| (a.w + a.h) * 0.5).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = sizes[sizes.len() / 2].max(8.0);
    let thresh = med * 2.4;

    // Separate chrome from content early — different clusters.
    for i in 0..n {
        for j in (i + 1)..n {
            if atoms[i].tab_chrome != atoms[j].tab_chrome {
                continue;
            }
            let d = (atoms[i].cx - atoms[j].cx).hypot(atoms[i].cy - atoms[j].cy);
            if d <= thresh {
                unite(&mut parent, &mut rank, i, j);
            }
        }
    }

    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        buckets.entry(r).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = buckets.into_values().collect();
    groups.sort_by_key(|g| g[0]);
    groups
}

fn promote_strip(kind: RegionKind, atoms: &[&SceneAtom], has_scroll: bool) -> RegionKind {
    if atoms.iter().any(|a| a.tab_chrome) {
        return kind;
    }
    // Fail-closed: strip only with scroll-container evidence or clear peek.
    match kind {
        RegionKind::Row if has_scroll => RegionKind::StripH,
        RegionKind::Col if has_scroll => RegionKind::StripV,
        RegionKind::Row => {
            let min_x = atoms
                .iter()
                .map(|a| a.cx - a.w * 0.5)
                .fold(f64::INFINITY, f64::min);
            let max_x = atoms
                .iter()
                .map(|a| a.cx + a.w * 0.5)
                .fold(f64::NEG_INFINITY, f64::max);
            let med_w = {
                let mut ws: Vec<f64> = atoms.iter().map(|a| a.w).collect();
                ws.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                ws[ws.len() / 2]
            };
            // Peek: total span clearly larger than packed members (overlap / off-edge).
            if atoms.len() >= 3 && (max_x - min_x) > med_w * (atoms.len() as f64) * 1.05 {
                RegionKind::StripH
            } else {
                kind
            }
        }
        other => other,
    }
}

fn role_for(kind: RegionKind, atoms: &[&SceneAtom]) -> Option<RegionRole> {
    if atoms.iter().any(|a| a.tab_chrome) || kind == RegionKind::ChromeBand {
        return Some(RegionRole::Chrome);
    }
    if atoms.iter().any(|a| a.editable) {
        return Some(RegionRole::Input);
    }
    match kind {
        RegionKind::Overlay => Some(RegionRole::Overlay),
        RegionKind::Point if atoms.iter().any(|a| {
            matches!(
                a.kind,
                Some(AffordanceKind::NavBack) | Some(AffordanceKind::PrimaryButton)
            )
        }) =>
        {
            Some(RegionRole::Nav)
        }
        _ => Some(RegionRole::Content),
    }
}

fn confidence_for(kind: RegionKind, residual: f64) -> f64 {
    if kind == RegionKind::Flat {
        return 0.0;
    }
    if kind == RegionKind::Point {
        return 1.0;
    }
    let lim = epsilon_limit(kind);
    if !lim.is_finite() || residual > lim {
        return 0.0;
    }
    ((1.0 - residual / lim).clamp(0.0, 1.0) * 0.95 + 0.05).clamp(0.0, 1.0)
}

/// Core transform: atoms → regions (partition, measured kinds).
pub fn regionize(atoms: &[SceneAtom], screen_h: f64, has_scroll: bool) -> (Vec<SceneRegion>, bool) {
    let groups = cluster_indices(atoms);
    let mut regions = Vec::new();
    let mut covered = 0usize;

    for (gi, idxs) in groups.iter().enumerate() {
        let refs: Vec<&SceneAtom> = idxs.iter().map(|&i| &atoms[i]).collect();
        let mut fit = best_fit(&refs, screen_h);
        fit.kind = promote_strip(fit.kind, &refs, has_scroll);
        // Re-validate strip against ε of promoted kind.
        if !matches!(fit.kind, RegionKind::Flat | RegionKind::Point)
            && fit.residual > epsilon_limit(fit.kind)
            && !matches!(fit.kind, RegionKind::StripH | RegionKind::StripV)
        {
            fit.kind = RegionKind::Flat;
            fit.residual = f64::INFINITY;
        }
        if matches!(fit.kind, RegionKind::StripH | RegionKind::StripV)
            && fit.residual > epsilon_limit(fit.kind)
        {
            // Strip promotion keeps base residual; allow slightly looser.
            if fit.residual > epsilon_limit(fit.kind) * 1.15 {
                fit.kind = RegionKind::Flat;
                fit.residual = f64::INFINITY;
            }
        }

        let mut members: Vec<String> = refs.iter().map(|a| a.identity.clone()).collect();
        members.sort();
        let overflow = if members.len() > MEMBER_CAP {
            Some((members.len() - MEMBER_CAP) as u32)
        } else {
            None
        };
        members.truncate(MEMBER_CAP);

        let primary = refs
            .iter()
            .max_by(|a, b| {
                (a.w * a.h)
                    .partial_cmp(&(b.w * b.h))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|a| a.identity.clone());

        let scroll = match fit.kind {
            RegionKind::StripH => Some(SceneScroll {
                axis: "x".into(),
                can_neg: true,
                can_pos: true,
            }),
            RegionKind::StripV => Some(SceneScroll {
                axis: "y".into(),
                can_neg: true,
                can_pos: true,
            }),
            _ => None,
        };

        let conf = confidence_for(fit.kind, fit.residual);
        let kind = if conf <= 0.0 && fit.kind != RegionKind::Point {
            RegionKind::Flat
        } else {
            fit.kind
        };

        regions.push(SceneRegion {
            id: format!("r{gi}"),
            kind,
            epsilon: if kind == RegionKind::Flat {
                None
            } else {
                Some((fit.residual * 1000.0).round() / 1000.0)
            },
            confidence: if kind == RegionKind::Flat {
                0.0
            } else {
                conf
            },
            role_hint: role_for(kind, &refs),
            members,
            members_overflow: overflow,
            primary,
            scroll,
        });
        covered += idxs.len();
    }

    regions.truncate(REGION_CAP);
    let partition_ok = covered == atoms.len();
    (regions, partition_ok)
}

fn phase_from_feel(phase: FeelPhase) -> ScenePhase {
    match phase {
        FeelPhase::Settled => ScenePhase::Settled,
        FeelPhase::Transition => ScenePhase::Transition,
        FeelPhase::Blocked => ScenePhase::Blocked,
        FeelPhase::EyesUnusable => ScenePhase::EyesUnusable,
    }
}

fn motor_from_world(world: &WorldModel, regions: &[SceneRegion]) -> SceneMotor {
    let mut allowed = vec![MotorOp::Tap];
    if world.elements.iter().any(|e| e.editable && e.on_screen) {
        allowed.push(MotorOp::Type);
    }
    if regions.iter().any(|r| r.kind == RegionKind::StripH) || world.has_scroll_container {
        allowed.push(MotorOp::ScrollX);
    }
    if regions.iter().any(|r| r.kind == RegionKind::StripV) || world.has_scroll_container {
        allowed.push(MotorOp::ScrollY);
    }
    if world.can_navigate_back {
        allowed.push(MotorOp::Back);
    }
    if world.has_tab_bar || regions.iter().any(|r| r.kind == RegionKind::ChromeBand) {
        allowed.push(MotorOp::Tab);
    }
    if world.elements.iter().any(|e| e.overlay_scope.is_some()) {
        allowed.push(MotorOp::Dismiss);
    }
    allowed.sort_by_key(|o| format!("{o:?}"));
    allowed.dedup();
    SceneMotor { allowed }
}

fn salience_digest(items: &[SalienceItem]) -> Vec<SceneSalience> {
    items
        .iter()
        .take(SALIENCE_CAP)
        .filter_map(|s| {
            let identity = s
                .id
                .clone()
                .or_else(|| s.label.clone())?;
            Some(SceneSalience {
                identity,
                kind: Some(format!("{:?}", s.kind).to_ascii_lowercase()),
                w: s.weight,
            })
        })
        .collect()
}

/// Build SceneDigest from a live snapshot + Feel frame.
pub fn build_scene_digest(snap: &ObserveSnapshot, feel: &FeelIR) -> SceneDigest {
    let nodes = snap.accessibility_tree.nodes();
    let atoms = atoms_from_nodes(nodes);
    let screen_h = snap
        .accessibility_tree
        .point_size()
        .map(|(_, h)| h)
        .or_else(|| {
            atoms
                .iter()
                .map(|a| a.cy + a.h * 0.5)
                .fold(None, |acc: Option<f64>, y| {
                    Some(acc.map_or(y, |m| m.max(y)))
                })
        })
        .unwrap_or(852.0);

    let (regions, partition_ok) =
        regionize(&atoms, screen_h, feel.world.has_scroll_container);

    let max_eps = regions
        .iter()
        .filter_map(|r| r.epsilon)
        .fold(None, |acc, e| Some(acc.map_or(e, |m: f64| m.max(e))));

    let complete = partition_ok && !matches!(feel.feel.phase, FeelPhase::EyesUnusable);

    let dynamics = {
        let mut d = Vec::new();
        if feel.delta.fingerprint_changed {
            d.push(SceneDynamic {
                on: "place".into(),
                class: DynamicClass::Reflow,
                score: 1.0,
            });
        }
        if matches!(feel.feel.phase, FeelPhase::Transition) {
            d.push(SceneDynamic {
                on: "scene".into(),
                class: DynamicClass::Settle,
                score: feel.world.motion_score.unwrap_or(0.5),
            });
        }
        d
    };

    let motor = motor_from_world(&feel.world, &regions);

    SceneDigest {
        schema: SCENE_SCHEMA_VERSION,
        place: ScenePlace {
            fp: feel.place.fingerprint.clone(),
            surface: feel.place.surface.clone(),
            title: feel.place.title.clone(),
            bundle: feel
                .place
                .bundle_id
                .clone()
                .or_else(|| snap.app_bundle_id.clone()),
        },
        phase: phase_from_feel(feel.feel.phase),
        trust: SceneTrust {
            partition_ok,
            complete,
            max_epsilon: max_eps,
        },
        regions,
        dynamics,
        salience: salience_digest(&feel.salience),
        motor,
    }
}

/// Compact JSON for the agent wire (same as struct serialize).
pub fn scene_agent_view(digest: &SceneDigest) -> serde_json::Value {
    serde_json::to_value(digest).unwrap_or_else(|_| serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn atom(id: &str, cx: f64, cy: f64, w: f64, h: f64) -> SceneAtom {
        SceneAtom {
            identity: id.into(),
            cx,
            cy,
            w,
            h,
            kind: None,
            on_screen: true,
            tab_chrome: false,
            editable: false,
        }
    }

    #[test]
    fn row_of_buttons_classifies_as_row_or_strip() {
        let atoms = vec![
            atom("a", 40.0, 100.0, 30.0, 30.0),
            atom("b", 90.0, 102.0, 30.0, 30.0),
            atom("c", 140.0, 99.0, 30.0, 30.0),
            atom("d", 190.0, 101.0, 30.0, 30.0),
        ];
        let (regs, ok) = regionize(&atoms, 800.0, false);
        assert!(ok);
        assert_eq!(regs.len(), 1);
        assert!(
            matches!(regs[0].kind, RegionKind::Row | RegionKind::StripH),
            "got {:?}",
            regs[0].kind
        );
        assert!(regs[0].confidence > 0.0);
        assert!(regs[0].epsilon.is_some());
    }

    #[test]
    fn column_form_classifies_as_col() {
        let atoms = vec![
            atom("u", 200.0, 120.0, 160.0, 36.0),
            atom("p", 200.0, 180.0, 160.0, 36.0),
            atom("go", 200.0, 250.0, 120.0, 44.0),
        ];
        let (regs, ok) = regionize(&atoms, 800.0, false);
        assert!(ok);
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].kind, RegionKind::Col);
    }

    #[test]
    fn scattered_points_stay_flat_or_points_no_radial_lie() {
        let atoms = vec![
            atom("a", 10.0, 10.0, 20.0, 20.0),
            atom("b", 300.0, 50.0, 20.0, 20.0),
            atom("c", 80.0, 400.0, 20.0, 20.0),
        ];
        let (regs, ok) = regionize(&atoms, 800.0, false);
        assert!(ok);
        for r in &regs {
            assert_ne!(r.kind, RegionKind::Radial, "must not invent radial");
            if r.kind == RegionKind::Flat {
                assert_eq!(r.confidence, 0.0);
            }
        }
    }

    #[test]
    fn radial_arc_classifies_when_geometry_fits() {
        let cx = 200.0;
        let cy = 300.0;
        let r = 120.0;
        let mut atoms = Vec::new();
        for (i, ang) in [0.0_f64, 0.7, 1.4, 2.1, 2.8, 3.5, 4.2, 5.0]
            .into_iter()
            .enumerate()
        {
            atoms.push(atom(
                &format!("av{i}"),
                cx + r * ang.cos(),
                cy + r * ang.sin(),
                40.0,
                40.0,
            ));
        }
        let (regs, ok) = regionize(&atoms, 800.0, false);
        assert!(ok);
        assert!(
            regs.iter().any(|r| r.kind == RegionKind::Radial && r.confidence > 0.0),
            "expected radial, got {:?}",
            regs.iter().map(|r| r.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tab_chrome_band_at_bottom() {
        let mut atoms = vec![
            atom("t0", 40.0, 820.0, 40.0, 40.0),
            atom("t1", 120.0, 820.0, 40.0, 40.0),
            atom("t2", 200.0, 820.0, 40.0, 40.0),
            atom("t3", 280.0, 820.0, 40.0, 40.0),
            atom("t4", 360.0, 820.0, 40.0, 40.0),
        ];
        for a in &mut atoms {
            a.tab_chrome = true;
        }
        let (regs, ok) = regionize(&atoms, 852.0, false);
        assert!(ok);
        assert!(
            matches!(
                regs[0].kind,
                RegionKind::ChromeBand | RegionKind::Row | RegionKind::StripH
            ),
            "got {:?}",
            regs[0].kind
        );
        assert_eq!(
            regs[0].role_hint,
            Some(RegionRole::Chrome),
            "tab chrome must role=chrome"
        );
    }

    #[test]
    fn partition_covers_every_atom() {
        let atoms = vec![
            atom("a", 50.0, 50.0, 20.0, 20.0),
            atom("b", 60.0, 55.0, 20.0, 20.0),
            atom("far", 350.0, 700.0, 30.0, 30.0),
        ];
        let (regs, ok) = regionize(&atoms, 800.0, false);
        assert!(ok);
        let mut ids: Vec<String> = regs.iter().flat_map(|r| r.members.clone()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["a".to_string(), "b".to_string(), "far".to_string()]
        );
    }

    #[test]
    fn atoms_from_nodes_skip_missing_frames() {
        let nodes = vec![
            json!({"identifier":"x","hittable":true,"visible":true,"frame":{"x":0,"y":0,"width":10,"height":10}}),
            json!({"identifier":"nof","hittable":true,"visible":true}),
        ];
        let atoms = atoms_from_nodes(&nodes);
        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms[0].identity, "x");
    }
}
