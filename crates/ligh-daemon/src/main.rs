//! `lighd` v3 — persistent daemon: IOSurface stream + Metal compositor + Unix socket RPC.
//!
//! Socket: `~/.ligh/lighd.sock`  (JSON-lines protocol, see ARCHITECTURE.md)

mod capabilities;
mod cognition;
mod device_hub;
mod fault_injection;
mod hybrid_physical;
mod motor;
mod pilot_cap;
mod qa_cap;
mod ux_cap;
mod wda;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    default_sock_path, diff_sense_events, parse_expectation, AccessibilityTree, DaemonRequest,
    DaemonResponse, DevicePreset, FeatureRequirements, FrameMeta, LighConfig, ObserveSnapshot,
    SenseEvent, SessionState,
};
use ligh_gpu::{HeadlessCompositor, Screenshot};
use ligh_host::{AxDump, HidInput, HostSession};
use ligh_sim::{ensure_headless, SimSupervisor};
use tracing::{info, warn};

// ─────────────────────────── Daemon state ────────────────────────────────────

pub(crate) struct DaemonState {
    pub(crate) compositor: Arc<HeadlessCompositor>,
    /// Width/height in points from last stream attach.
    pub(crate) sim_width: f64,
    pub(crate) sim_height: f64,
    pub(crate) udid: Option<String>,
    pub(crate) session_id: String,
    pub(crate) boot_epoch: u64,
    pub(crate) launch_epoch: u64,
    pub(crate) screen_epoch: u64,
    pub(crate) stability_streak: u32,
    pub(crate) expected_bundle_id: Option<String>,
    /// Serializes stateful simulator operations. Clone before locking; never hold DaemonState too.
    pub(crate) operation_lease: Arc<Mutex<()>>,
    pub(crate) last_screen_fingerprint: Option<String>,
    /// Previous AX flat nodes for sensation diff.
    pub(crate) last_ax_nodes: Option<Vec<serde_json::Value>>,
    /// Recent sense events (ring, newest last).
    pub(crate) sense_buf: Vec<SenseEvent>,
}

impl DaemonState {
    pub(crate) fn current_udid(&self) -> Result<String, String> {
        if ligh_host::physical_ui_active() {
            if let Some(ui) = ligh_host::physical_ui() {
                return Ok(ui.session_id());
            }
        }
        let cfg = LighConfig::load().map_err(|e| e.to_string())?;
        if let Some(u) = &self.udid {
            return Ok(u.clone());
        }
        SessionState::load(&cfg.state_dir)
            .map_err(|e| e.to_string())?
            .map(|s| s.udid)
            .ok_or_else(|| "no session — run `lighd` boot or `ligh up` first".into())
    }

    pub(crate) fn push_action_result(&mut self, ok: bool, kind: &str, detail: serde_json::Value) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.sense_buf.push(SenseEvent {
            t: now,
            kind: "action_result".into(),
            payload: Some(serde_json::json!({
                "ok": ok,
                "action": kind,
                "detail": detail,
            })),
        });
        if self.sense_buf.len() > 64 {
            let drop_n = self.sense_buf.len() - 64;
            self.sense_buf.drain(0..drop_n);
        }
    }
}

fn build_observe_once(state: &Arc<Mutex<DaemonState>>, include_ax: bool) -> ObserveSnapshot {
    HostSession::poll_stream();
    if let Some(ui) = ligh_host::physical_ui() {
        if ui.active() {
            if let Some((w, h)) = ui.screen_points() {
                let mut st = state.lock().unwrap();
                st.sim_width = w.max(1.0);
                st.sim_height = h.max(1.0);
                if let Some(bid) = ui.bundle_id() {
                    st.expected_bundle_id = Some(bid);
                }
            }
        }
    }
    let (udid, gpu) = {
        let st = state.lock().unwrap();
        let udid = st.udid.clone().unwrap_or_default();
        let gpu = st.compositor.stats();
        (udid, gpu)
    };
    let booted = !udid.is_empty() || ligh_host::physical_ui_active();
    let persisted = LighConfig::load()
        .ok()
        .and_then(|c| SessionState::load(&c.state_dir).ok().flatten());
    let app_bundle_id = ligh_host::physical_ui()
        .and_then(|u| u.bundle_id())
        .or_else(|| persisted.as_ref().and_then(|s| s.app_bundle_id.clone()));
    let (session_id, boot_epoch, launch_epoch, screen_epoch, expected_bundle_id) = {
        let st = state.lock().unwrap();
        (
            Some(st.session_id.clone()).filter(|s| !s.is_empty()),
            st.boot_epoch,
            st.launch_epoch,
            st.screen_epoch,
            if ligh_host::physical_ui_active() {
                app_bundle_id.clone().or(st.expected_bundle_id.clone())
            } else {
                st.expected_bundle_id.clone().or_else(|| app_bundle_id.clone())
            },
        )
    };
    let frame = if gpu.imports_ok > 0 {
        Some(FrameMeta {
            width: gpu.last_width,
            height: gpu.last_height,
            id: gpu.imports_ok,
            fps: gpu.fps,
            imports_ok: true,
        })
    } else if let Some((w, h)) = ligh_host::physical_ui().and_then(|u| u.screen_points()) {
        Some(FrameMeta {
            width: w as u32,
            height: h as u32,
            id: 1,
            fps: 0.0,
            imports_ok: true,
        })
    } else {
        None
    };
    let ax = if include_ax && (!udid.is_empty() || ligh_host::physical_ui_active()) {
        let dump_id = if udid.is_empty() { "device" } else { udid.as_str() };
        match AxDump::dump(dump_id) {
            Ok(v) => AccessibilityTree::from_ax_dump(v),
            Err(e) => AccessibilityTree::Error {
                message: e.to_string(),
            },
        }
    } else {
        AccessibilityTree::Empty
    };
    let mut snap = ObserveSnapshot {
        schema_version: ligh_core::OBSERVE_SCHEMA_VERSION,
        udid,
        session_id,
        boot_epoch,
        launch_epoch,
        screen_epoch,
        stability_streak: 0,
        motion_score: None,
        expected_bundle_id,
        observed_app_label: None,
        booted,
        simulator_app_running: false,
        frame,
        app_bundle_id,
        accessibility_tree: ax,
        scene: None,
        actionable_topk: vec![],
        events: vec![],
        ax_quality: "empty".into(),
        settled: false,
        observe_ms: None,
        path: Some("lighd".into()),
        phase: None,
        eyes_unusable: false,
        overlay: None,
        screen_sig: None,
    };
    snap.enrich_v2();
    snap.observed_app_label = ligh_core::foreground_app_label(snap.accessibility_tree.nodes());
    if snap.observed_app_label.is_none() && ligh_host::physical_ui_active() {
        snap.observed_app_label = snap.app_bundle_id.clone();
    }
    let fp = ligh_core::screen_fingerprint(snap.accessibility_tree.nodes());
    let mut st = state.lock().unwrap();
    if st.last_screen_fingerprint.as_deref() != Some(fp.as_str()) {
        st.screen_epoch = st.screen_epoch.saturating_add(1).max(1);
        st.last_screen_fingerprint = Some(fp);
        st.stability_streak = 1;
    } else {
        st.stability_streak = st.stability_streak.saturating_add(1);
    }
    snap.screen_epoch = st.screen_epoch;
    snap.stability_streak = st.stability_streak;
    drop(st);
    fault_injection::apply(&mut snap);
    snap
}

