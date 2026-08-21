//! `lighd` v3 — persistent daemon: IOSurface stream + Metal compositor + Unix socket RPC.
//!
//! Socket: `~/.ligh/lighd.sock`  (JSON-lines protocol, see ARCHITECTURE.md)

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    default_sock_path, diff_sense_events, AccessibilityTree, DaemonRequest, DaemonResponse,
    DevicePreset, FeatureRequirements, FrameMeta, LighConfig, ObserveSnapshot, SenseEvent,
    SessionState,
};
use ligh_gpu::{HeadlessCompositor, Screenshot};
use ligh_host::{AxDump, HidInput, HostSession};
use ligh_sim::{ensure_headless, SimSupervisor};
use tracing::{info, warn};

// ─────────────────────────── Daemon state ────────────────────────────────────

struct DaemonState {
    compositor: Arc<HeadlessCompositor>,
    /// Width/height in points from last stream attach.
    sim_width: f64,
    sim_height: f64,
    udid: Option<String>,
    /// Previous AX flat nodes for sensation diff.
    last_ax_nodes: Option<Vec<serde_json::Value>>,
    /// Recent sense events (ring, newest last).
    sense_buf: Vec<SenseEvent>,
}

impl DaemonState {
    fn current_udid(&self) -> Result<String, String> {
        let cfg = LighConfig::load().map_err(|e| e.to_string())?;
        if let Some(u) = &self.udid {
            return Ok(u.clone());
        }
        SessionState::load(&cfg.state_dir)
            .map_err(|e| e.to_string())?
            .map(|s| s.udid)
            .ok_or_else(|| "no session — run `lighd` boot or `ligh up` first".into())
    }

    fn push_action_result(&mut self, ok: bool, kind: &str, detail: serde_json::Value) {
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
            DaemonResponse::ok(serde_json::json!({
                "udid": session.as_ref().map(|s| &s.udid),
                "booted": booted,
                "simulator_app_running": false,
                "frame": {
                    "w": gpu.last_width,
                    "h": gpu.last_height,
                    "id": gpu.imports_ok,
                    "fps": gpu.fps,
                    "imports_ok": gpu.imports_ok > 0,
                },
                "app_bundle_id": session.and_then(|s| s.app_bundle_id),
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
                        s.app_bundle_id = Some(bid.clone());
                        s.app_path = Some(app_path.clone());
                        let _ = s.save(&cfg.state_dir);
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
                            s.app_bundle_id = Some(bundle_id.clone());
                            let _ = s.save(&cfg.state_dir);
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
                    let detail = serde_json::json!({
                        "x": nx, "y": ny, "label": used_label, "id": used_id, "waited_ms": waited_ms
                    });
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
            match HidInput::swipe(&udid, fnx, fny, tnx, tny, w, h) {
                Ok(_) => {
                    state.lock().unwrap().push_action_result(
                        true,
                        "swipe",
                        serde_json::json!({ "from": [fnx, fny], "to": [tnx, tny] }),
                    );
                    DaemonResponse::ok_empty()
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

            let build_once = |state: &Arc<Mutex<DaemonState>>, include_ax: bool| -> ObserveSnapshot {
                HostSession::poll_stream();
                let (udid, gpu) = {
                    let st = state.lock().unwrap();
                    let udid = st.udid.clone().unwrap_or_default();
                    let gpu = st.compositor.stats();
                    (udid, gpu)
                };
                let booted = !udid.is_empty();
                let app_bundle_id = LighConfig::load()
                    .ok()
                    .and_then(|c| SessionState::load(&c.state_dir).ok().flatten())
                    .and_then(|s| s.app_bundle_id);
                let frame = if gpu.imports_ok > 0 {
                    Some(FrameMeta {
                        width: gpu.last_width,
                        height: gpu.last_height,
                        id: gpu.imports_ok,
                        fps: gpu.fps,
                        imports_ok: true,
                    })
                } else {
                    None
                };
                let ax = if include_ax && !udid.is_empty() {
                    match AxDump::dump(&udid) {
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
                };
                snap.enrich_v2();
                snap
            };

            let mut snap = build_once(state, include_ax);
            while let Some(dl) = deadline {
                if snap.is_actionable_eyes() {
                    break;
                }
                if Instant::now() >= dl {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
                snap = build_once(state, include_ax);
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            snap.observe_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
            {
                let mut st = state.lock().unwrap();
                let curr = snap.accessibility_tree.nodes().to_vec();
                let mut ev = diff_sense_events(st.last_ax_nodes.as_deref(), &curr, now);
                ev.extend(st.sense_buf.iter().cloned());
                st.sense_buf.clear();
                st.last_ax_nodes = Some(curr);
                snap.events = ev;
            }
            DaemonResponse::ok(snap)
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
    let sim_width = 393f64;
    let sim_height = 852f64;

    if let Ok(cfg) = LighConfig::load() {
        if let Ok(Some(session)) = SessionState::load(&cfg.state_dir) {
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

    let state = Arc::new(Mutex::new(DaemonState {
        compositor: compositor.clone(),
        sim_width,
        sim_height,
        udid,
        last_ax_nodes: None,
        sense_buf: Vec::new(),
    }));

    // DisplayRing — keep IOSurface imports hot (~60 Hz).
    std::thread::spawn(|| loop {
        HostSession::poll_stream();
        std::thread::sleep(Duration::from_millis(16));
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
