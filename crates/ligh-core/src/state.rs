use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::device::DevicePreset;
use crate::error::Result;

fn state_schema() -> u32 {
    2
}

fn new_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default = "state_schema")]
    pub schema_version: u32,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub boot_epoch: u64,
    #[serde(default)]
    pub launch_epoch: u64,
    pub udid: String,
    pub device: DevicePreset,
    pub name: String,
    pub slim_applied: bool,
    pub app_bundle_id: Option<String>,
    pub app_path: Option<PathBuf>,
}

impl SessionState {
    pub fn fresh(
        udid: String,
        device: DevicePreset,
        name: String,
        slim_applied: bool,
    ) -> Self {
        let epoch = new_epoch();
        Self {
            schema_version: state_schema(),
            session_id: format!("session-{epoch:016x}"),
            boot_epoch: epoch,
            launch_epoch: 0,
            udid,
            device,
            name,
            slim_applied,
            app_bundle_id: None,
            app_path: None,
        }
    }

    /// Starts a new app-ownership epoch. Every target resolved before this call is stale.
    pub fn begin_launch(&mut self, bundle_id: String, app_path: Option<PathBuf>) {
        self.launch_epoch = self.launch_epoch.saturating_add(1).max(1);
        self.app_bundle_id = Some(bundle_id);
        self.app_path = app_path;
    }

    /// Backfill epochs for state files written by pre-v2 clients.
    pub fn ensure_contract(&mut self) {
        if self.schema_version == 0 {
            self.schema_version = state_schema();
        }
        if self.boot_epoch == 0 {
            self.boot_epoch = new_epoch();
        }
        if self.session_id.is_empty() {
            self.session_id = format!("session-{:016x}", self.boot_epoch);
        }
    }

    pub fn path(base: &Path) -> PathBuf {
        base.join("session.json")
    }

    pub fn load(base: &Path) -> Result<Option<Self>> {
        let path = Self::path(base);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)?;
        let mut state: Self = serde_json::from_str(&raw)?;
        state.ensure_contract();
        Ok(Some(state))
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
