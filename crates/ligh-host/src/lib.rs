//! CoreSimulator + SimulatorKit bridge — IOSurface framebuffer stream.
//!
//! Uses Apple's private frameworks (same approach as simsapp / idb / SimDeck).
//! Zero-copy GPU path: simulator → IOSurface → Rust Metal compositor.

mod ffi;

use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ligh_core::LighError;
use tracing::{info, warn};

pub use ffi::LighFrameFn;

type FrameHandler = Box<dyn Fn(u32, u32, u32) + Send + Sync>;

static FRAME_HANDLER: OnceLock<Mutex<Option<FrameHandler>>> = OnceLock::new();

static FRAMES: AtomicU64 = AtomicU64::new(0);
static LAST_WIDTH: AtomicU64 = AtomicU64::new(0);
static LAST_HEIGHT: AtomicU64 = AtomicU64::new(0);

/// Serialize CoreSimulator / AXP ObjC entry points — concurrent DisplayRing poll + AX
/// dump races abort the process (`Rust cannot catch foreign exceptions`).
fn bridge_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone)]
pub struct StreamStats {
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

pub struct HostSession {
    udid: String,
    started: Instant,
}

impl HostSession {
    /// Wire IOSurface frames into the Metal compositor (or any consumer).
    pub fn set_frame_handler<F>(handler: F)
    where
        F: Fn(u32, u32, u32) + Send + Sync + 'static,
    {
        FRAME_HANDLER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .replace(Box::new(handler));
    }

    pub fn init() -> Result<(), LighError> {
        let dev_dir = resolve_developer_dir();
        let c_dir = CString::new(dev_dir.clone()).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_init(c_dir.as_ptr(), &mut err) } {
            info!(developer_dir = %dev_dir, "private frameworks loaded");
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_init"))
        }
    }

    /// Headless boot via private SimDevice API — never opens Simulator.app.
    pub fn boot(udid: &str) -> Result<(), LighError> {
        Self::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_boot(c_udid.as_ptr(), &mut err) } {
            info!(udid, "private headless boot");
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_boot"))
        }
    }

    pub fn shutdown(udid: &str) -> Result<(), LighError> {
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_shutdown(c_udid.as_ptr(), &mut err) } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_shutdown"))
        }
    }

    /// Subscribe to IOSurface framebuffer updates (GPU memory, zero-copy).
    pub fn stream_start(udid: &str) -> Result<Self, LighError> {
        Self::init()?;
        let _guard = bridge_lock();
        FRAMES.store(0, Ordering::Relaxed);
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe {
            ffi::ligh_host_stream_start(c_udid.as_ptr(), Some(frame_callback), std::ptr::null_mut(), &mut err)
        } {
            info!(udid, "IOSurface stream connected");
            Ok(Self {
                udid: udid.to_string(),
                started: Instant::now(),
            })
        } else {
            Err(host_err(&err, "ligh_host_stream_start"))
        }
    }

    pub fn stream_stop(&self) {
        let _guard = bridge_lock();
        unsafe { ffi::ligh_host_stream_stop() };
    }

    /// Detach IOSurface without a session handle (daemon quit).
    pub fn detach_stream() {
        let _guard = bridge_lock();
        unsafe { ffi::ligh_host_stream_stop() };
    }

    /// Re-import latest IOSurface (SpringBoard may not redraw on a static home screen).
    pub fn poll_frame(&self) {
        Self::poll_stream();
    }

    /// Poll active stream without holding a session handle (for GUI background tick).
    pub fn poll_stream() {
        let _guard = bridge_lock();
        unsafe { ffi::ligh_host_stream_poll() };
    }

    pub fn stats(&self) -> StreamStats {
        let frames = FRAMES.load(Ordering::Relaxed);
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        StreamStats {
            frames,
            width: LAST_WIDTH.load(Ordering::Relaxed) as u32,
            height: LAST_HEIGHT.load(Ordering::Relaxed) as u32,
            fps: frames as f64 / elapsed,
        }
    }

    pub fn wait_for_frames(&self, min_frames: u64, timeout: Duration) -> Result<StreamStats, LighError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let stats = self.stats();
            if stats.frames >= min_frames && stats.width > 0 {
                return Ok(stats);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let stats = self.stats();
        if stats.frames == 0 {
            return Err(LighError::Simctl(
                "no IOSurface frames received — is SpringBoard up?".into(),
            ));
        }
        Ok(stats)
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        self.stream_stop();
    }
}

