//! Optional physical-device UI override.
//!
//! Simulator AXPTranslator + IndigoHID stay the default and work on **any**
//! Debug `.app` with no app changes. When a DevDriver session is live and
//! `LIGH_UI` is `auto` (default) or `device`, dump/tap/type skip CoreSimulator
//! so Autopilot does not fork.
//!
//! `LIGH_UI=sim|device|auto`

use std::sync::{Arc, OnceLock, RwLock};

use serde_json::Value;

use crate::LighError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    Auto,
    Sim,
    Device,
}

impl UiMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Sim => "sim",
            Self::Device => "device",
        }
    }
}

pub fn ui_mode_from(raw: Option<&str>) -> UiMode {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("sim") | Some("simulator") => UiMode::Sim,
        Some("device") | Some("physical") | Some("phone") => UiMode::Device,
        _ => UiMode::Auto,
    }
}

pub fn ui_mode() -> UiMode {
    ui_mode_from(std::env::var("LIGH_UI").ok().as_deref())
}

pub trait PhysicalUi: Send + Sync {
    fn active(&self) -> bool;
    fn session_id(&self) -> String;
    fn transport(&self) -> &'static str;
    fn bundle_id(&self) -> Option<String>;
    fn screen_points(&self) -> Option<(f64, f64)>;
    fn dump(&self) -> Result<Value, LighError>;
    fn tap(&self, nx: f64, ny: f64, width: f64, height: f64) -> Result<(), LighError>;
    fn tap_hold(
        &self,
        nx: f64,
        ny: f64,
        width: f64,
        height: f64,
        hold_ms: f64,
    ) -> Result<(), LighError>;
    fn swipe(
        &self,
        from_nx: f64,
        from_ny: f64,
        to_nx: f64,
        to_ny: f64,
        width: f64,
        height: f64,
    ) -> Result<(), LighError>;
    fn type_text(&self, text: &str) -> Result<(), LighError>;
    fn clear(&self, count: u32) -> Result<(), LighError>;
    fn key_named(&self, name: &str) -> Result<(), LighError>;
    fn home(&self) -> Result<(), LighError>;
    fn press_id(&self, id: &str) -> Result<(), LighError>;
    fn press_label(&self, label: &str) -> Result<(), LighError>;
    /// Human Gesture IR: ordered samples `{nx,ny,t_ms,phase}` (normalized).
    fn gesture(&self, points: &[Value]) -> Result<(), LighError> {
        let _ = points;
        Err(LighError::NotReady("gesture not supported on this motor".into()))
    }
    /// Driver capability document from device `hello` (empty on Simulator).
    fn capabilities(&self) -> Value {
        Value::Object(Default::default())
    }
    fn driver_version(&self) -> u64 {
        0
    }
}

fn slot() -> &'static RwLock<Option<Arc<dyn PhysicalUi>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<dyn PhysicalUi>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

pub fn set_physical_ui(ui: Option<Arc<dyn PhysicalUi>>) {
    *slot().write().expect("physical ui slot") = ui;
}

pub fn physical_ui() -> Option<Arc<dyn PhysicalUi>> {
    slot().read().ok().and_then(|g| g.clone())
}

pub fn physical_ui_active() -> bool {
    if ui_mode() == UiMode::Sim {
        return false;
    }
    physical_ui().is_some_and(|u| u.active())
}

pub fn ui_target() -> &'static str {
    if physical_ui_active() {
        "physical"
    } else {
        "simulator"
    }
}

pub(crate) fn with_physical<T>(
    f: impl FnOnce(&dyn PhysicalUi) -> Result<T, LighError>,
) -> Option<Result<T, LighError>> {
    match ui_mode() {
        UiMode::Sim => None,
        UiMode::Device => match physical_ui() {
            Some(ui) if ui.active() => Some(f(&*ui)),
            _ => Some(Err(LighError::NotReady(
                "LIGH_UI=device but no DevDriver connected — open the Debug/dev-client app on the phone"
                    .into(),
            ))),
        },
        UiMode::Auto => {
            let ui = physical_ui()?;
            ui.active().then(|| f(&*ui))
        }
    }
}
