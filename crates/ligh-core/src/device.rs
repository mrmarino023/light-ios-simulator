use std::fmt;

use serde::{Deserialize, Serialize};

/// Well-known simulator device presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DevicePreset {
    IphoneSe,
    #[default]
    Iphone15Pro,
    Iphone15ProMax,
    IpadPro11,
}

impl DevicePreset {
    pub fn simctl_name(self) -> &'static str {
        match self {
            Self::IphoneSe => "iPhone SE (3rd generation)",
            Self::Iphone15Pro => "iPhone 15 Pro",
            Self::Iphone15ProMax => "iPhone 15 Pro Max",
            Self::IpadPro11 => "iPad Pro (11-inch) (4th generation)",
        }
    }

    /// Logical screen size in points (IndigoHID / UIKit). Not framebuffer pixels.
    pub fn logical_size(self) -> (f64, f64) {
        match self {
            Self::IphoneSe => (375.0, 667.0),
            Self::Iphone15Pro => (393.0, 852.0),
            Self::Iphone15ProMax => (430.0, 932.0),
            Self::IpadPro11 => (834.0, 1194.0),
        }
    }

    pub fn native_scale(self) -> f64 {
        match self {
            Self::IphoneSe | Self::IpadPro11 => 2.0,
            Self::Iphone15Pro | Self::Iphone15ProMax => 3.0,
        }
    }

    pub fn is_tablet(self) -> bool {
        matches!(self, Self::IpadPro11)
    }

    /// Convert an IOSurface pixel size into HID points.
    pub fn hid_size_from_framebuffer(self, px_w: u32, px_h: u32) -> (f64, f64) {
        let s = self.native_scale();
        if px_w > 0 && px_h > 0 {
            (px_w as f64 / s, px_h as f64 / s)
        } else {
            self.logical_size()
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::IphoneSe,
            Self::Iphone15Pro,
            Self::Iphone15ProMax,
            Self::IpadPro11,
        ]
    }
}

impl fmt::Display for DevicePreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::IphoneSe => "iphone-se",
            Self::Iphone15Pro => "iphone-15-pro",
            Self::Iphone15ProMax => "iphone-15-pro-max",
            Self::IpadPro11 => "ipad-pro-11",
        })
    }
}

impl std::str::FromStr for DevicePreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "iphone-se" | "iphonese" => Ok(Self::IphoneSe),
            "iphone-15-pro" | "iphone15pro" => Ok(Self::Iphone15Pro),
            "iphone-15-pro-max" | "iphone15promax" => Ok(Self::Iphone15ProMax),
            "ipad-pro-11" | "ipadpro11" => Ok(Self::IpadPro11),
            other => Err(format!(
                "unknown device preset '{other}'; try iphone-15-pro, iphone-se, iphone-15-pro-max, ipad-pro-11"
            )),
        }
    }
}
