//! Physical-device hub — Expo-style transport.
//!
//! Same shape as Metro on a development build:
//! the Mac listens, the phone connects over LAN (Wi-Fi, USB-with-LAN, tunnel).
//! If the phone cannot reach the Mac it listens on loopback and lighd reaches
//! it via `iproxy` (cable, no Wi-Fi).
//!
//! Protocol: JSON lines. The device always sends `hello` first.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligh_core::LighError;
use ligh_host::PhysicalUi;
use serde_json::{json, Value};
use tracing::{info, warn};

pub const DEFAULT_PORT: u16 = 7700;

struct Session {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    bundle_id: String,
    width: f64,
    height: f64,
    transport: &'static str,
    next_id: u64,
    driver_version: u64,
    capabilities: Value,
}

pub struct DeviceHub {
    port: u16,
    session: Mutex<Option<Session>>,
    iproxy: Mutex<Option<Child>>,
    mdns: Mutex<Option<Child>>,
    stop: AtomicBool,
}

impl DeviceHub {
    pub fn start(port: u16) -> Arc<Self> {
        let hub = Arc::new(Self {
            port,
            session: Mutex::new(None),
            iproxy: Mutex::new(None),
            mdns: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        advertise_mdns(port, &hub.mdns);
        let accept = hub.clone();
        std::thread::Builder::new()
            .name("ligh-device-lan".into())
            .spawn(move || accept.lan_accept_loop())
            .expect("device lan thread");
        let usb = hub.clone();
        std::thread::Builder::new()
            .name("ligh-device-usb".into())
            .spawn(move || usb.usb_probe_loop())
            .expect("device usb thread");
        let beat = hub.clone();
        std::thread::Builder::new()
            .name("ligh-device-heartbeat".into())
            .spawn(move || beat.heartbeat_loop())
            .expect("device heartbeat thread");
        info!(port, "physical device hub listening (LAN + USB probe)");
        hub
    }

    #[cfg(test)]
    fn for_test(port: u16) -> Arc<Self> {
        Arc::new(Self {
            port,
            session: Mutex::new(None),
            iproxy: Mutex::new(None),
            mdns: Mutex::new(None),
            stop: AtomicBool::new(false),
        })
    }

    pub fn snapshot(&self) -> Option<Value> {
        let g = self.session.lock().ok()?;
        let s = g.as_ref()?;
        Some(json!({
            "connected": true,
            "bundle_id": s.bundle_id,
            "transport": s.transport,
            "port": self.port,
            "screen": { "width": s.width, "height": s.height },
            "driver_version": s.driver_version,
            "capabilities": s.capabilities,
            "motor": "expo_or_native_debug",
            "eyes": "devdriver",
        }))
    }

    /// Bundle id of the connected DevDriver, if any.
    pub fn bundle_id_hint(&self) -> Option<String> {
        self.session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.bundle_id.clone()))
    }

