//! System surfaces — foreign-process overlays above the expected app.
//!
//! Architectural contract (not Safari-specific patches):
//! 1. **Discover** occlusion by hit-test / alternate AX root (host may still be "frontmost").
//! 2. **Classify** the foreign process into a [`SystemSurfaceRole`].
//! 3. **Motor policy** comes only from the role — never from hard-coded bundle ifs in motor.
//!
//! Auth (ASWebAuthentication → SafariViewService) is one role. Share sheets, ATT prompts,
//! and future UIServices plug into the same table.

use serde::{Deserialize, Serialize};

use crate::control::Overlay;

/// Why a non-host AX tree is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SystemSurfaceRole {
    /// ASWebAuthenticationSession / SFSafariViewController / AuthKit / SSO.
    Auth,
    /// System share sheet / activity view.
    Share,
    /// Permission / consent prompts (ATT, notifications, local network, …).
    Permission,
    /// SpringBoard transient (control center, notification shade) — usually wait/recover.
    SpringBoardTransient,
    #[default]
    Other,
}

impl SystemSurfaceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Share => "share",
            Self::Permission => "permission",
            Self::SpringBoardTransient => "springboard_transient",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "auth" | "system_auth" => Self::Auth,
            "share" => Self::Share,
            "permission" => Self::Permission,
            "springboard_transient" => Self::SpringBoardTransient,
            _ => Self::Other,
        }
    }
}

/// Motor / clear_path behavior for a system surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceMotorPolicy {
    pub blocks_path: bool,
    pub prefer_ax: bool,
    /// Host may try dismiss gestures (swipe-down / Cancel).
    pub auto_dismiss: bool,
}

impl SystemSurfaceRole {
    pub fn motor_policy(self) -> SurfaceMotorPolicy {
        match self {
            Self::Auth => SurfaceMotorPolicy {
                blocks_path: true,
                prefer_ax: true,
                // Never swipe away OAuth — agent must interact inside.
                auto_dismiss: false,
            },
            Self::Share => SurfaceMotorPolicy {
                blocks_path: true,
                prefer_ax: true,
                auto_dismiss: true,
            },
            Self::Permission => SurfaceMotorPolicy {
                blocks_path: true,
                prefer_ax: true,
                // Prefer tapping Allow/Don't Allow via AX, not blind swipe.
                auto_dismiss: false,
            },
            Self::SpringBoardTransient => SurfaceMotorPolicy {
                blocks_path: true,
                prefer_ax: false,
                auto_dismiss: false,
            },
            Self::Other => SurfaceMotorPolicy {
                blocks_path: true,
                prefer_ax: true,
                auto_dismiss: false,
            },
        }
    }
}

/// Provenance stamped on observe when AX came from a foreign process.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SystemSurfaceInfo {
    pub role: SystemSurfaceRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
}

impl SystemSurfaceInfo {
    pub fn overlay(self) -> Overlay {
        Overlay::SystemSurface
    }

    pub fn motor_policy(&self) -> SurfaceMotorPolicy {
        self.role.motor_policy()
    }
}

/// Classify a guest process name or bundle id into a surface role.
///
/// Markers are a **classification catalog**, not the only discovery mechanism.
/// Discovery belongs to hit-test / alternate AX roots in the host bridge.
pub fn classify_system_process(name_or_bundle: &str) -> SystemSurfaceRole {
    let s = name_or_bundle.to_ascii_lowercase();
    if s.contains("safariview")
        || s.contains("authenticationservices")
        || s.contains("authkit")
        || s.contains("appsso")
        || s.contains("aswebauthentication")
    {
        return SystemSurfaceRole::Auth;
    }
    if s.contains("shareplay") || s.contains("sharing") || s.contains("activityview") {
        return SystemSurfaceRole::Share;
    }
    if s.contains("privacy")
        || s.contains("promptkit")
        || s.contains("usernotifications")
        || s.contains("springboardpermissions")
        || s.contains("localnetwork")
    {
        return SystemSurfaceRole::Permission;
    }
    if s.contains("springboard") || s.contains("cover-sheet") || s.contains("coversheet") {
        return SystemSurfaceRole::SpringBoardTransient;
    }
    SystemSurfaceRole::Other
}

/// Build info from AX dump fields (`ax_bundle`, `ax_process`, `ax_role`, `ax_pid`).
pub fn system_surface_from_ax_dump(
    ax_source: Option<&str>,
    ax_bundle: Option<&str>,
    ax_process: Option<&str>,
    ax_role: Option<&str>,
    ax_pid: Option<i32>,
) -> Option<SystemSurfaceInfo> {
    let source = ax_source.unwrap_or("");
    if source != "system_surface" && source != "system_auth" {
        return None;
    }
    let role = if let Some(r) = ax_role.filter(|r| !r.is_empty()) {
        SystemSurfaceRole::parse(r)
    } else {
        let hint = ax_bundle.or(ax_process).unwrap_or("other");
        classify_system_process(hint)
    };
    Some(SystemSurfaceInfo {
        role,
        bundle: ax_bundle.map(|s| s.to_string()),
        process_name: ax_process.map(|s| s.to_string()),
        pid: ax_pid,
    })
}

/// Policy helper used by motor / qa — works from overlay + optional role.
pub fn policy_for_overlay(overlay: Overlay, role: Option<SystemSurfaceRole>) -> SurfaceMotorPolicy {
    match overlay {
        Overlay::SystemSurface => role.unwrap_or(SystemSurfaceRole::Other).motor_policy(),
        Overlay::Sheet | Overlay::Alert => SurfaceMotorPolicy {
            blocks_path: true,
            prefer_ax: true,
            auto_dismiss: true,
        },
        Overlay::Keyboard => SurfaceMotorPolicy {
            blocks_path: true,
            prefer_ax: false,
            auto_dismiss: true,
        },
        Overlay::Transition => SurfaceMotorPolicy {
            blocks_path: true,
            prefer_ax: false,
            auto_dismiss: false,
        },
        Overlay::None => SurfaceMotorPolicy {
            blocks_path: false,
            prefer_ax: false,
            auto_dismiss: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_markers_classify() {
        assert_eq!(
            classify_system_process("com.apple.SafariViewService"),
            SystemSurfaceRole::Auth
        );
        assert_eq!(
            classify_system_process("AuthenticationServicesUI"),
            SystemSurfaceRole::Auth
        );
    }

    #[test]
    fn auth_never_auto_dismiss() {
        assert!(!SystemSurfaceRole::Auth.motor_policy().auto_dismiss);
        assert!(SystemSurfaceRole::Auth.motor_policy().prefer_ax);
    }

    #[test]
    fn dump_legacy_system_auth_source_still_maps() {
        let info = system_surface_from_ax_dump(
            Some("system_auth"),
            Some("com.apple.SafariViewService"),
            None,
            None,
            Some(42),
        )
        .unwrap();
        assert_eq!(info.role, SystemSurfaceRole::Auth);
        assert_eq!(info.pid, Some(42));
    }
}
