//! Hybrid physical UI: DevDriver eyes + WDA arms.
//!
//! Dump/perceive stays on the in-app DevDriver (fast AX). Tap/swipe/scroll/
//! type/press go through WebDriverAgent so gestures actually hit RN.

use std::sync::Arc;

use ligh_core::LighError;
use ligh_host::PhysicalUi;
use serde_json::{json, Value};

use crate::device_hub::DeviceHub;
use crate::wda::WdaArms;

pub struct HybridPhysical {
    hub: Arc<DeviceHub>,
    arms: Arc<WdaArms>,
}

impl HybridPhysical {
    pub fn new(hub: Arc<DeviceHub>, arms: Arc<WdaArms>) -> Arc<Self> {
        Arc::new(Self { hub, arms })
    }

    fn ensure_arms(&self) -> Result<(), LighError> {
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
}

impl PhysicalUi for HybridPhysical {
    fn active(&self) -> bool {
        // Eyes online is enough to "have a physical target"; arms connect on first act.
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
        // Advertise gesture-capable when arms can come online.
        self.hub.driver_version().max(2)
    }

    fn tap(&self, nx: f64, ny: f64, _w: f64, _h: f64) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.tap_norm(nx, ny)
    }

    fn tap_hold(
        &self,
        nx: f64,
        ny: f64,
        _w: f64,
        _h: f64,
        hold_ms: f64,
    ) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.tap_hold_norm(nx, ny, hold_ms)
    }

    fn swipe(
        &self,
        from_nx: f64,
        from_ny: f64,
        to_nx: f64,
        to_ny: f64,
        _w: f64,
        _h: f64,
    ) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms
            .swipe_norm(from_nx, from_ny, to_nx, to_ny, 320.0)
    }

    fn gesture(&self, points: &[Value]) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.gesture(points)
    }

    fn type_text(&self, text: &str) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.type_text(text)
    }

    fn clear(&self, count: u32) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.clear(count)
    }

    fn key_named(&self, name: &str) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.key_named(name)
    }

    fn home(&self) -> Result<(), LighError> {
        // Prefer staying in-app; real Home kills Mae session. Only if forced.
        if std::env::var("LIGH_WDA_ALLOW_HOME").ok().as_deref() == Some("1") {
            self.ensure_arms()?;
            return self.arms.home();
        }
        Ok(())
    }

    fn press_id(&self, id: &str) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.click_id(id)
    }

    fn press_label(&self, label: &str) -> Result<(), LighError> {
        self.ensure_arms()?;
        self.arms.click_label(label)
    }
}