    /// True when the live DevDriver advertises a usable human gesture stream.
    pub fn supports_gesture(&self) -> bool {
        let Some(snap) = self.snapshot() else {
            return false;
        };
        let caps = snap.get("capabilities");
        let ver = snap
            .get("driver_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if ver >= 2 {
            return true;
        }
        caps.and_then(|c| c.get("gesture"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || caps
                .and_then(|c| c.get("swipe"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
    }

    fn lan_accept_loop(&self) {
        let bind = format!("0.0.0.0:{}", self.port);
        let listener = match TcpListener::bind(&bind) {
            Ok(l) => l,
            Err(e) => {
                warn!(error=%e, bind=%bind, "device hub bind failed");
                return;
            }
        };
        let _ = listener.set_nonblocking(false);
        for incoming in listener.incoming() {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            match incoming {
                Ok(stream) => {
                    if let Err(e) = self.take_stream(stream, "lan") {
                        warn!(error=%e, "device hello (LAN) failed");
                    } else {
                        info!("physical DevDriver connected over LAN/Wi-Fi/USB-LAN");
                    }
                }
                Err(e) => warn!(error=%e, "device accept"),
            }
        }
    }

    fn usb_probe_loop(&self) {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1500));
            if self.active() {
                continue;
            }
            self.ensure_iproxy();
            // Local port+1 → device :port so we never connect to our own LAN listener.
            let local = self.port.saturating_add(1);
            match TcpStream::connect_timeout(
                &format!("127.0.0.1:{local}").parse().unwrap(),
                Duration::from_millis(400),
            ) {
                Ok(stream) => {
                    if let Err(e) = self.take_stream(stream, "usb") {
                        warn!(error=%e, "device hello (USB) failed");
                    } else {
                        info!("physical DevDriver connected over USB (iproxy)");
                    }
                }
                Err(_) => {}
            }
        }
    }

    fn heartbeat_loop(&self) {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_secs(12));
            if !self.active() {
                continue;
            }
            // Keep the phone socket from going idle; motor v2 also disables RCVTIMEO.
            let _ = self.rpc(json!({"op": "ping"}));
        }
    }

    fn ensure_iproxy(&self) {
        let mut slot = self.iproxy.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = slot.as_mut() {
            if child.try_wait().ok().flatten().is_none() {
                return;
            }
        }
        let local = self.port.saturating_add(1);
        match Command::new("iproxy")
            .args([local.to_string(), self.port.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                info!(port = self.port, "iproxy started for USB DevDriver");
                *slot = Some(child);
            }
            Err(_) => {}
        }
    }

    fn take_stream(&self, stream: TcpStream, transport: &'static str) -> Result<(), LighError> {
        stream
            .set_nodelay(true)
            .map_err(|e| LighError::NotReady(e.to_string()))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .ok();
        let writer = stream
            .try_clone()
            .map_err(|e| LighError::NotReady(e.to_string()))?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| LighError::NotReady(format!("device hello read: {e}")))?;
        let hello: Value = serde_json::from_str(line.trim()).map_err(|e| {
            LighError::NotReady(format!("device hello json: {e} ({})", line.trim()))
        })?;
        if hello.get("op").and_then(|v| v.as_str()) != Some("hello") {
            return Err(LighError::NotReady("expected device hello".into()));
        }
        let bundle_id = hello
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let width = hello
            .get("width")
            .and_then(|v| v.as_f64())
            .or_else(|| hello.pointer("/screen/width").and_then(|v| v.as_f64()))
            .unwrap_or(393.0);
        let height = hello
            .get("height")
            .and_then(|v| v.as_f64())
            .or_else(|| hello.pointer("/screen/height").and_then(|v| v.as_f64()))
            .unwrap_or(852.0);
        let driver_version = hello
            .get("driver_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let capabilities = hello
            .get("capabilities")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let ack = json!({"op": "hello_ok", "port": self.port});
        {
            let mut w = writer
                .try_clone()
                .map_err(|e| LighError::NotReady(e.to_string()))?;
            writeln!(w, "{ack}").map_err(|e| LighError::NotReady(e.to_string()))?;
            w.flush().ok();
        }
        let session = Session {
            writer,
            reader,
            bundle_id: bundle_id.clone(),
            width,
            height,
            transport,
            next_id: 1,
            driver_version,
            capabilities: capabilities.clone(),
        };
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
        info!(
            bundle_id = %bundle_id,
            transport,
            width,
            height,
            driver_version,
            "DevDriver session ready"
        );
        Ok(())
    }

    fn rpc(&self, op: Value) -> Result<Value, LighError> {
        let mut last = LighError::NotReady("no physical DevDriver connected".into());
        for attempt in 0..5 {
            match self.rpc_once(op.clone()) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last = e;
                    if attempt + 1 < 5 {
                        std::thread::sleep(Duration::from_millis(350));
                    }
                }
            }
        }
        Err(last)
    }

    fn rpc_once(&self, mut op: Value) -> Result<Value, LighError> {
        let mut g = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let session = g
            .as_mut()
            .ok_or_else(|| LighError::NotReady("no physical DevDriver connected".into()))?;
        let id = session.next_id;
        session.next_id += 1;
        op.as_object_mut()
            .ok_or_else(|| LighError::NotReady("op must be object".into()))?
            .insert("id".into(), json!(id));
        if writeln!(session.writer, "{op}").is_err() {
            *g = None;
            return Err(LighError::NotReady("device write failed — disconnected".into()));
        }
        let _ = session.writer.flush();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() > deadline {
                return Err(LighError::NotReady("device rpc timeout".into()));
            }
            let mut line = String::new();
            match session.reader.read_line(&mut line) {
                Ok(0) => {
                    *g = None;
                    return Err(LighError::NotReady("device disconnected".into()));
                }
                Ok(_) => {
                    let v: Value = serde_json::from_str(line.trim()).map_err(|e| {
                        LighError::NotReady(format!("device reply json: {e}"))
                    })?;
                    if v.get("op").and_then(|x| x.as_str()) == Some("hello") {
                        continue;
                    }
                    if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                        continue;
                    }
                    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
                        let msg = v
                            .get("error")
                            .and_then(|x| x.as_str())
                            .unwrap_or("device op failed");
                        return Err(LighError::NotReady(msg.into()));
                    }
                    return Ok(v);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    *g = None;
                    return Err(LighError::NotReady(format!("device read: {e}")));
                }
            }
        }
    }
}

