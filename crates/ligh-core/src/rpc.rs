//! Shared JSON-lines RPC for `lighd` — the agent hot path.
//!
//! Socket: `~/.ligh/lighd.sock`
//! One request object per line → one response object per line.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{LighError, Result};

/// Default daemon socket path.
pub fn default_sock_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ligh")
        .join("lighd.sock")
}

/// Client → daemon request (one JSON line).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    Ping,
    Status,
    Boot {
        #[serde(skip_serializing_if = "Option::is_none")]
        device: Option<String>,
    },
    Install {
        app: String,
    },
    Launch {
        bundle_id: String,
    },
    Tap {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// `true` → coordinates are 0..1 (normalized).
        #[serde(default = "default_true")]
        normalized: bool,
        /// If set, resolve via AX (waits up to `timeout_ms`) then tap center.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Type {
        text: String,
    },
    Wait {
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Exists {
        label: String,
    },
    Swipe {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        #[serde(default = "default_true")]
        normalized: bool,
    },
    Home,
    Screenshot {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    FrameMeta,
    Observe {
        /// Dump accessibility tree. Frame-only observe is much cheaper.
        #[serde(default = "default_true")]
        ax: bool,
    },
    StreamStats,
    /// Tear down the simulator session and exit the daemon.
    Shutdown,
    /// Exit the daemon only — leave the guest booted (stream detaches).
    Quit,
}

fn default_true() -> bool {
    true
}

/// Daemon → client response (one JSON line).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl DaemonResponse {
    pub fn ok(data: impl Serialize) -> Self {
        Self {
            ok: true,
            error: None,
            data: Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null)),
        }
    }

    pub fn ok_empty() -> Self {
        Self {
            ok: true,
            error: None,
            data: None,
        }
    }

    pub fn err(msg: impl std::fmt::Display) -> Self {
        Self {
            ok: false,
            error: Some(msg.to_string()),
            data: None,
        }
    }

    pub fn into_result(self) -> Result<Option<serde_json::Value>> {
        if self.ok {
            Ok(self.data)
        } else {
            Err(LighError::Simctl(
                self.error.unwrap_or_else(|| "lighd error".into()),
            ))
        }
    }
}

/// Thin Unix-socket client. Prefer this over cold `HostSession` per CLI invoke.
pub struct DaemonClient {
    sock: PathBuf,
}

impl DaemonClient {
    pub fn new(sock: impl Into<PathBuf>) -> Self {
        Self { sock: sock.into() }
    }

    pub fn default_sock() -> Self {
        Self::new(default_sock_path())
    }

    pub fn sock_path(&self) -> &Path {
        &self.sock
    }

    pub fn is_alive(&self) -> bool {
        self.call(&DaemonRequest::Ping).is_ok()
    }

    /// Connect, send one request, read one response, disconnect.
    /// Persistent connection can come later; even reconnect-per-call beats
    /// reloading private frameworks + IOSurface for every `ligh observe`.
    pub fn call(&self, req: &DaemonRequest) -> Result<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.sock).map_err(|e| {
            LighError::NotReady(format!(
                "lighd not reachable at {}: {e} — run `lighd` or `ligh daemon start`",
                self.sock.display()
            ))
        })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(45)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .ok();

        let mut line = serde_json::to_string(req).map_err(|e| LighError::Simctl(e.to_string()))?;
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .map_err(LighError::Io)?;
        stream.flush().map_err(LighError::Io)?;

        let mut reader = BufReader::new(stream);
        let mut resp_line = String::new();
        reader.read_line(&mut resp_line).map_err(LighError::Io)?;
        if resp_line.trim().is_empty() {
            return Err(LighError::NotReady("lighd closed connection".into()));
        }
        serde_json::from_str(resp_line.trim()).map_err(|e| LighError::Simctl(e.to_string()))
    }

    pub fn observe(&self) -> Result<serde_json::Value> {
        self.observe_ax(true)
    }

    pub fn observe_ax(&self, ax: bool) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Observe { ax })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("observe returned no data".into()))
    }

    pub fn tap(&self, x: f64, y: f64, normalized: bool) -> Result<()> {
        self.call(&DaemonRequest::Tap {
            x,
            y,
            normalized,
            label: None,
            timeout_ms: None,
        })?
        .into_result()?;
        Ok(())
    }

    pub fn tap_label(&self, label: &str, timeout_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Tap {
            x: 0.0,
            y: 0.0,
            normalized: true,
            label: Some(label.to_string()),
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("tap_label returned no data".into()))
    }

    pub fn wait_label(&self, label: &str, timeout_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Wait {
            label: label.to_string(),
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("wait returned no data".into()))
    }

    pub fn exists_label(&self, label: &str) -> Result<bool> {
        let resp = self.call(&DaemonRequest::Exists {
            label: label.to_string(),
        })?;
        let data = resp
            .into_result()?
            .ok_or_else(|| LighError::Simctl("exists returned no data".into()))?;
        Ok(data.get("found").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    pub fn type_text(&self, text: &str) -> Result<()> {
        self.call(&DaemonRequest::Type {
            text: text.to_string(),
        })?
        .into_result()?;
        Ok(())
    }

    pub fn swipe(
        &self,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        normalized: bool,
    ) -> Result<()> {
        self.call(&DaemonRequest::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            normalized,
        })?
        .into_result()?;
        Ok(())
    }

    pub fn home(&self) -> Result<()> {
        self.call(&DaemonRequest::Home)?.into_result()?;
        Ok(())
    }

    pub fn screenshot(&self, path: impl Into<String>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Screenshot {
            path: Some(path.into()),
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("screenshot returned no data".into()))
    }

    pub fn boot(&self, device: Option<String>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Boot { device })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("boot returned no data".into()))
    }

    pub fn stream_stats(&self) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::StreamStats)?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("stream_stats returned no data".into()))
    }
}

/// Ensure `lighd` is running. Spawns it from the sibling binary next to `ligh` if needed.
pub fn ensure_daemon(sock: &Path, lighd_bin: &Path) -> Result<DaemonClient> {
    let client = DaemonClient::new(sock);
    if client.is_alive() {
        return Ok(client);
    }

    if !lighd_bin.exists() {
        return Err(LighError::NotReady(format!(
            "lighd binary not found at {} — build with `cargo build --release`",
            lighd_bin.display()
        )));
    }

    if let Some(parent) = sock.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(sock);

    Command::new(lighd_bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| LighError::Io(e))?;

    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if client.is_alive() {
            return Ok(client);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(LighError::NotReady(
        "timed out waiting for lighd to accept connections".into(),
    ))
}

/// Resolve `lighd` next to the current executable.
pub fn sibling_lighd() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| {
            let mut d = p;
            d.set_file_name("lighd");
            Some(d)
        })
        .unwrap_or_else(|| PathBuf::from("lighd"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_request_serializes_normalized_true() {
        let r = DaemonRequest::Tap {
            x: 0.5,
            y: 0.5,
            normalized: true,
            label: None,
            timeout_ms: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"normalized\":true"));
        assert!(s.contains("\"cmd\":\"tap\"") || s.contains("\"cmd\": \"tap\""));
    }
}
