//! LIGH v3 session — private boot + IOSurface stream + Metal compositor + GUI.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ligh_core::{DevicePreset, LighConfig, LighError, SessionState};
use ligh_gpu::{run_window, FrameCompositor, GuiOptions, PointerPhase, TouchBridge};
use ligh_host::{HostSession, HidInput, StreamStats};
use ligh_gpu::CompositorStats;
use ligh_sim::{ensure_headless, DeviceManager, Simctl};
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct GpuSessionReport {
    pub udid: String,
    pub boot_secs: f64,
    pub sample_secs: f64,
    pub stream: StreamStats,
    pub compositor: CompositorStats,
    pub hid_tap_ok: bool,
}

pub struct GpuSession {
    udid: String,
    sim_width: u32,
    sim_height: u32,
    point_width: f64,
    point_height: f64,
    tablet: bool,
    device_title: String,
    _host: HostSession,
    compositor: Arc<FrameCompositor>,
}

impl GpuSession {
    /// Boot + IOSurface stream (headless verify).
    pub fn probe(preset: DevicePreset) -> Result<GpuSessionReport, LighError> {
        let t0 = Instant::now();
        let session = Self::start(preset, None)?;
        let boot_secs = t0.elapsed().as_secs_f64();
        let w = session.point_width;
        let h = session.point_height;

        let t1 = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut nudges = 0u32;
        while Instant::now() < deadline {
            session._host.poll_frame();
            if nudges < 5 && t1.elapsed().as_secs() >= (nudges as u64 + 1) * 2 {
                let nx = 0.3 + (nudges as f64 * 0.1);
                let _ = HidInput::tap(&session.udid, nx, 0.5, w, h);
                nudges += 1;
            }
            if session.compositor.stats().imports_ok >= 60 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let sample_secs = t1.elapsed().as_secs_f64();
        let hid_tap_ok = HidInput::tap(&session.udid, 0.5, 0.5, w, h).is_ok();

        Ok(GpuSessionReport {
            udid: session.udid,
            boot_secs,
            sample_secs,
            stream: session._host.stats(),
            compositor: session.compositor.stats(),
            hid_tap_ok,
        })
    }

    /// Full session: boot, stream, open Metal window with touch input.
    pub fn run_gui(
        preset: DevicePreset,
        explicit_udid: Option<&str>,
        verify: bool,
    ) -> Result<(), LighError> {
        let session = Self::start(preset, explicit_udid)?;
        info!(udid = %session.udid, verify, "opening LIGH GUI");

        let udid = session.udid.clone();
        let w = session.point_width;
        let h = session.point_height;

        let touch = TouchBridge {
            pointer: Box::new(move |phase, nx, ny| {
                let p = match phase {
                    PointerPhase::Down => 1,
                    PointerPhase::Up => 2,
                    PointerPhase::Move => 3,
                };
                HidInput::pointer(&udid, nx, ny, p, w, h)
            }),
            home: Box::new({
                let udid = session.udid.clone();
                move || HidInput::home(&udid)
            }),
        };

        let poll_stop = Arc::new(AtomicBool::new(false));
        let poll_flag = poll_stop.clone();
        let poll_thread = std::thread::spawn(move || {
            while !poll_flag.load(Ordering::Relaxed) {
                HostSession::poll_stream();
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        let gui_result = run_window(
            session.compositor,
            touch,
            GuiOptions {
                title: format!("LIGH — {}", session.device_title),
                pixel_width: session.sim_width,
                pixel_height: session.sim_height,
                point_width: session.point_width,
                point_height: session.point_height,
                tablet: session.tablet,
                self_test_secs: if verify { Some(5) } else { None },
            },
        );

        poll_stop.store(true, Ordering::Relaxed);
        let _ = poll_thread.join();
        gui_result
    }

    fn start(preset: DevicePreset, explicit_udid: Option<&str>) -> Result<Self, LighError> {
        ensure_headless();
        let session_udid = LighConfig::load()
            .ok()
            .and_then(|c| SessionState::load(&c.state_dir).ok().flatten())
            .map(|s| s.udid);
        let device = DeviceManager::resolve(preset, explicit_udid, session_udid.as_deref())?;
        let udid = device.udid.clone();
        let device_name = device.name.clone();

        let compositor = Arc::new(FrameCompositor::new()?);
        let comp = compositor.clone();

        HostSession::set_frame_handler(move |id, w, h| {
            comp.ingest(id, w, h);
        });

        Simctl::shutdown_other_booted(&udid)?;

        if !Simctl::is_booted(&udid)? {
            if HostSession::boot(&udid).is_err() {
                info!("private boot unavailable — simctl fallback");
                boot_simctl(&udid)?;
            }
        }

        if Simctl::wait_ready(&udid, Duration::from_secs(180)).is_err() {
            warn!("userspace spawn slow after private boot — simctl re-boot");
            let _ = Simctl::run(&["shutdown", &udid]);
            std::thread::sleep(Duration::from_secs(3));
            boot_simctl(&udid)?;
            Simctl::wait_ready(&udid, Duration::from_secs(180))?;
        }
        if let Err(e) = Simctl::wait_springboard(&udid, Duration::from_secs(90)) {
            warn!(error = %e, "SpringBoard host check timed out — continuing to IOSurface");
        }

        let host = connect_stream(&udid, &compositor)?;
        let stats = compositor.stats();
        let sim_width = stats.last_width.max(393);
        let sim_height = stats.last_height.max(852);
        let (point_width, point_height) = preset.hid_size_from_framebuffer(sim_width, sim_height);

        if let Ok(config) = LighConfig::load() {
            let _ = SessionState {
                udid: udid.clone(),
                device: preset,
                name: device_name.clone(),
                slim_applied: false,
                app_bundle_id: None,
                app_path: None,
            }
            .save(&config.state_dir);
        }

        Ok(Self {
            udid,
            sim_width,
            sim_height,
            point_width,
            point_height,
            tablet: preset.is_tablet(),
            device_title: preset.simctl_name().to_string(),
            _host: host,
            compositor,
        })
    }
}

fn boot_simctl(udid: &str) -> Result<(), LighError> {
    let jobs: Vec<String> = ligh_core::resolve_disabled_jobs(&Default::default());
    let mut cmd = std::process::Command::new("xcrun");
    cmd.arg("simctl").arg("boot").arg(udid);
    for label in &jobs {
        cmd.arg(format!("--disabledJob={label}"));
    }
    let output = cmd.output().map_err(LighError::Io)?;
    if output.status.success() || Simctl::is_booted(udid)? {
        Ok(())
    } else {
        Simctl::run(&["boot", udid])?;
        Ok(())
    }
}

fn wait_first_frame(
    host: &HostSession,
    compositor: &FrameCompositor,
    timeout: Duration,
) -> Result<(), LighError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        host.poll_frame();
        let stats = compositor.stats();
        if stats.imports_ok >= 1 && stats.last_width > 0 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(LighError::NotReady(
        "no IOSurface frame imported — SpringBoard may still be starting".into(),
    ))
}

/// Connect IOSurface stream with retries (framebuffer port can lag SpringBoard).
fn connect_stream(udid: &str, compositor: &FrameCompositor) -> Result<HostSession, LighError> {
    const MAX_ATTEMPTS: u32 = 5;
    let started = Instant::now();
    let overall = Duration::from_secs(60);

    for attempt in 1..=MAX_ATTEMPTS {
        if started.elapsed() >= overall {
            break;
        }

        match HostSession::stream_start(udid) {
            Ok(host) => {
                if wait_first_frame(&host, compositor, Duration::from_secs(20)).is_ok() {
                    info!(attempt, "IOSurface stream ready");
                    return Ok(host);
                }
                warn!(attempt, "stream connected but no frames — retrying");
                host.stream_stop();
            }
            Err(e) => {
                warn!(attempt, error = %e, "stream_start failed — retrying");
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }

    Err(LighError::NotReady(
        "failed to connect IOSurface stream after retries — try `ligh down` and retry".into(),
    ))
}