impl PhysicalUi for DeviceHub {
    fn active(&self) -> bool {
        self.session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|_| ()))
            .is_some()
    }

    fn session_id(&self) -> String {
        self.session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| format!("device:{}", s.bundle_id)))
            .unwrap_or_else(|| "device".into())
    }

    fn transport(&self) -> &'static str {
        self.session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.transport))
            .unwrap_or("none")
    }

    fn bundle_id(&self) -> Option<String> {
        self.session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.bundle_id.clone()))
    }

    fn screen_points(&self) -> Option<(f64, f64)> {
        self.session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| (s.width, s.height)))
    }

    fn dump(&self) -> Result<Value, LighError> {
        let reply = self.rpc(json!({"op": "dump"}))?;
        reply
            .get("dump")
            .cloned()
            .ok_or_else(|| LighError::NotReady("device dump missing payload".into()))
    }

    fn tap(&self, nx: f64, ny: f64, _width: f64, _height: f64) -> Result<(), LighError> {
        self.rpc(json!({"op": "tap", "nx": nx, "ny": ny}))?;
        Ok(())
    }

    fn tap_hold(
        &self,
        nx: f64,
        ny: f64,
        _width: f64,
        _height: f64,
        hold_ms: f64,
    ) -> Result<(), LighError> {
        self.rpc(json!({"op": "tap_hold", "nx": nx, "ny": ny, "hold_ms": hold_ms}))?;
        Ok(())
    }

    fn swipe(
        &self,
        from_nx: f64,
        from_ny: f64,
        to_nx: f64,
        to_ny: f64,
        _width: f64,
        _height: f64,
    ) -> Result<(), LighError> {
        self.rpc(json!({
            "op": "swipe",
            "from_nx": from_nx, "from_ny": from_ny,
            "to_nx": to_nx, "to_ny": to_ny,
            "duration_ms": 320
        }))?;
        Ok(())
    }

    fn gesture(&self, points: &[Value]) -> Result<(), LighError> {
        self.rpc(json!({
            "op": "gesture",
            "points": points,
        }))?;
        Ok(())
    }

    fn capabilities(&self) -> Value {
        self.snapshot()
            .and_then(|s| s.get("capabilities").cloned())
            .unwrap_or_else(|| json!({}))
    }

    fn driver_version(&self) -> u64 {
        self.snapshot()
            .and_then(|s| s.get("driver_version").and_then(|v| v.as_u64()))
            .unwrap_or(0)
    }

    fn type_text(&self, text: &str) -> Result<(), LighError> {
        self.rpc(json!({"op": "type", "text": text}))?;
        Ok(())
    }

    fn clear(&self, count: u32) -> Result<(), LighError> {
        self.rpc(json!({"op": "clear", "count": count}))?;
        Ok(())
    }

    fn key_named(&self, name: &str) -> Result<(), LighError> {
        self.rpc(json!({"op": "key", "name": name}))?;
        Ok(())
    }

    fn home(&self) -> Result<(), LighError> {
        // Stay inside the instrumented app. Real Home would kill the session.
        let _ = self.rpc(json!({"op": "home"}));
        Ok(())
    }

    fn press_id(&self, id: &str) -> Result<(), LighError> {
        self.rpc(json!({"op": "tap_id", "target": id}))?;
        Ok(())
    }

    fn press_label(&self, label: &str) -> Result<(), LighError> {
        self.rpc(json!({"op": "tap_label", "target": label}))?;
        Ok(())
    }
}

