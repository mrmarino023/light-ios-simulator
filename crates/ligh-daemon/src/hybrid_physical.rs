//! Hybrid physical UI: DevDriver eyes + cascade hands (in-app → WDA).

use std::sync::Arc;

use ligh_core::LighError;
use ligh_host::PhysicalUi;
use serde_json::{json, Value};

use crate::device_hub::DeviceHub;
use crate::wda::WdaArms;

pub struct HybridPhysical {
    pub(crate) hub: Arc<DeviceHub>,
    pub(crate) arms: Arc<WdaArms>,
}

impl HybridPhysical {
    pub fn new(hub: Arc<DeviceHub>, arms: Arc<WdaArms>) -> Arc<Self> {
        Arc::new(Self { hub, arms })
    }

    pub(crate) fn ensure_arms(&self) -> Result<(), LighError> {
        if self.arms.active() {
            return Ok(());
        }
        crate::wda::load_wda_dotenv();
        let udid = std::env::var("LIGH_WDA_UDID").unwrap_or_default();
        if udid.is_empty() {
            return Err(LighError::NotReady(
                "set LIGH_WDA_UDID (or ~/.ligh/wda.env) to the phone UDID".into(),
            ));
        }
        let bundle = self
            .hub
            .bundle_id_hint()
            .or_else(|| std::env::var("LIGH_WDA_BUNDLE").ok());
        self.arms.ensure(&udid, bundle.as_deref())
    }

    fn tap_cascade(&self, nx: f64, ny: f64, w: f64, h: f64) -> Result<(), LighError> {
        if self.hub.active() {
            if self.hub.tap(nx, ny, w, h).is_ok() {
                return Ok(());
            }
        }
        self.ensure_arms()?;
        self.arms.tap_norm(nx, ny)
    }

    fn swipe_cascade(
        &self,
        from_nx: f64,
        from_ny: f64,
        to_nx: f64,
        to_ny: f64,
        w: f64,
        h: f64,
    ) -> Result<(), LighError> {
        if self.hub.active() {
            if self
                .hub
                .swipe(from_nx, from_ny, to_nx, to_ny, w, h)
                .is_ok()
            {
                return Ok(());
            }
        }
        self.ensure_arms()?;
        self.arms
            .swipe_norm(from_nx, from_ny, to_nx, to_ny, 320.0)
    }
}

impl PhysicalUi for HybridPhysical {
    fn active(&self) -> bool {
        self.hub.active()
    }

    fn session_id(&self) -> String {
        self.hub.session_id()
    }

    fn transport(&self) -> &'static str {
        if self.arms.active() {
            "lan+wda"
        } else {
            self.hub.transport()
        }
    }

    fn bundle_id(&self) -> Option<String> {
        self.hub.bundle_id()
    }

    fn screen_points(&self) -> Option<(f64, f64)> {
        self.arms.screen().or_else(|| self.hub.screen_points())
    }

    fn dump(&self) -> Result<Value, LighError> {
        self.hub.dump()
    }

    fn capabilities(&self) -> Value {
        let mut caps = self.hub.capabilities();
        if let Some(obj) = caps.as_object_mut() {
            obj.insert("wda_arms".into(), json!(self.arms.active()));
            obj.insert("motor_cascade".into(), json!(true));
            obj.insert("tap".into(), json!(true));
            obj.insert("swipe".into(), json!(true));
            obj.insert("scroll_until".into(), json!(true));
            obj.insert("long_press".into(), json!(true));
            obj.insert("gesture".into(), json!(true));
            obj.insert("type".into(), json!(true));
        }
        caps
    }

    fn driver_version(&self) -> u64 {
        self.hub.driver_version().max(2)
    }

    fn tap(&self, nx: f64, ny: f64, w: f64, h: f64) -> Result<(), LighError> {
        self.tap_cascade(nx, ny, w, h)
    }

    fn tap_hold(
        &self,
        nx: f64,
        ny: f64,
        w: f64,
        h: f64,
        hold_ms: f64,
    ) -> Result<(), LighError> {
        if self.hub.active() && self.hub.tap_hold(nx, ny, w, h, hold_ms).is_ok() {
            return Ok(());
        }
        self.ensure_arms()?;
        self.arms.tap_hold_norm(nx, ny, hold_ms)
    }

    fn swipe(
        &self,
        from_nx: f64,
        from_ny: f64,
        to_nx: f64,
        to_ny: f64,
        w: f64,
        h: f64,
    ) -> Result<(), LighError> {
        self.swipe_cascade(from_nx, from_ny, to_nx, to_ny, w, h)
    }

    fn gesture(&self, points: &[Value]) -> Result<(), LighError> {
        if self.hub.active() && self.hub.gesture(points).is_ok() {
            return Ok(());
        }
        self.ensure_arms()?;
        self.arms.gesture(points)
    }

    fn type_text(&self, text: &str) -> Result<(), LighError> {
        if self.hub.active() && self.hub.type_text(text).is_ok() {
            return Ok(());
        }
        self.ensure_arms()?;
        self.arms.type_text(text)
    }

    fn clear(&self, count: u32) -> Result<(), LighError> {
        if self.hub.active() && self.hub.clear(count).is_ok() {
            return Ok(());
        }
        self.ensure_arms()?;
        self.arms.clear(count)
    }

    fn key_named(&self, name: &str) -> Result<(), LighError> {
        if self.hub.active() && self.hub.key_named(name).is_ok() {
            return Ok(());
        }
        self.ensure_arms()?;
        self.arms.key_named(name)
    }

    fn home(&self) -> Result<(), LighError> {
        if std::env::var("LIGH_WDA_ALLOW_HOME").ok().as_deref() == Some("1") {
            self.ensure_arms()?;
            return self.arms.home();
        }
        Ok(())
    }

    /// DevDriver semantic press only — WDA fallback lives in `physical_motor`.
    fn press_id(&self, id: &str) -> Result<(), LighError> {
        self.hub.press_id(id)
    }

    fn press_label(&self, label: &str) -> Result<(), LighError> {
        self.hub.activate_label(label).or_else(|_| self.hub.press_label(label))
    }
}
