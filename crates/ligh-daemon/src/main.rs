//! `lighd` v3 — persistent daemon: IOSurface stream + Metal compositor + Unix socket RPC.
//!
//! Socket: `~/.ligh/lighd.sock`  (JSON-lines protocol, see ARCHITECTURE.md)

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::{
    default_sock_path, AccessibilityTree, DaemonRequest, DaemonResponse, DevicePreset,
    FeatureRequirements, FrameMeta, LighConfig, ObserveSnapshot, SessionState,
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
            let (nx, ny, waited_ms, used_label) = if let Some(ref label) = label {
                let timeout = Duration::from_millis(timeout_ms.unwrap_or(2000));
                match AxDump::wait_label(&udid, label, timeout) {
                    Ok((x, y, waited)) => (x, y, Some(waited.as_secs_f64() * 1000.0), Some(label.clone())),
                    Err(e) => return DaemonResponse::err(e),
                }
            } else if normalized {
                (x, y, None, None)
            } else {
                let ww = w.max(1.0);
                let hh = h.max(1.0);
                (x / ww, y / hh, None, None)
            };
            match HidInput::tap(&udid, nx, ny, w, h) {
                Ok(_) => DaemonResponse::ok(serde_json::json!({
                    "x": nx,
                    "y": ny,
                    "label": used_label,
                    "waited_ms": waited_ms,
                })),
                Err(e) => DaemonResponse::err(e),
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
                Ok(_) => DaemonResponse::ok(serde_json::json!({ "chars": text.chars().count() })),
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Wait { label, timeout_ms } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            let timeout = Duration::from_millis(timeout_ms.unwrap_or(8000));
            match AxDump::wait_label(&udid, &label, timeout) {
                Ok((x, y, waited)) => DaemonResponse::ok(serde_json::json!({
                    "label": label,
                    "x": x,
                    "y": y,
                    "waited_ms": waited.as_secs_f64() * 1000.0,
                    "found": true,
                })),
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Exists { label } => {
            let st = state.lock().unwrap();
            let udid = match st.current_udid() {
                Ok(u) => u,
                Err(e) => return DaemonResponse::err(e),
            };
            drop(st);
            match AxDump::exists_label(&udid, &label) {
                Ok(found) => DaemonResponse::ok(serde_json::json!({ "label": label, "found": found })),
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Swipe { from_x, from_y, to_x, to_y, normalized } => {
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
                Ok(_) => DaemonResponse::ok_empty(),
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
                Ok(_) => DaemonResponse::ok_empty(),
                Err(e) => DaemonResponse::err(e),
            }
        }

        DaemonRequest::Screenshot { path } => {
            let st = state.lock().unwrap();
            let comp = st.compositor.clone();
            drop(st);
            // Poll one more frame before capture
            HostSession::poll_stream();
            match Screenshot::capture(&comp) {
                Err(e) => DaemonResponse::err(e),
                Ok(shot) => {
                    if let Some(p) = path {
                        match shot.write_png(std::path::Path::new(&p)) {
                            Ok(_) => DaemonResponse::ok(serde_json::json!({ "path": p, "width": shot.width, "height": shot.height })),
                            Err(e) => DaemonResponse::err(e),
                        }
                    } else {
                        // Return base64 PNG in response
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

        DaemonRequest::Observe { ax: include_ax } => {
            let t0 = Instant::now();
            HostSession::poll_stream();
            let st = state.lock().unwrap();
            let udid = st.udid.clone().unwrap_or_default();
            let gpu = st.compositor.stats();
            let booted = !udid.is_empty();
            drop(st);
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
            let snap = ObserveSnapshot {
                schema_version: ligh_core::OBSERVE_SCHEMA_VERSION,
                udid,
                booted,
                simulator_app_running: false,
                frame,
                app_bundle_id,
                accessibility_tree: ax,
                observe_ms: Some(t0.elapsed().as_secs_f64() * 1000.0),
                path: Some("lighd".into()),
            };
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