fn advertise_mdns(port: u16, slot: &Mutex<Option<Child>>) {
    match Command::new("dns-sd")
        .args([
            "-R",
            "ligh",
            "_ligh._tcp",
            "local",
            &port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            info!(port, "advertising _ligh._tcp via dns-sd");
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        }
        Err(_) => {}
    }
}

pub fn device_port() -> u16 {
    std::env::var("LIGH_DEVICE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Shared with tests: parse a device hello line.
pub fn parse_hello_line(line: &str) -> Result<(String, f64, f64), String> {
    let v: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
    if v.get("op").and_then(|x| x.as_str()) != Some("hello") {
        return Err("not hello".into());
    }
    let bundle = v
        .get("bundle_id")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let w = v.get("width").and_then(|x| x.as_f64()).unwrap_or(393.0);
    let h = v.get("height").and_then(|x| x.as_f64()).unwrap_or(852.0);
    Ok((bundle, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn parses_expo_style_hello() {
        let (b, w, h) = parse_hello_line(
            r#"{"op":"hello","bundle_id":"dev.ligh.fixture","width":393,"height":852}"#,
        )
        .unwrap();
        assert_eq!(b, "dev.ligh.fixture");
        assert!((w - 393.0).abs() < f64::EPSILON);
        assert!((h - 852.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fake_phone_roundtrip_dump() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let phone = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).ok();
            writeln!(
                stream,
                r#"{{"op":"hello","bundle_id":"dev.ligh.fake","width":390,"height":844}}"#
            )
            .unwrap();
            stream.flush().ok();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut ack = String::new();
            reader.read_line(&mut ack).unwrap();
            let mut op = String::new();
            reader.read_line(&mut op).unwrap();
            let v: Value = serde_json::from_str(op.trim()).unwrap();
            assert_eq!(v.get("op").and_then(|x| x.as_str()), Some("dump"));
            let id = v.get("id").cloned().unwrap_or(json!(1));
            writeln!(
                stream,
                r#"{{"id":{id},"ok":true,"dump":{{"status":"available","elements":[{{"identifier":"sign_in","label":"SIGN IN","role":"Button","hittable":true,"frame":{{"x":20,"y":400,"width":350,"height":48}}}}],"element_count":1,"point_size":{{"width":390,"height":844}}}}}}"#
            )
            .unwrap();
        });

        let hub = DeviceHub::for_test(addr.port());
        let stream = TcpStream::connect(addr).unwrap();
        hub.take_stream(stream, "lan").unwrap();
        assert!(hub.active());
        assert_eq!(hub.bundle_id().as_deref(), Some("dev.ligh.fake"));
        let dump = hub.dump().unwrap();
        assert_eq!(
            dump["elements"][0]["identifier"].as_str(),
            Some("sign_in")
        );
        phone.join().unwrap();
    }
}
