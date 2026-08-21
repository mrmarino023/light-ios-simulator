//! Core types and configuration for LIGH.

pub mod config;
pub mod device;
pub mod error;
pub mod observe;
pub mod profile;
pub mod rpc;
pub mod state;

pub use config::LighConfig;
pub use device::DevicePreset;
pub use error::{LighError, Result};
pub use observe::{
    build_actionable_topk, build_scene, detect_surface, diff_sense_events, eyes_ready,
    find_id_center, find_id_in_dump, find_label_center, find_label_in_dump, is_chrome_node,
    is_transition_sparse, AccessibilityTree, FrameMeta, ObserveSnapshot, SceneMeta, SenseEvent,
    ACTIONABLE_TOPK, OBSERVE_SCHEMA_VERSION,
};
pub use profile::{FeatureRequirements, resolve_disabled_jobs, slim_labels};
pub use rpc::{
    default_sock_path, ensure_daemon, sibling_lighd, DaemonClient, DaemonRequest, DaemonResponse,
};
pub use state::SessionState;

#[cfg(test)]
mod tests {
    use super::DevicePreset;

    #[test]
    fn parses_device_presets() {
        assert_eq!(
            "iphone-15-pro".parse::<DevicePreset>().unwrap(),
            DevicePreset::Iphone15Pro
        );
    }

    #[test]
    fn iphone_15_pro_hid_points_from_framebuffer() {
        let (w, h) = DevicePreset::Iphone15Pro.hid_size_from_framebuffer(1179, 2556);
        assert!((w - 393.0).abs() < 0.01);
        assert!((h - 852.0).abs() < 0.01);
    }
}
