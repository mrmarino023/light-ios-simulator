//! Explicit daemon fault injection for transaction/fault-ownership tests.
//! Disabled unless `LIGH_FAULT_INJECT` is set; never enabled by benchmark code.

use ligh_core::{AccessibilityTree, ObserveSnapshot};

pub(crate) fn apply(snapshot: &mut ObserveSnapshot) {
    let Ok(mode) = std::env::var("LIGH_FAULT_INJECT") else {
        return;
    };
    apply_mode(snapshot, &mode);
}

fn apply_mode(snapshot: &mut ObserveSnapshot, mode: &str) {
    match mode {
        "ax_lag" => {
            snapshot.accessibility_tree = AccessibilityTree::Empty;
            snapshot.ax_quality = "transition".into();
            snapshot.settled = false;
            snapshot.eyes_unusable = true;
        }
        "foreground_drift" => {
            snapshot.observed_app_label = Some("SpringBoard".into());
        }
        "stale_epoch" => {
            snapshot.screen_epoch = snapshot.screen_epoch.saturating_add(1);
        }
        "duplicated_labels" => {
            if let AccessibilityTree::Available { nodes, .. } = &mut snapshot.accessibility_tree {
                if let Some(node) = nodes
                    .iter()
                    .find(|node| node.get("label").and_then(|v| v.as_str()).is_some())
                    .cloned()
                {
                    nodes.push(node);
                }
            }
        }
        "focus_not_reflected" => {
            if let AccessibilityTree::Available { nodes, .. } = &mut snapshot.accessibility_tree {
                for node in nodes {
                    if node.get("focused").and_then(|v| v.as_bool()) == Some(true) {
                        node["focused"] = serde_json::Value::Bool(false);
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_epoch_injection_invalidates_target_handle() {
        let mut snapshot = ObserveSnapshot {
            schema_version: 2,
            udid: "test".into(),
            session_id: Some("s".into()),
            boot_epoch: 1,
            launch_epoch: 1,
            screen_epoch: 7,
            stability_streak: 2,
            motion_score: None,
            expected_bundle_id: Some("com.test".into()),
            observed_app_label: Some("Test".into()),
            booted: true,
            simulator_app_running: false,
            frame: None,
            app_bundle_id: Some("com.test".into()),
            accessibility_tree: AccessibilityTree::Empty,
            scene: None,
            actionable_topk: vec![],
            events: vec![],
            ax_quality: "ready".into(),
            settled: true,
            observe_ms: None,
            path: Some("test".into()),
            phase: Some("ready".into()),
            eyes_unusable: false,
            overlay: Some("none".into()),
        };
        apply_mode(&mut snapshot, "stale_epoch");
        assert_eq!(snapshot.screen_epoch, 8);
    }
}
