use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::device::DevicePreset;
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub udid: String,
    pub device: DevicePreset,
    pub name: String,
    pub slim_applied: bool,
    pub app_bundle_id: Option<String>,
    pub app_path: Option<PathBuf>,
}

impl SessionState {
    pub fn path(base: &Path) -> PathBuf {
        base.join("session.json")
    }

    pub fn load(base: &Path) -> Result<Option<Self>> {
        let path = Self::path(base);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    pub fn save(&self, base: &Path) -> Result<()> {
        std::fs::create_dir_all(base)?;
        std::fs::write(Self::path(base), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn clear(base: &Path) -> Result<()> {
        let path = Self::path(base);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