extern "C" fn frame_callback(
    _ctx: *mut std::ffi::c_void,
    surface_id: u32,
    width: u32,
    height: u32,
) {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    LAST_WIDTH.store(width as u64, Ordering::Relaxed);
    LAST_HEIGHT.store(height as u64, Ordering::Relaxed);
    if let Some(lock) = FRAME_HANDLER.get() {
        if let Some(ref handler) = *lock.lock().unwrap() {
            handler(surface_id, width, height);
        }
    }
}

fn host_err(err: &ffi::LighHostError, op: &str) -> LighError {
    let msg = if !err.message.is_null() {
        unsafe { CStr::from_ptr(err.message) }
            .to_string_lossy()
            .into_owned()
    } else {
        format!("{op} failed (code {})", err.code)
    };
    LighError::Simctl(msg)
}

fn resolve_developer_dir() -> String {
    if let Ok(d) = std::env::var("DEVELOPER_DIR") {
        if Path::new(&d)
            .join("Library/PrivateFrameworks/SimulatorKit.framework")
            .exists()
        {
            return d;
        }
    }
    let default = "/Applications/Xcode.app/Contents/Developer".to_string();
    if Path::new(&default)
        .join("Library/PrivateFrameworks/SimulatorKit.framework")
        .exists()
    {
        return default;
    }
    warn!("SimulatorKit not found — set DEVELOPER_DIR to Xcode.app/Contents/Developer");
    default
}

/// Touch + hardware button injection via SimulatorKit IndigoHID.
pub struct HidInput;

impl HidInput {
    pub fn tap(udid: &str, norm_x: f64, norm_y: f64, width: f64, height: f64) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe {
            ffi::ligh_host_hid_tap(c_udid.as_ptr(), norm_x, norm_y, width, height, &mut err)
        } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_tap"))
        }
    }

    pub fn tap_hold(
        udid: &str,
        norm_x: f64,
        norm_y: f64,
        width: f64,
        height: f64,
        hold_ms: f64,
    ) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe {
            ffi::ligh_host_hid_tap_hold(
                c_udid.as_ptr(),
                norm_x,
                norm_y,
                width,
                height,
                hold_ms,
                &mut err,
            )
        } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_tap_hold"))
        }
    }

    pub fn swipe(
        udid: &str,
        from_norm_x: f64,
        from_norm_y: f64,
        to_norm_x: f64,
        to_norm_y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe {
            ffi::ligh_host_hid_swipe(
                c_udid.as_ptr(),
                from_norm_x,
                from_norm_y,
                to_norm_x,
                to_norm_y,
                width,
                height,
                &mut err,
            )
        } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_swipe"))
        }
    }

    pub fn pointer(
        udid: &str,
        norm_x: f64,
        norm_y: f64,
        phase: u32,
        width: f64,
        height: f64,
    ) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe {
            ffi::ligh_host_hid_pointer(
                c_udid.as_ptr(),
                norm_x,
                norm_y,
                phase,
                width,
                height,
                &mut err,
            )
        } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_pointer"))
        }
    }

    pub fn home(udid: &str) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_hid_home(c_udid.as_ptr(), &mut err) } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_home"))
        }
    }

    pub fn prepare(udid: &str) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_hid_prepare(c_udid.as_ptr(), &mut err) } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_prepare"))
        }
    }

    pub fn type_text(udid: &str, text: &str) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let c_text = CString::new(text).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_hid_type(c_udid.as_ptr(), c_text.as_ptr(), &mut err) } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_type"))
        }
    }

    /// USB HID usage down+up (e.g. 0x2A = Delete/Backspace, 0x28 = Return).
    pub fn key_usage(udid: &str, usage: u32) -> Result<(), LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        if unsafe { ffi::ligh_host_hid_key(c_udid.as_ptr(), usage, &mut err) } {
            Ok(())
        } else {
            Err(host_err(&err, "ligh_host_hid_key"))
        }
    }

    pub fn clear(udid: &str, count: u32) -> Result<(), LighError> {
        for _ in 0..count.max(1) {
            Self::key_usage(udid, 0x2A)?; // Delete/Backspace
        }
        Ok(())
    }

    pub fn key_named(udid: &str, name: &str) -> Result<(), LighError> {
        let usage = match name.to_ascii_lowercase().as_str() {
            "return" | "enter" => 0x28u32,
            "delete" | "backspace" => 0x2A,
            "escape" | "esc" => 0x29,
            "tab" => 0x2B,
            "space" => 0x2C,
            "up" => 0x52,
            "down" => 0x51,
            "left" => 0x50,
            "right" => 0x4F,
            _ => {
                return Err(LighError::Simctl(format!(
                    "unknown key {name:?} (return|delete|escape|tab|space|up|down|left|right)"
                )))
            }
        };
        Self::key_usage(udid, usage)
    }
}

