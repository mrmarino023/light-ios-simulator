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
        /// Exact scene-graph id from observe v2.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    LongPress {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default = "default_true")]
        normalized: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    ScrollUntil {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_swipes: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Type {
        text: String,
    },
    Clear {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    Key {
        name: String,
    },
    Wait {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Exists {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Sense,
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
        /// Poll AX until settled (ready + actionable) or this budget elapses (ms).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Control plane: recover until Ready (home + settle) or fault.
    EnsureReady {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        /// Max Home presses while recovering (default 6).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recover_homes: Option<u32>,
    },
    /// Capability: open Settings (IT/EN) → assert surface=settings.
    OpenSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Capability: Settings search field → type query → settle.
    SettingsSearch {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Capability: settle then assert scene.surface.
    AssertSurface {
        surface: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Capability: settle → tap label/id → settle (act-with-settle).
    ActTap {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Capability: settle → type → settle.
    ActType {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Product path: install Debug `.app` → launch → settle → optional wait_label.
    RunApp {
        app: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        /// When false, skip simctl install (relaunch only). Default true.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<bool>,
        /// Extra argv passed to simctl launch (e.g. `--ui_test_login_failure`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_args: Option<Vec<String>>,
    },
    /// Capability: settle → wait until AX label exists.
    WaitLabel {
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Product job: install+launch then motor steps (wait/tap/type) as one capability.
    AppJob {
        app: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        steps: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_args: Option<Vec<String>>,
    },
    /// QA layer: settled world model for agents (affordances + fingerprint).
    Perceive {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// QA layer: act with built-in verify (tap/type/key + optional expect).
    Attempt {
        intent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// QA layer: find label/id, optionally scroll.
    Find {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scroll: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_swipes: Option<u32>,
    },
    /// QA layer: dismiss blocking overlay (keyboard/alert/sheet).
    Dismiss {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Host-owned reach: scroll + dismiss overlay + wait for id/label.
    Reach {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_swipes: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Clear keyboard/sheet/alert overlay if present (motor FSM).
    DismissOverlay {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// Host explore: reach → probe gestures → reach (human recovery).
    Explore {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_probes: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_swipes: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Goal job: setup steps + postconditions (declarative verify).
    AppGoal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        #[serde(default)]
        setup: Vec<serde_json::Value>,
        postconditions: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_args: Option<Vec<String>>,
    },
    /// Autopilot: host drives the UI to a goal, zero LLM tokens. Path is discovered.
    Autopilot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        /// Typed GoalSpec on the wire — never `Value`, so unknown predicate
        /// fields fail at the RPC boundary instead of deserializing to `{}`.
        goal: crate::PilotGoal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_steps: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        /// Absolute Unix deadline. Relative timeout remains for compatibility; the
        /// earliest deadline wins.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_unix_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_args: Option<Vec<String>>,
    },
    /// UX graph: status (nodes, edges, baselines).
    UxStatus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// UX graph: snapshot current screens as baseline.
    UxBaseline {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// UX graph: diff current screen vs baseline.
    UxRegress {
        baseline: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
    },
    /// UX graph: safe BFS explore (records transitions).
    UxExplore {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_steps: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_depth: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// UX graph: correlate fingerprint with source file edit.
    UxHint {
        fingerprint: String,
        source_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// UX graph: compile intent_met path to motor steps (`.ligh/compiled/{goal}.json`).
    UxCompileFlow {
        goal_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// UX graph: execute compiled flow — zero LLM, motor only.
    UxExecuteCompiled {
        goal_id: String,
        app: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settle_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    StreamStats,
    /// Tear down the simulator session and exit the daemon.
    Shutdown,
    /// Exit the daemon only — leave the guest booted (stream detaches).
    Quit,
}

impl DaemonRequest {
    /// Requests that can mutate simulator, foreground, keyboard, or planner state.
    /// The daemon holds one exclusive lease for the full request.
    pub fn requires_operation_lease(&self) -> bool {
        !matches!(
            self,
            Self::Ping
                | Self::Status
                | Self::Sense
                | Self::Screenshot { .. }
                | Self::FrameMeta
                | Self::Observe { .. }
                | Self::Exists { .. }
                | Self::Perceive { .. }
                | Self::UxStatus { .. }
                | Self::UxRegress { .. }
        )
    }
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

    pub fn fault(msg: impl std::fmt::Display, data: impl Serialize) -> Self {
        Self {
            ok: false,
            error: Some(msg.to_string()),
            data: Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null)),
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

/// Long-running capabilities (app-job, run-app) can exceed the default RPC read window.
fn read_timeout_for(req: &DaemonRequest) -> Duration {
    match req {
        DaemonRequest::AppJob {
            timeout_ms,
            steps,
            ..
        } => {
            let per = timeout_ms.unwrap_or(10_000);
            let n = steps.len().max(1) as u64;
            // install/launch + motor steps + settle slack
            Duration::from_millis(per.saturating_mul(n).saturating_add(90_000))
        }
        DaemonRequest::AppGoal {
            timeout_ms,
            setup,
            postconditions,
            ..
        } => {
            let per = timeout_ms.unwrap_or(12_000);
            let n = (setup.len() + postconditions.len()).max(1) as u64;
            Duration::from_millis(per.saturating_mul(n).saturating_add(120_000))
        }
        DaemonRequest::Reach { timeout_ms, .. } => {
            Duration::from_millis(timeout_ms.unwrap_or(12_000).saturating_add(30_000))
        }
        DaemonRequest::Autopilot {
            timeout_ms,
            max_steps,
            ..
        } => {
            let per = timeout_ms.unwrap_or(8_000);
            let n = max_steps.unwrap_or(24).max(1) as u64;
            Duration::from_millis(per.saturating_mul(n).saturating_add(120_000))
        }
        DaemonRequest::RunApp { timeout_ms, .. } => {
            Duration::from_millis(timeout_ms.unwrap_or(8_000).saturating_add(60_000))
        }
        DaemonRequest::UxExplore { timeout_ms, .. } => {
            Duration::from_millis(timeout_ms.unwrap_or(8_000).saturating_mul(6).saturating_add(60_000))
        }
        DaemonRequest::ActTap { timeout_ms, .. }
        | DaemonRequest::WaitLabel { timeout_ms, .. }
        | DaemonRequest::Attempt { timeout_ms, .. }
        | DaemonRequest::Find { timeout_ms, .. } => {
            Duration::from_millis(timeout_ms.unwrap_or(8_000).saturating_add(15_000))
        }
        DaemonRequest::ActType { .. } => Duration::from_secs(30),
        _ => Duration::from_secs(45),
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
            .set_read_timeout(Some(read_timeout_for(req)))
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
        self.observe_ax_settle(true, Some(2500))
    }

    pub fn observe_ax(&self, ax: bool) -> Result<serde_json::Value> {
        self.observe_ax_settle(ax, None)
    }

    pub fn observe_ax_settle(&self, ax: bool, settle_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Observe { ax, settle_ms })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("observe returned no data".into()))
    }

    pub fn ensure_ready(
        &self,
        settle_ms: Option<u64>,
        recover_homes: Option<u32>,
    ) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::EnsureReady {
            settle_ms,
            recover_homes,
        })?;
        if resp.ok {
            resp.into_result()?
                .ok_or_else(|| LighError::Simctl("ensure_ready returned no data".into()))
        } else {
            Err(LighError::Simctl(format!(
                "{} — {}",
                resp.error.unwrap_or_else(|| "ensure_ready failed".into()),
                resp.data
                    .map(|d| d.to_string())
                    .unwrap_or_default()
            )))
        }
    }

    pub fn open_settings(&self, settle_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::OpenSettings { settle_ms })?;
        if resp.ok {
            resp.into_result()?
                .ok_or_else(|| LighError::Simctl("open_settings returned no data".into()))
        } else {
            Err(LighError::NotReady(
                resp.error.unwrap_or_else(|| "open_settings failed".into()),
            ))
        }
    }

    pub fn settings_search(&self, query: &str, settle_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::SettingsSearch {
            query: query.to_string(),
            settle_ms,
        })?;
        if resp.ok {
            resp.into_result()?
                .ok_or_else(|| LighError::Simctl("settings_search returned no data".into()))
        } else {
            Err(LighError::NotReady(
                resp.error.unwrap_or_else(|| "settings_search failed".into()),
            ))
        }
    }

    pub fn assert_surface(&self, surface: &str, settle_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::AssertSurface {
            surface: surface.to_string(),
            settle_ms,
        })?;
        if resp.ok {
            resp.into_result()?
                .ok_or_else(|| LighError::Simctl("assert_surface returned no data".into()))
        } else {
            Err(LighError::NotReady(
                resp.error.unwrap_or_else(|| "assert_surface failed".into()),
            ))
        }
    }

    pub fn act_tap(
        &self,
        label: Option<&str>,
        id: Option<&str>,
        settle_ms: Option<u64>,
        timeout_ms: Option<u64>,
    ) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::ActTap {
            label: label.map(|s| s.to_string()),
            id: id.map(|s| s.to_string()),
            settle_ms,
            timeout_ms,
        })?;
        if resp.ok {
            resp.into_result()?
                .ok_or_else(|| LighError::Simctl("act_tap returned no data".into()))
        } else {
            Err(LighError::NotReady(
                resp.error.unwrap_or_else(|| "act_tap failed".into()),
            ))
        }
    }

    pub fn act_type(&self, text: &str, settle_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::ActType {
            text: text.to_string(),
            settle_ms,
        })?;
        if resp.ok {
            resp.into_result()?
                .ok_or_else(|| LighError::Simctl("act_type returned no data".into()))
        } else {
            Err(LighError::NotReady(
                resp.error.unwrap_or_else(|| "act_type failed".into()),
            ))
        }
    }

    pub fn tap(&self, x: f64, y: f64, normalized: bool) -> Result<()> {
        self.call(&DaemonRequest::Tap {
            x,
            y,
            normalized,
            label: None,
            id: None,
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
            id: None,
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("tap_label returned no data".into()))
    }

    pub fn tap_id(&self, id: &str, timeout_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Tap {
            x: 0.0,
            y: 0.0,
            normalized: true,
            label: None,
            id: Some(id.to_string()),
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("tap_id returned no data".into()))
    }

    pub fn long_press_label(
        &self,
        label: &str,
        hold_ms: Option<u64>,
        timeout_ms: Option<u64>,
    ) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::LongPress {
            x: 0.0,
            y: 0.0,
            normalized: true,
            label: Some(label.to_string()),
            id: None,
            hold_ms,
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("long_press returned no data".into()))
    }

    pub fn scroll_until(
        &self,
        label: Option<&str>,
        id: Option<&str>,
        max_swipes: Option<u32>,
        timeout_ms: Option<u64>,
    ) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::ScrollUntil {
            label: label.map(|s| s.to_string()),
            id: id.map(|s| s.to_string()),
            max_swipes,
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("scroll_until returned no data".into()))
    }

    pub fn wait_label(&self, label: &str, timeout_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Wait {
            label: Some(label.to_string()),
            id: None,
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("wait returned no data".into()))
    }

    pub fn wait_id(&self, id: &str, timeout_ms: Option<u64>) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Wait {
            label: None,
            id: Some(id.to_string()),
            timeout_ms,
        })?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("wait id returned no data".into()))
    }

    pub fn exists_label(&self, label: &str) -> Result<bool> {
        let resp = self.call(&DaemonRequest::Exists {
            label: Some(label.to_string()),
            id: None,
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

    pub fn clear(&self, count: Option<u32>) -> Result<()> {
        self.call(&DaemonRequest::Clear { count })?.into_result()?;
        Ok(())
    }

    pub fn key(&self, name: &str) -> Result<()> {
        self.call(&DaemonRequest::Key {
            name: name.to_string(),
        })?
        .into_result()?;
        Ok(())
    }

    pub fn sense(&self) -> Result<serde_json::Value> {
        let resp = self.call(&DaemonRequest::Sense)?;
        resp.into_result()?
            .ok_or_else(|| LighError::Simctl("sense returned no data".into()))
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
            id: None,
            timeout_ms: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"normalized\":true"));
        assert!(s.contains("\"cmd\":\"tap\"") || s.contains("\"cmd\": \"tap\""));
    }
}
