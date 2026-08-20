use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::device::DevicePreset;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LighConfig {
    pub default_device: DevicePreset,
    pub slim_on_boot: bool,
    pub boot_timeout_secs: u64,
    pub state_dir: PathBuf,
}

impl Default for LighConfig {
    fn default() -> Self {
        Self {
            default_device: DevicePreset::Iphone15Pro,
            slim_on_boot: true,
            boot_timeout_secs: 120,
            state_dir: default_state_dir(),
        }
    }
}

impl LighConfig {
    pub fn load() -> crate::Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let config: Self = toml::from_str(&raw).map_err(|e| crate::LighError::Config {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        Ok(config)
    }

    pub fn save(&self) -> crate::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self).map_err(|e| crate::LighError::Config {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        std::fs::write(path, raw)?;
        Ok(())
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ligh")
        .join("config.toml")
}

pub fn default_state_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ligh")
}
