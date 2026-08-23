//! Host cognition — settle judge, probe planner, product taste (no LLM).

use std::time::{Duration, Instant};

use ligh_core::ObserveSnapshot;
use ligh_core::overlay_from_snapshot;
use ligh_host::HidInput;

use crate::capabilities::settle_eyes;
use crate::motor::tap_effect_observed;

/// Wait until AX looks settled enough to act (blocks acting during transition).
pub fn wait_settled(build: &dyn Fn() -> ObserveSnapshot, budget_ms: u64) -> ObserveSnapshot {
    let deadline = Instant::now() + Duration::from_millis(budget_ms.max(400));
    let mut stable = 0u32;
    let mut last = build();
    while Instant::now() < deadline {
        let snap = settle_eyes(build, 350);
        if snap.eyes_unusable || snap.ax_quality == "transition" || !snap.settled {
            stable = 0;
        } else if snap.is_actionable_eyes() {
            stable += 1;
            if stable >= 2 {
                return snap;
            }
        }
        last = snap;
        std::thread::sleep(Duration::from_millis(45));
    }
    last
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeEntry {
    pub gesture: String,
    pub effect: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Run bounded exploration gestures; return log + whether UI shifted.
pub fn run_probes(
    build: &dyn Fn() -> ObserveSnapshot,
    udid: &str,
    w: f64,
    h: f64,
    max_probes: u32,
) -> (Vec<ProbeEntry>, bool) {
    let mut log = Vec::new();
    let mut any_effect = false;

    let try_gesture = |name: &str, fire: &dyn Fn()| -> bool {
        let before = build();
        fire();
        std::thread::sleep(Duration::from_millis(280));
        let after = settle_eyes(build, 600);
        tap_effect_observed(&before, &after, None)
    };

    let mut n = 0u32;

    let snap = build();
    if overlay_from_snapshot(&snap).blocks_path() && n < max_probes {
        n += 1;
        let eff = try_gesture("dismiss_keyboard", &|| {
            let _ = HidInput::key_named(udid, "return");
            std::thread::sleep(Duration::from_millis(80));
            let _ = HidInput::tap(udid, 0.5, 0.08, w, h);
        });
        any_effect |= eff;
        log.push(ProbeEntry {
            gesture: "dismiss_keyboard".into(),
            effect: eff,
            note: None,
        });
    }

    if n < max_probes {
        n += 1;
        let eff = try_gesture("scroll_up", &|| {
            let _ = HidInput::swipe(udid, 0.5, 0.84, 0.5, 0.16, w, h);
        });
        any_effect |= eff;
        log.push(ProbeEntry {
            gesture: "scroll_up".into(),
            effect: eff,
            note: None,
        });
    }

    if n < max_probes {
        n += 1;
        let eff = try_gesture("swipe_back", &|| {
            let _ = HidInput::swipe(udid, 0.06, 0.5, 0.42, 0.5, w, h);
        });
        any_effect |= eff;
        log.push(ProbeEntry {
            gesture: "swipe_back".into(),
            effect: eff,
            note: Some("iOS edge back".into()),
        });
    }

    if n < max_probes {
        n += 1;
        let eff = try_gesture("scroll_down", &|| {
            let _ = HidInput::swipe(udid, 0.5, 0.16, 0.5, 0.84, w, h);
        });
        any_effect |= eff;
        log.push(ProbeEntry {
            gesture: "scroll_down".into(),
            effect: eff,
            note: None,
        });
    }

    (log, any_effect)
}
