use ligh_core::{DevicePreset, LighError};
use serde::{Deserialize, Serialize};

use crate::simctl::Simctl;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    pub state: String,
    pub data_path: Option<String>,
    pub data_path_size: Option<u64>,
}

pub struct DeviceManager;

impl DeviceManager {
    pub fn ligh_name(preset: DevicePreset) -> String {
        format!("LIGH-{preset}")
    }

    /// Devices created by LIGH for this preset (`LIGH-iphone-15-pro`, …).
    pub fn list_matching(preset: DevicePreset) -> Result<Vec<SimDevice>, LighError> {
        let ligh_name = Self::ligh_name(preset);
        let json = Simctl::run_ok(&["list", "devices", "available", "-j"])?;
        let value: serde_json::Value = serde_json::from_str(&json)?;

        let mut devices = Vec::new();
        if let Some(map) = value.get("devices").and_then(|d| d.as_object()) {
            for list in map.values() {
                for entry in list.as_array().into_iter().flatten() {
                    let name = entry.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name != ligh_name {
                        continue;
                    }
                    let device: SimDevice = serde_json::from_value(entry.clone())?;
                    devices.push(device);
                }
            }
        }
        Ok(devices)
    }

    /// Pick best device: explicit udid > session > booted LIGH-* > existing LIGH-* > create.
    pub fn resolve(
        preset: DevicePreset,
        explicit_udid: Option<&str>,
        session_udid: Option<&str>,
    ) -> Result<SimDevice, LighError> {
        if let Some(udid) = explicit_udid {
            return Self::get(udid);
        }
        if let Some(udid) = session_udid {
            if let Ok(d) = Self::get(udid) {
                return Ok(d);
            }
        }

        let mut matches = Self::list_matching(preset)?;

        if let Some(d) = matches.iter().find(|d| d.state == "Booted").cloned() {
            return Ok(d);
        }

        matches.sort_by_key(|d| d.data_path_size.unwrap_or(u64::MAX));
        if let Some(d) = matches.into_iter().next() {
            return Ok(d);
        }

        Self::create(preset)
    }

    pub fn create(preset: DevicePreset) -> Result<SimDevice, LighError> {
        let runtime = Self::latest_ios_runtime()?;
        let device_type = Self::device_type_id(preset)?;
        let name = Self::ligh_name(preset);
        let udid = Simctl::run_ok(&["create", &name, &device_type, &runtime])?;
        Self::get(&udid)
    }

    pub fn get(udid: &str) -> Result<SimDevice, LighError> {
        let json = Simctl::run_ok(&["list", "devices", "-j"])?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        if let Some(map) = value.get("devices").and_then(|d| d.as_object()) {
            for list in map.values() {
                for entry in list.as_array().into_iter().flatten() {
                    if entry.get("udid").and_then(|u| u.as_str()) == Some(udid) {
                        return Ok(serde_json::from_value(entry.clone())?);
                    }
                }
            }
        }
        Err(LighError::DeviceNotFound(udid.to_string()))
    }

    fn latest_ios_runtime() -> Result<String, LighError> {
        let json = Simctl::run_ok(&["list", "runtimes", "-j"])?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        let runtimes = value
            .get("runtimes")
            .and_then(|r| r.as_array())
            .into_iter()
            .flatten();

        let mut ids: Vec<String> = runtimes
            .filter(|rt| {
                rt.get("platform").and_then(|p| p.as_str()) == Some("iOS")
                    && rt.get("isAvailable").and_then(|a| a.as_bool()).unwrap_or(true)
            })
            .filter_map(|rt| rt.get("identifier").and_then(|id| id.as_str()))
            .map(str::to_string)
            .collect();

        ids.sort();
        ids.pop().ok_or_else(|| {
            LighError::Doctor("no iOS runtime installed — open Xcode → Settings → Platforms".into())
        })
    }

    fn device_type_id(preset: DevicePreset) -> Result<String, LighError> {
        let target = preset.simctl_name();
        let json = Simctl::run_ok(&["list", "devicetypes", "-j"])?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        let types = value
            .get("devicetypes")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
            .chain(value.as_array().into_iter().flatten());

        for dt in types {
            if dt.get("name").and_then(|n| n.as_str()) == Some(target) {
                if let Some(id) = dt.get("identifier").and_then(|i| i.as_str()) {
                    return Ok(id.to_string());
                }
            }
        }
        Err(LighError::Doctor(format!("device type '{target}' not found")))
    }
}