fn attach_sense(state: &Arc<Mutex<DaemonState>>, snap: &mut ObserveSnapshot, t0: Instant) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    snap.observe_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
    let mut st = state.lock().unwrap();
    let curr = snap.accessibility_tree.nodes().to_vec();
    let mut ev = diff_sense_events(st.last_ax_nodes.as_deref(), &curr, now);
    ev.extend(st.sense_buf.iter().cloned());
    st.sense_buf.clear();
    st.last_ax_nodes = Some(curr);
    snap.events = ev;
}

fn cap_response(mut r: ligh_core::CapabilityResult) -> DaemonResponse {
    if let Some(ref mut obs) = r.observe {
        // ensure control stamps present
        let has = !obs.udid.is_empty();
        ligh_core::stamp_control_fields(obs, has);
    }
    if r.ok {
        DaemonResponse::ok(r)
    } else {
        DaemonResponse::fault(
            format!("{}:{}", r.fault.as_str(), r.capability.as_deref().unwrap_or("cap")),
            r,
        )
    }
}

// ─────────────────────────── Socket helpers ──────────────────────────────────

fn sock_path() -> PathBuf {
    default_sock_path()
}

fn handle_client(stream: UnixStream, state: Arc<Mutex<DaemonState>>) {
    let reader = BufReader::new(stream.try_clone().expect("stream clone"));
    let mut writer = stream;
    for line in reader.lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            _ => break,
        };
        let resp = dispatch(&line, &state);
        let json = serde_json::to_string(&resp).unwrap_or_else(|_| r#"{"ok":false}"#.into());
        let _ = writer.write_all(json.as_bytes());
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

fn dispatch(line: &str, state: &Arc<Mutex<DaemonState>>) -> DaemonResponse {
    let req: DaemonRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return DaemonResponse::err(format!("parse error: {e}")),
    };
    // Serialize stateful simulator operations. The guard lives through the whole
    // dispatch, including Autopilot's observe/plan/act/verify transaction.
    let operation_lease = if req.requires_operation_lease() {
        Some(state.lock().unwrap().operation_lease.clone())
    } else {
        None
    };
    let _operation_guard = operation_lease
        .as_ref()
        .map(|lease| lease.lock().unwrap_or_else(|e| e.into_inner()));

    match req {
        DaemonRequest::Ping => DaemonResponse::ok(serde_json::json!({ "pong": true })),

        DaemonRequest::Status => {
            let cfg = match LighConfig::load() {
                Ok(c) => c,
                Err(e) => return DaemonResponse::err(e),
            };
            let session = SessionState::load(&cfg.state_dir).ok().flatten();
            let booted = session
                .as_ref()
                .and_then(|s| ligh_sim::Simctl::is_booted(&s.udid).ok())
                .unwrap_or(false);
            let st = state.lock().unwrap();
            let gpu = st.compositor.stats();
            let physical = ligh_host::physical_ui().and_then(|ui| {
                ui.active().then(|| {
                    serde_json::json!({
                        "connected": true,
                        "bundle_id": ui.bundle_id(),
                        "transport": ui.transport(),
                        "session_id": ui.session_id(),
                        "screen": ui.screen_points().map(|(w, h)| serde_json::json!({"width": w, "height": h})),
                        "driver_version": ui.driver_version(),
                        "capabilities": ui.capabilities(),
                    })
                })
            });
            DaemonResponse::ok(serde_json::json!({
                "udid": session.as_ref().map(|s| &s.udid),
                "booted": booted,
                "simulator_app_running": false,
                "ui_mode": ligh_host::ui_mode().as_str(),
                "target": ligh_host::ui_target(),
                "physical": physical,
                "frame": {
                    "w": gpu.last_width,
                    "h": gpu.last_height,
                    "id": gpu.imports_ok,
                    "fps": gpu.fps,
                    "imports_ok": gpu.imports_ok > 0,
                },
                "app_bundle_id": session.and_then(|s| s.app_bundle_id),
                "transaction": {
                    "session_id": st.session_id,
                    "boot_epoch": st.boot_epoch,
                    "launch_epoch": st.launch_epoch,
                    "screen_epoch": st.screen_epoch,
                    "expected_bundle_id": st.expected_bundle_id,
                },
            }))
        }

        DaemonRequest::Boot { device } => {
            let preset = device
                .as_deref()
                .and_then(|d| d.parse::<DevicePreset>().ok())
                .unwrap_or(DevicePreset::Iphone15Pro);
            let cfg = match LighConfig::load() {
                Ok(c) => c,
                Err(e) => return DaemonResponse::err(e),
            };
            ensure_headless();
            let sup = SimSupervisor::new(cfg).with_requirements(FeatureRequirements::default());
            match sup.up(preset, true, None) {
                Ok(session) => {
                    let udid = session.udid.clone();
                    // Attach stream
                    let comp = {
                        let st = state.lock().unwrap();
                        st.compositor.clone()
                    };
                    let comp2 = comp.clone();
                    HostSession::set_frame_handler(move |id, w, h| comp2.ingest(id, w, h));
                    match HostSession::stream_start(&udid) {
                        Ok(_host) => {
                            let stats = comp.stats();
                            let mut st = state.lock().unwrap();
                            st.udid = Some(udid.clone());
                            st.session_id = session.session_id.clone();
                            st.boot_epoch = session.boot_epoch;
                            st.launch_epoch = session.launch_epoch;
                            st.expected_bundle_id = session.app_bundle_id.clone();
                            st.screen_epoch = 0;
                            st.stability_streak = 0;
                            st.last_screen_fingerprint = None;
                            let (pw, ph) = preset.hid_size_from_framebuffer(
                                stats.last_width.max(1),
                                stats.last_height.max(1),
                            );
                            st.sim_width = pw;
                            st.sim_height = ph;
                            // Keep _host alive by leaking into state (stream lives for daemon lifetime)
                            std::mem::forget(_host);
                            drop(st);
                            if let Err(e) = HidInput::prepare(&udid) {
                                warn!(error=%e, "hid prepare after boot");
                            }
                        }
                        Err(e) => warn!(error=%e, "stream_start after boot"),
                    }
                    DaemonResponse::ok(serde_json::json!({ "udid": udid }))
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Install { app } => {
            let cfg = match LighConfig::load() {
                Ok(c) => c,
                Err(e) => return DaemonResponse::err(e),
            };
            let session = match SessionState::load(&cfg.state_dir) {
                Ok(Some(s)) => s,
                Ok(None) => return DaemonResponse::err("no session — boot first"),
                Err(e) => return DaemonResponse::err(e),
            };
            let app_path = match std::path::Path::new(&app).canonicalize() {
                Ok(p) => p,
                Err(e) => return DaemonResponse::err(format!("invalid app: {e}")),
            };
            match ligh_sim::Simctl::run(&[
                "install",
                &session.udid,
                app_path.to_str().unwrap_or(""),
            ]) {
                Ok(_) => {
                    // Detect + persist bundle id for later launch
                    let bundle_id = std::process::Command::new("plutil")
                        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
                        .arg(app_path.join("Info.plist"))
                        .output()
                        .ok()
                        .and_then(|o| {
                            if o.status.success() {
                                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                            } else {
                                None
                            }
                        });
                    if let Some(ref bid) = bundle_id {
                        let mut s = session.clone();
                        s.begin_launch(bid.clone(), Some(app_path.clone()));
                        let _ = s.save(&cfg.state_dir);
                        let mut st = state.lock().unwrap();
                        st.launch_epoch = s.launch_epoch;
                        st.expected_bundle_id = Some(bid.clone());
                        st.screen_epoch = st.screen_epoch.saturating_add(1);
                        st.stability_streak = 0;
                        st.last_screen_fingerprint = None;
                    }
                    DaemonResponse::ok(serde_json::json!({
                        "udid": session.udid,
                        "bundle_id": bundle_id,
                        "app": app_path.display().to_string(),
                    }))
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Launch { bundle_id } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            match ligh_sim::Simctl::run(&["launch", &udid, &bundle_id, "--terminate-running-process"]) {
                Ok(_) => {
                    // Persist bundle id
                    if let Ok(cfg) = LighConfig::load() {
                        if let Ok(Some(mut s)) = SessionState::load(&cfg.state_dir) {
                            s.begin_launch(bundle_id.clone(), s.app_path.clone());
                            let _ = s.save(&cfg.state_dir);
                            let mut st = state.lock().unwrap();
                            st.launch_epoch = s.launch_epoch;
                            st.expected_bundle_id = Some(bundle_id.clone());
                            st.screen_epoch = st.screen_epoch.saturating_add(1);
                            st.stability_streak = 0;
                            st.last_screen_fingerprint = None;
                        }
                    }
                    DaemonResponse::ok(serde_json::json!({ "bundle_id": bundle_id }))
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Tap {
            x,
            y,
            normalized,
            label,
            id,
            timeout_ms,
        } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            let w = st.sim_width;
            let h = st.sim_height;
            drop(st);
            let before_sig = if ligh_host::physical_ui_active() {
                build_observe_once(&state, true)
                    .screen_sig
                    .unwrap_or_default()
            } else {
                String::new()
            };
            // Label/id taps: AX activate first (in-app DevDriver or Simulator AX).
            // Coordinate HID on RN often hits a glyph child and never fires onPress.
            // On physical, always go through HidInput (WDA arms) + effect check — never
            // trust DevDriver press ACK alone.
            if !ligh_host::physical_ui_active() {
            if let Some(ref lab) = label {
                if AxDump::press_label(&udid, lab).is_ok() {
                    let detail = serde_json::json!({
                        "label": lab, "id": id, "motor": "ax_press_label"
                    });
                    state.lock().unwrap().push_action_result(true, "tap", detail.clone());
                    return DaemonResponse::ok(detail);
                }
            } else if let Some(ref eid) = id {
                if AxDump::press_id(&udid, eid).is_ok() {
                    let detail = serde_json::json!({
                        "label": label, "id": eid, "motor": "ax_press_id"
                    });
                    state.lock().unwrap().push_action_result(true, "tap", detail.clone());
                    return DaemonResponse::ok(detail);
                }
            }
            }
            let (nx, ny, waited_ms, used_label, used_id) = if let Some(ref lab) = label {
                // Prefer label — semantic and stable across transitions.
                let timeout = Duration::from_millis(timeout_ms.unwrap_or(2000));
                match AxDump::wait_label(&udid, lab, timeout) {
                    Ok((x, y, waited)) => (
                        x,
                        y,
                        Some(waited.as_secs_f64() * 1000.0),
                        Some(lab.clone()),
                        None,
                    ),
                    Err(e) => {
                        // Optional id fallback
                        if let Some(ref eid) = id {
                            match AxDump::wait_id(&udid, eid, timeout) {
                                Ok((x, y, waited)) => (
                                    x,
                                    y,
                                    Some(waited.as_secs_f64() * 1000.0),
                                    Some(lab.clone()),
                                    Some(eid.clone()),
                                ),
                                Err(_) => return DaemonResponse::err(e),
                            }
                        } else {
                            return DaemonResponse::err(e);
                        }
                    }
                }
            } else if let Some(ref eid) = id {
                let timeout = Duration::from_millis(timeout_ms.unwrap_or(2000));
                match AxDump::wait_id(&udid, eid, timeout) {
                    Ok((x, y, waited)) => (
                        x,
                        y,
                        Some(waited.as_secs_f64() * 1000.0),
                        None,
                        Some(eid.clone()),
                    ),
                    Err(e) => return DaemonResponse::err(e),
                }
            } else if normalized {
                (x, y, None, None, None)
            } else {
                let ww = w.max(1.0);
                let hh = h.max(1.0);
                (x / ww, y / hh, None, None, None)
            };
            match HidInput::tap(&udid, nx, ny, w, h) {
                Ok(_) => {
                    let mut detail = serde_json::json!({
                        "x": nx, "y": ny, "label": used_label, "id": used_id, "waited_ms": waited_ms,
                        "motor": if ligh_host::physical_ui_active() { "physical" } else { "sim" },
                    });
                    if ligh_host::physical_ui_active() {
                        let before_sig = before_sig.clone();
                        std::thread::sleep(Duration::from_millis(280));
                        let after = build_observe_once(&state, true);
                        let after_sig = after.screen_sig.clone().unwrap_or_default();
                        detail["before_sig"] = serde_json::json!(before_sig);
                        detail["after_sig"] = serde_json::json!(after_sig);
                        detail["actionable_n"] = serde_json::json!(after.actionable_topk.len());
                        if !before_sig.is_empty() && before_sig == after_sig {
                            let err = ligh_core::LighError::NotReady(
                                "physical tap had no UI effect (screen_sig unchanged) — arms did not move the tree".into(),
                            );
                            state.lock().unwrap().push_action_result(
                                false,
                                "tap",
                                detail.clone(),
                            );
                            return DaemonResponse::err(err);
                        }
                        detail["effect"] = serde_json::json!("ok");
                    }
                    state.lock().unwrap().push_action_result(true, "tap", detail.clone());
                    DaemonResponse::ok(detail)
                }
                Err(e) => {
                    state.lock().unwrap().push_action_result(
                        false,
                        "tap",
                        serde_json::json!({ "error": e.to_string() }),
                    );
                    DaemonResponse::err(e)
                }
            }
        }

        DaemonRequest::LongPress {
            x,
            y,
            normalized,
            label,
            id,
            hold_ms,
            timeout_ms,
        } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            let w = st.sim_width;
            let h = st.sim_height;
            drop(st);
            let hold = hold_ms.unwrap_or(600) as f64;
            let (nx, ny) = if let Some(ref eid) = id {
                let timeout = Duration::from_millis(timeout_ms.unwrap_or(2000));
                match AxDump::wait_id(&udid, eid, timeout) {
                    Ok((x, y, _)) => (x, y),
                    Err(e) => return DaemonResponse::err(e),
                }
            } else if let Some(ref label) = label {
                let timeout = Duration::from_millis(timeout_ms.unwrap_or(2000));
                match AxDump::wait_label(&udid, label, timeout) {
                    Ok((x, y, _)) => (x, y),
                    Err(e) => return DaemonResponse::err(e),
                }
            } else if normalized {
                (x, y)
            } else {
                (x / w.max(1.0), y / h.max(1.0))
            };
            match HidInput::tap_hold(&udid, nx, ny, w, h, hold) {
                Ok(_) => {
                    let detail = serde_json::json!({ "x": nx, "y": ny, "hold_ms": hold, "label": label, "id": id });
                    state.lock().unwrap().push_action_result(true, "long_press", detail.clone());
                    DaemonResponse::ok(detail)
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::ScrollUntil {
            label,
            id,
            max_swipes,
            timeout_ms,
        } => {
            if label.is_none() && id.is_none() {
                return DaemonResponse::err("scroll_until needs label or id");
            }
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            let w = st.sim_width;
            let h = st.sim_height;
            drop(st);
            let max = max_swipes.unwrap_or(8);
            let deadline = Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(12000));
            let mut swipes = 0u32;
            loop {
                if let Some(ref eid) = id {
                    if let Ok(true) = AxDump::exists_id(&udid, eid) {
                        let detail = serde_json::json!({ "found": true, "id": eid, "swipes": swipes });
                        state.lock().unwrap().push_action_result(true, "scroll_until", detail.clone());
                        return DaemonResponse::ok(detail);
                    }
                }
                if let Some(ref lab) = label {
                    if let Ok(true) = AxDump::exists_label(&udid, lab) {
                        let detail = serde_json::json!({ "found": true, "label": lab, "swipes": swipes });
                        state.lock().unwrap().push_action_result(true, "scroll_until", detail.clone());
                        return DaemonResponse::ok(detail);
                    }
                }
                if swipes >= max || Instant::now() >= deadline {
                    return DaemonResponse::err(format!(
                        "scroll_until miss after {swipes} swipes (label={label:?} id={id:?})"
                    ));
                }
                // Human-like fling up (content moves up → finger moves down→up on screen).
                if let Err(e) = HidInput::swipe(&udid, 0.5, 0.72, 0.5, 0.28, w, h) {
                    return DaemonResponse::err(e);
                }
                swipes += 1;
                std::thread::sleep(Duration::from_millis(280));
            }
        }

        DaemonRequest::Type { text } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            match HidInput::type_text(&udid, &text) {
                Ok(_) => {
                    let detail = serde_json::json!({ "chars": text.chars().count(), "text": text });
                    let mut st = state.lock().unwrap();
                    st.push_action_result(true, "type", detail.clone());
                    // Host-side sensation: Messages often omits body in AX value.
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0);
                    st.sense_buf.push(ligh_core::SenseEvent {
                        t: now,
                        kind: "typed".into(),
                        payload: Some(serde_json::json!({ "text": text, "verified": "host_accepted" })),
                    });
                    DaemonResponse::ok(detail)
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Clear { count } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            let n = count.unwrap_or(40);
            match HidInput::clear(&udid, n) {
                Ok(_) => {
                    let detail = serde_json::json!({ "count": n });
                    state.lock().unwrap().push_action_result(true, "clear", detail.clone());
                    DaemonResponse::ok(detail)
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Key { name } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            match HidInput::key_named(&udid, &name) {
                Ok(_) => {
                    let detail = serde_json::json!({ "key": name });
                    state.lock().unwrap().push_action_result(true, "key", detail.clone());
                    DaemonResponse::ok(detail)
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Wait {
            label,
            id,
            timeout_ms,
        } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            let timeout = Duration::from_millis(timeout_ms.unwrap_or(8000));
            if let Some(ref eid) = id {
                match AxDump::wait_id(&udid, eid, timeout) {
                    Ok((x, y, waited)) => DaemonResponse::ok(serde_json::json!({
                        "id": eid, "x": x, "y": y,
                        "waited_ms": waited.as_secs_f64() * 1000.0, "found": true,
                    })),
                    Err(e) => DaemonResponse::err(e),
                }
            } else if let Some(ref label) = label {
                match AxDump::wait_label(&udid, label, timeout) {
                    Ok((x, y, waited)) => DaemonResponse::ok(serde_json::json!({
                        "label": label, "x": x, "y": y,
                        "waited_ms": waited.as_secs_f64() * 1000.0, "found": true,
                    })),
                    Err(e) => DaemonResponse::err(e),
                }
            } else {
                DaemonResponse::err("wait needs label or id")
            }
        }

        DaemonRequest::Exists { label, id } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            if let Some(ref eid) = id {
                match AxDump::exists_id(&udid, eid) {
                    Ok(found) => DaemonResponse::ok(serde_json::json!({ "id": eid, "found": found })),
                    Err(e) => DaemonResponse::err(e),
                }
            } else if let Some(ref label) = label {
                match AxDump::exists_label(&udid, label) {
                    Ok(found) => {
                        DaemonResponse::ok(serde_json::json!({ "label": label, "found": found }))
                    }
                    Err(e) => DaemonResponse::err(e),
                }
            } else {
                DaemonResponse::err("exists needs label or id")
            }
        }

        DaemonRequest::Sense => {
            let st = state.lock().unwrap();
            DaemonResponse::ok(serde_json::json!({ "events": st.sense_buf }))
        }

        DaemonRequest::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            normalized,
        } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            let (fnx, fny, tnx, tny) = if normalized {
                (from_x, from_y, to_x, to_y)
            } else {
                let w = st.sim_width.max(1.0);
                let h = st.sim_height.max(1.0);
                (from_x / w, from_y / h, to_x / w, to_y / h)
            };
            let w = st.sim_width;
            let h = st.sim_height;
            drop(st);
            let before_sig = if ligh_host::physical_ui_active() {
                build_observe_once(&state, true)
                    .screen_sig
                    .unwrap_or_default()
            } else {
                String::new()
            };
            match HidInput::swipe(&udid, fnx, fny, tnx, tny, w, h) {
                Ok(_) => {
                    let mut detail = serde_json::json!({
                        "from": [fnx, fny], "to": [tnx, tny],
                        "motor": if ligh_host::physical_ui_active() { "physical" } else { "sim" },
                    });
                    if ligh_host::physical_ui_active() {
                        std::thread::sleep(Duration::from_millis(320));
                        let after = build_observe_once(&state, true);
                        let after_sig = after.screen_sig.clone().unwrap_or_default();
                        detail["before_sig"] = serde_json::json!(before_sig);
                        detail["after_sig"] = serde_json::json!(after_sig);
                        if !before_sig.is_empty() && before_sig == after_sig {
                            let err = ligh_core::LighError::NotReady(
                                "physical swipe had no UI effect (screen_sig unchanged)".into(),
                            );
                            state.lock().unwrap().push_action_result(false, "swipe", detail);
                            return DaemonResponse::err(err);
                        }
                        detail["effect"] = serde_json::json!("ok");
                    }
                    state.lock().unwrap().push_action_result(true, "swipe", detail.clone());
                    DaemonResponse::ok(detail)
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Home => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            match HidInput::home(&udid) {
                Ok(_) => {
                    state
                        .lock()
                        .unwrap()
                        .push_action_result(true, "home", serde_json::json!({}));
                    DaemonResponse::ok_empty()
                }
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Screenshot { path } => {
            let st = state.lock().unwrap();
            let comp = st.compositor.clone();
            drop(st);
            HostSession::poll_stream();
            match Screenshot::capture(&comp) {
                Err(e) => DaemonResponse::err(e),
                Ok(shot) => {
                    if let Some(p) = path {
                        match shot.write_png(std::path::Path::new(&p)) {
                            Ok(_) => DaemonResponse::ok(serde_json::json!({
                                "path": p, "width": shot.width, "height": shot.height
                            })),
                            Err(e) => DaemonResponse::err(e),
                        }
                    } else {
                        match shot.to_png_bytes() {
                            Ok(bytes) => {
                                let b64 = base64_encode(&bytes);
                                DaemonResponse::ok(serde_json::json!({
                                    "png_base64": b64,
                                    "width": shot.width,
                                    "height": shot.height,
                                }))
                            }
                            Err(e) => DaemonResponse::err(e),
                        }
                    }
                }
            }
        }

        DaemonRequest::FrameMeta => {
            let st = state.lock().unwrap();
            let gpu = st.compositor.stats();
            drop(st);
            let meta = FrameMeta {
                width: gpu.last_width,
                height: gpu.last_height,
                id: gpu.imports_ok,
                fps: gpu.fps,
                imports_ok: gpu.imports_ok > 0,
            };
            DaemonResponse::ok(meta)
        }

        DaemonRequest::Observe {
            ax: include_ax,
            settle_ms,
        } => {
            let t0 = Instant::now();
            let settle_budget = Duration::from_millis(settle_ms.unwrap_or(0));
            let deadline = if settle_budget.is_zero() {
                None
            } else {
                Some(Instant::now() + settle_budget)
            };

            let mut snap = build_observe_once(state, include_ax);
            while let Some(dl) = deadline {
                if snap.is_actionable_eyes() {
                    break;
                }
                if Instant::now() >= dl {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
                snap = build_observe_once(state, include_ax);
            }
            attach_sense(state, &mut snap, t0);
            DaemonResponse::ok(snap)
        }

        DaemonRequest::EnsureReady {
            settle_ms,
            recover_homes,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let homes = recover_homes.unwrap_or(6);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::ensure_ready(&build, state, settle, homes);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::OpenSettings { settle_ms } => {
            let settle = settle_ms.unwrap_or(2500);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::open_settings(&build, state, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::SettingsSearch { query, settle_ms } => {
            let settle = settle_ms.unwrap_or(2500);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::settings_search(&build, state, &query, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::AssertSurface { surface, settle_ms } => {
            let settle = settle_ms.unwrap_or(2500);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::assert_surface(&build, &surface, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::ActTap {
            label,
            id,
            settle_ms,
            timeout_ms,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let timeout = timeout_ms.unwrap_or(5000);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::act_tap(
                &build,
                state,
                label.as_deref(),
                id.as_deref(),
                settle,
                timeout,
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::ActType { text, settle_ms } => {
            let settle = settle_ms.unwrap_or(2500);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::act_type(&build, state, &text, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::RunApp {
            app,
            bundle_id,
            wait_label,
            wait_id,
            settle_ms,
            timeout_ms,
            install,
            launch_args,
        } => {
            let settle = settle_ms.unwrap_or(3500);
            let timeout = timeout_ms.unwrap_or(20000);
            let do_install = install.unwrap_or(true);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::run_app(
                &build,
                state,
                &app,
                bundle_id.as_deref(),
                wait_label.as_deref(),
                wait_id.as_deref(),
                settle,
                timeout,
                do_install,
                launch_args.as_deref(),
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::WaitLabel {
            label,
            settle_ms,
            timeout_ms,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let timeout = timeout_ms.unwrap_or(8000);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::wait_label(&build, state, &label, settle, timeout);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::AppJob {
            app,
            bundle_id,
            steps,
            settle_ms,
            timeout_ms,
            install,
            launch_args,
        } => {
            let settle = settle_ms.unwrap_or(3500);
            let timeout = timeout_ms.unwrap_or(10000);
            let do_install = install.unwrap_or(true);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::app_job(
                &build,
                state,
                &app,
                bundle_id.as_deref(),
                &steps,
                settle,
                timeout,
                do_install,
                launch_args.as_deref(),
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Perceive {
            settle_ms,
            workspace,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let ws = workspace.as_deref().map(std::path::Path::new);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = qa_cap::cap_perceive(&build, state, settle, ws);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Attempt {
            intent,
            label,
            id,
            text,
            key,
            expect,
            settle_ms,
            timeout_ms,
            workspace,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let timeout = timeout_ms.unwrap_or(8000);
            let ws = workspace.as_deref().map(std::path::Path::new);
            let exp = parse_expectation(expect.as_ref());
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = qa_cap::cap_attempt(
                &build,
                state,
                &intent,
                label.as_deref(),
                id.as_deref(),
                text.as_deref(),
                key.as_deref(),
                exp.as_ref(),
                settle,
                timeout,
                ws,
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Find {
            label,
            id,
            scroll,
            settle_ms,
            timeout_ms,
            max_swipes,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let timeout = timeout_ms.unwrap_or(12000);
            let do_scroll = scroll.unwrap_or(true);
            let swipes = max_swipes.unwrap_or(8);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = qa_cap::cap_find(
                &build,
                state,
                label.as_deref(),
                id.as_deref(),
                do_scroll,
                settle,
                timeout,
                swipes,
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Dismiss { settle_ms } => {
            let settle = settle_ms.unwrap_or(2500);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = qa_cap::cap_dismiss(&build, state, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Reach {
            label,
            id,
            max_swipes,
            settle_ms,
            timeout_ms,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let timeout = timeout_ms.unwrap_or(12000);
            let max = max_swipes.unwrap_or(12);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = crate::motor::motor_reach(
                &build,
                state,
                label.as_deref(),
                id.as_deref(),
                max,
                settle,
                timeout,
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::DismissOverlay { settle_ms } => {
            let settle = settle_ms.unwrap_or(2500);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = crate::motor::motor_dismiss_overlay(&build, state, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::UxStatus { workspace } => {
            let ws = workspace.as_deref().map(std::path::Path::new);
            cap_response(ux_cap::cap_ux_status(ws))
        }

        DaemonRequest::UxBaseline {
            name,
            workspace,
            settle_ms,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let ws = workspace.as_deref().map(std::path::Path::new);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = ux_cap::cap_ux_baseline(ws, &name, &build, state, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::UxRegress {
            baseline,
            workspace,
            settle_ms,
        } => {
            let settle = settle_ms.unwrap_or(2500);
            let ws = workspace.as_deref().map(std::path::Path::new);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = ux_cap::cap_ux_regress(ws, &baseline, &build, state, settle);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::UxExplore {
            max_steps,
            max_depth,
            workspace,
            settle_ms,
            timeout_ms,
        } => {
            let steps = max_steps.unwrap_or(6);
            let depth = max_depth.unwrap_or(3);
            let settle = settle_ms.unwrap_or(2500);
            let timeout = timeout_ms.unwrap_or(8000);
            let ws = workspace.as_deref().map(std::path::Path::new);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r =
                ux_cap::cap_ux_explore(ws, &build, state, steps, depth, settle, timeout);
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Explore {
            label,
            id,
            max_probes,
            max_swipes,
            settle_ms,
            timeout_ms,
        } => {
            let settle = settle_ms.unwrap_or(3500);
            let timeout = timeout_ms.unwrap_or(18000);
            let probes = max_probes.unwrap_or(4);
            let swipes = max_swipes.unwrap_or(10);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = crate::motor::motor_explore(
                &build,
                state,
                label.as_deref(),
                id.as_deref(),
                probes,
                swipes,
                settle,
                timeout,
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::Autopilot {
            app,
            bundle_id,
            goal,
            max_steps,
            workspace,
            settle_ms,
            timeout_ms,
            deadline_unix_ms,
            install,
            launch_args,
        } => {
            let parsed: ligh_core::PilotGoal = match serde_json::from_value(goal) {
                Ok(g) => g,
                Err(e) => return DaemonResponse::err(format!("goal: {e}")),
            };
            let settle = settle_ms.unwrap_or(1500);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let absolute_remaining = deadline_unix_ms.map(|d| d.saturating_sub(now_ms));
            let action_timeout = timeout_ms.unwrap_or(8000);
            let steps = max_steps.unwrap_or(24);
            let compatibility_run_budget = action_timeout
                .saturating_mul(steps.max(1) as u64)
                .saturating_add(30_000);
            let run_timeout = absolute_remaining
                .map(|absolute| absolute.min(compatibility_run_budget))
                .unwrap_or(compatibility_run_budget);
            if run_timeout == 0 {
                return DaemonResponse::err("deadline exceeded before autopilot started");
            }
            let ws = workspace.as_deref().map(std::path::Path::new);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = pilot_cap::cap_autopilot(
                ws,
                &build,
                state,
                app.as_deref(),
                bundle_id.as_deref(),
                &parsed,
                steps,
                settle,
                action_timeout.min(run_timeout),
                run_timeout,
                install.unwrap_or(true),
                launch_args.as_deref(),
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::UxHint {
            fingerprint,
            source_path,
            workspace,
        } => {
            let ws = workspace.as_deref().map(std::path::Path::new);
            cap_response(ux_cap::cap_ux_hint(ws, &fingerprint, &source_path))
        }

        DaemonRequest::UxCompileFlow { goal_id, workspace } => {
            let ws = workspace.as_deref().map(std::path::Path::new);
            cap_response(ux_cap::cap_ux_compile_flow(ws, &goal_id))
        }

        DaemonRequest::UxExecuteCompiled {
            goal_id,
            app,
            bundle_id,
            workspace,
            settle_ms,
            timeout_ms,
        } => {
            let settle = settle_ms.unwrap_or(3500);
            let timeout = timeout_ms.unwrap_or(20000);
            let ws = workspace.as_deref().map(std::path::Path::new);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = ux_cap::cap_ux_execute_compiled(
                ws,
                &goal_id,
                &app,
                bundle_id.as_deref(),
                &build,
                state,
                settle,
                timeout,
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::AppGoal {
            app,
            bundle_id,
            setup,
            postconditions,
            settle_ms,
            timeout_ms,
            install,
            launch_args,
        } => {
            let settle = settle_ms.unwrap_or(3500);
            let timeout = timeout_ms.unwrap_or(15000);
            let do_install = install.unwrap_or(true);
            let state_c = state.clone();
            let build = move || build_observe_once(&state_c, true);
            let mut r = capabilities::app_goal(
                &build,
                state,
                app.as_deref(),
                bundle_id.as_deref(),
                &setup,
                &postconditions,
                settle,
                timeout,
                do_install,
                launch_args.as_deref(),
            );
            if let Some(ref mut obs) = r.observe {
                attach_sense(state, obs, Instant::now());
            }
            cap_response(r)
        }

        DaemonRequest::StreamStats => {
            let st = state.lock().unwrap();
            let gpu = st.compositor.stats();
            drop(st);
            DaemonResponse::ok(serde_json::json!({
                "frames": gpu.frames,
                "imports_ok": gpu.imports_ok,
                "imports_fail": gpu.imports_fail,
                "last_width": gpu.last_width,
                "last_height": gpu.last_height,
                "fps": gpu.fps,
            }))
        }

        DaemonRequest::Shutdown => {
            let cfg = LighConfig::load().ok();
            if let Some(cfg) = cfg {
                if let Ok(Some(session)) = SessionState::load(&cfg.state_dir) {
                    let sup = SimSupervisor::new(cfg);
                    let _ = sup.down();
                    info!(udid = %session.udid, "shutdown via RPC");
                }
            }
            let _ = std::fs::remove_file(sock_path());
            // Exit after responding — handled by dropping listener on process exit.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                std::process::exit(0);
            });
            DaemonResponse::ok_empty()
        }

        DaemonRequest::Quit => {
            info!("quit via RPC — guest left booted");
            HostSession::detach_stream();
            let _ = std::fs::remove_file(sock_path());
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                std::process::exit(0);
            });
            DaemonResponse::ok_empty()
        }
    }
}

// ─────────────────────────── Tiny base64 ─────────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

// ─────────────────────────── main ────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("ligh=info")
        .try_init();

    info!("lighd v3 — GPU-native headless daemon");

    // Boot the compositor and optionally attach stream if a session exists.
    let compositor = Arc::new(HeadlessCompositor::new()?);
    let comp = compositor.clone();
    HostSession::set_frame_handler(move |id, w, h| comp.ingest(id, w, h));

    let mut udid: Option<String> = None;
    let mut persisted_session: Option<SessionState> = None;
    let sim_width = 393f64;
    let sim_height = 852f64;

    if let Ok(cfg) = LighConfig::load() {
        if let Ok(Some(session)) = SessionState::load(&cfg.state_dir) {
            persisted_session = Some(session.clone());
            if ligh_sim::Simctl::is_booted(&session.udid).unwrap_or(false) {
                match HostSession::stream_start(&session.udid) {
                    Ok(host) => {
                        info!(udid = %session.udid, "IOSurface stream attached on startup");
                        udid = Some(session.udid.clone());
                        std::mem::forget(host); // keep stream alive
                        if let Err(e) = HidInput::prepare(&session.udid) {
                            warn!(error=%e, "hid prepare on startup");
                        }
                    }
                    Err(e) => warn!(error=%e, "could not attach stream on startup"),
                }
            }
        }
    }
    let epoch_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1);

    let state = Arc::new(Mutex::new(DaemonState {
        compositor: compositor.clone(),
        sim_width,
        sim_height,
        udid,
        session_id: persisted_session
            .as_ref()
            .map(|s| s.session_id.clone())
            .unwrap_or_else(|| format!("daemon-{epoch_seed:016x}")),
        boot_epoch: persisted_session.as_ref().map(|s| s.boot_epoch).unwrap_or(epoch_seed),
        launch_epoch: persisted_session.as_ref().map(|s| s.launch_epoch).unwrap_or(0),
        screen_epoch: 0,
        stability_streak: 0,
        expected_bundle_id: persisted_session
            .as_ref()
            .and_then(|s| s.app_bundle_id.clone()),
        operation_lease: Arc::new(Mutex::new(())),
        last_screen_fingerprint: None,
        last_ax_nodes: None,
        sense_buf: Vec::new(),
    }));

    // DisplayRing — keep IOSurface imports hot (~60 Hz).
    std::thread::spawn(|| loop {
        HostSession::poll_stream();
        std::thread::sleep(Duration::from_millis(16));
    });

    // Physical: DevDriver eyes (AX) + WDA arms (real tap/swipe). Fake UITouch
    // alone ACK'd without moving RN — WDA is the agent hand on device.
    let hub = device_hub::DeviceHub::start(device_hub::device_port());
    let arms = Arc::new(wda::WdaArms::new());
    let hybrid = hybrid_physical::HybridPhysical::new(hub.clone(), arms.clone());
    ligh_host::set_physical_ui(Some(hybrid));
    // Warm WDA in background when UDID is configured.
    std::thread::spawn(move || {
        wda::load_wda_dotenv();
        let udid = std::env::var("LIGH_WDA_UDID").unwrap_or_default();
        if udid.is_empty() {
            info!("LIGH_WDA_UDID unset — physical taps will connect WDA on first act if Appium is up");
            return;
        }
        let bundle = std::env::var("LIGH_WDA_BUNDLE").ok();
        for attempt in 1..=30 {
            match arms.ensure(&udid, bundle.as_deref()) {
                Ok(()) => {
                    info!(attempt, "WDA arms ready");
                    return;
                }
                Err(e) => {
                    if attempt == 1 || attempt % 5 == 0 {
                        warn!(attempt, error=%e, "waiting for Appium/WDA");
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });

    // Unix socket server
    let sock = sock_path();
    if let Some(parent) = sock.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }
    let listener = UnixListener::bind(&sock)?;
    info!(path = %sock.display(), "RPC socket listening");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let st = state.clone();
                std::thread::spawn(move || handle_client(s, st));
            }
            Err(e) => warn!(error=%e, "accept error"),
        }
    }
    Ok(())
}