/// Accessibility tree dump (headless AXPTranslator → SimDevice XPC).
pub struct AxDump;

impl AxDump {
    /// Returns JSON: `{ status, root, elements: [{label,role,frame,...}], ... }`.
    pub fn dump(udid: &str) -> Result<serde_json::Value, LighError> {
        HostSession::init()?;
        let _guard = bridge_lock();
        let c_udid = CString::new(udid).map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut err = ffi::LighHostError {
            message: std::ptr::null(),
            code: 0,
        };
        let ptr = unsafe { ffi::ligh_host_ax_dump(c_udid.as_ptr(), &mut err) };
        if ptr.is_null() {
            return Err(host_err(&err, "ligh_host_ax_dump"));
        }
        let json = unsafe {
            let s = std::ffi::CStr::from_ptr(ptr)
                .to_string_lossy()
                .into_owned();
            ffi::ligh_host_ax_free(ptr);
            s
        };
        serde_json::from_str(&json).map_err(|e| LighError::Simctl(e.to_string()))
    }

    /// Poll AX until `label` matches or `timeout`.
    /// Rich trees (≥5 elements): accept on first hit.
    /// Sparse/transition trees: require two consecutive hits.
    pub fn wait_label(
        udid: &str,
        label: &str,
        timeout: Duration,
    ) -> Result<(f64, f64, Duration), LighError> {
        let t0 = Instant::now();
        let mut hits = 0u8;
        loop {
            match Self::dump(udid) {
                Ok(dump) => {
                    let rich = dump
                        .get("element_count")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0)
                        >= 5
                        || dump
                            .get("elements")
                            .and_then(|e| e.as_array())
                            .map(|a| a.len() >= 5)
                            .unwrap_or(false);
                    if let Some((x, y)) = ligh_core::find_label_in_dump(&dump, label) {
                        hits = hits.saturating_add(1);
                        if rich || hits >= 2 {
                            return Ok((x, y, t0.elapsed()));
                        }
                    } else {
                        hits = 0;
                    }
                }
                Err(e) => {
                    hits = 0;
                    if t0.elapsed() >= timeout {
                        return Err(e);
                    }
                }
            }
            if t0.elapsed() >= timeout {
                return Err(LighError::NotReady(format!(
                    "wait timeout for {label:?} after {timeout:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_id(
        udid: &str,
        id: &str,
        timeout: Duration,
    ) -> Result<(f64, f64, Duration), LighError> {
        let t0 = Instant::now();
        let mut hits = 0u8;
        loop {
            match Self::dump(udid) {
                Ok(dump) => {
                    let rich = dump
                        .get("element_count")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0)
                        >= 5;
                    if let Some((x, y)) = ligh_core::find_id_in_dump(&dump, id) {
                        hits = hits.saturating_add(1);
                        // Semantic ids are stable — accept first hit when tree has any elements.
                        if rich || hits >= 1 {
                            return Ok((x, y, t0.elapsed()));
                        }
                    } else {
                        hits = 0;
                    }
                }
                Err(e) => {
                    hits = 0;
                    if t0.elapsed() >= timeout {
                        return Err(e);
                    }
                }
            }
            if t0.elapsed() >= timeout {
                return Err(LighError::NotReady(format!(
                    "wait timeout for id={id:?} after {timeout:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn exists_label(udid: &str, label: &str) -> Result<bool, LighError> {
        let dump = Self::dump(udid)?;
        Ok(ligh_core::find_label_in_dump(&dump, label).is_some())
    }

    pub fn exists_id(udid: &str, id: &str) -> Result<bool, LighError> {
        let dump = Self::dump(udid)?;
        Ok(ligh_core::find_id_in_dump(&dump, id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn developer_dir_exists() {
        let d = resolve_developer_dir();
        assert!(Path::new(&d).exists());
    }
}
