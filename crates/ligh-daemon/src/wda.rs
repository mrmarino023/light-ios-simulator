//! Physical arms — Appium XCUITest / WebDriverAgent.
//!
//! DevDriver fake UITouch ACK'd without moving RN UI. WDA injects real
//! system taps/swipes (the same hand Appium uses). Eyes can stay on the
//! DevDriver AX dump; arms go through this client when a session is live.
//!
//! Env:
//!   LIGH_WDA_URL   default http://127.0.0.1:4723
//!   LIGH_WDA_UDID  required for create (or auto from connected device)
//!   LIGH_WDA_BUNDLE optional bundleId to attach (e.g. com.mattisky999.MaeApp)

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

use ligh_core::LighError;
use serde_json::{json, Value};
use tracing::{info, warn};

#[derive(Debug)]
pub struct WdaSession {
    base: String,
    sid: String,
    width: f64,
    height: f64,
}

pub struct WdaArms {
    inner: Mutex<Option<WdaSession>>,
}

impl WdaArms {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub fn active(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|_| true))
            .unwrap_or(false)
    }

    pub fn snapshot(&self) -> Option<Value> {
        let g = self.inner.lock().ok()?;
        let s = g.as_ref()?;
        Some(json!({
            "connected": true,
            "motor": "wda_appium",
            "session_id": s.sid,
            "base": s.base,
            "screen": { "width": s.width, "height": s.height },
        }))
    }

    /// Ensure a WDA session exists. Safe to call often.
    pub fn ensure(&self, udid: &str, bundle_id: Option<&str>) -> Result<(), LighError> {
        load_wda_dotenv();
        {
            let g = self.inner.lock().map_err(|_| LighError::NotReady("wda lock".into()))?;
            if g.is_some() {
                return Ok(());
            }
        }
        // Reuse an already-running Appium session if provided.
        if let Ok(sid) = std::env::var("LIGH_WDA_SESSION") {
            if !sid.is_empty() {
                let base = WdaSession::base_url();
                let mut sess = WdaSession {
                    base: base.clone(),
                    sid: sid.clone(),
                    width: 390.0,
                    height: 844.0,
                };
                if let Ok((w, h)) = sess.window_size() {
                    sess.width = w;
                    sess.height = h;
                }
                info!(%sid, "WDA arms attached to existing Appium session");
                *self
                    .inner
                    .lock()
                    .map_err(|_| LighError::NotReady("wda lock".into()))? = Some(sess);
                return Ok(());
            }
        }
        let sess = WdaSession::create(udid, bundle_id)?;
        info!(
            sid = %sess.sid,
            w = sess.width,
            h = sess.height,
            "WDA arms online (physical motor)"
        );
        *self
            .inner
            .lock()
            .map_err(|_| LighError::NotReady("wda lock".into()))? = Some(sess);
        Ok(())
    }

    fn with_sess<T>(&self, f: impl FnOnce(&WdaSession) -> Result<T, LighError>) -> Result<T, LighError> {
        let g = self
            .inner
            .lock()
            .map_err(|_| LighError::NotReady("wda lock".into()))?;
        let s = g.as_ref().ok_or_else(|| {
            LighError::NotReady(
                "WDA arms offline — start Appium (APPIUM_HOME=.appium appium) and set LIGH_WDA_UDID"
                    .into(),
            )
        })?;
        f(s)
    }

    pub fn tap_norm(&self, nx: f64, ny: f64) -> Result<(), LighError> {
        self.with_sess(|s| {
            let x = nx.clamp(0.0, 1.0) * s.width;
            let y = ny.clamp(0.0, 1.0) * s.height;
            s.tap_xy(x, y)
        })
    }

    pub fn tap_hold_norm(&self, nx: f64, ny: f64, hold_ms: f64) -> Result<(), LighError> {
        self.with_sess(|s| {
            let x = nx.clamp(0.0, 1.0) * s.width;
            let y = ny.clamp(0.0, 1.0) * s.height;
            s.tap_hold_xy(x, y, hold_ms)
        })
    }

    pub fn swipe_norm(
        &self,
        from_nx: f64,
        from_ny: f64,
        to_nx: f64,
        to_ny: f64,
        duration_ms: f64,
    ) -> Result<(), LighError> {
        self.with_sess(|s| {
            s.swipe_xy(
                from_nx.clamp(0.0, 1.0) * s.width,
                from_ny.clamp(0.0, 1.0) * s.height,
                to_nx.clamp(0.0, 1.0) * s.width,
                to_ny.clamp(0.0, 1.0) * s.height,
                duration_ms,
            )
        })
    }

    pub fn click_label(&self, label: &str) -> Result<(), LighError> {
        self.with_sess(|s| s.click_label(label))
    }

    pub fn click_id(&self, id: &str) -> Result<(), LighError> {
        self.with_sess(|s| s.click_id(id))
    }

    pub fn type_text(&self, text: &str) -> Result<(), LighError> {
        self.with_sess(|s| s.type_text(text))
    }

    pub fn clear(&self, count: u32) -> Result<(), LighError> {
        self.with_sess(|s| s.clear(count))
    }

    pub fn key_named(&self, name: &str) -> Result<(), LighError> {
        self.with_sess(|s| s.key_named(name))
    }

    pub fn home(&self) -> Result<(), LighError> {
        self.with_sess(|s| s.home())
    }

    /// Human gesture path: points as `{nx,ny,t_ms,phase?}`.
    pub fn gesture(&self, points: &[Value]) -> Result<(), LighError> {
        self.with_sess(|s| s.gesture_norm(points))
    }

    pub fn screen(&self) -> Option<(f64, f64)> {
        let g = self.inner.lock().ok()?;
        let s = g.as_ref()?;
        Some((s.width, s.height))
    }
}

impl WdaSession {
    fn base_url() -> String {
        std::env::var("LIGH_WDA_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:4723".into())
            .trim_end_matches('/')
            .to_string()
    }

    fn create(udid: &str, bundle_id: Option<&str>) -> Result<Self, LighError> {
        load_wda_dotenv();
        let base0 = Self::base_url();
        let mut always = json!({
            "platformName": "iOS",
            "appium:automationName": "XCUITest",
            "appium:udid": udid,
            "appium:deviceName": "iPhone",
            "appium:noReset": true,
            "appium:newCommandTimeout": 300,
            "appium:wdaLaunchTimeout": 180000,
            "appium:wdaConnectionTimeout": 180000,
            "appium:skipLogCapture": true,
            "appium:usePrebuiltWDA": false,
            "appium:showXcodeLog": false,
        });
        if let Some(b) = bundle_id {
            always["appium:bundleId"] = json!(b);
        }
        if let Ok(team) = std::env::var("LIGH_WDA_XCODE_ORG_ID") {
            if !team.is_empty() {
                always["appium:xcodeOrgId"] = json!(team);
                always["appium:xcodeSigningId"] = json!(
                    std::env::var("LIGH_WDA_XCODE_SIGNING_ID")
                        .unwrap_or_else(|_| "Apple Development".into())
                );
                always["appium:usePrebuiltWDA"] = json!(true);
                always["appium:allowProvisioningDeviceRegistration"] = json!(true);
            }
        }
        let body = json!({ "capabilities": { "alwaysMatch": always } });

        let mut last = LighError::NotReady("WDA session create failed".into());
        for prefix in ["", "/wd/hub"] {
            let base = if prefix.is_empty() {
                base0.clone()
            } else {
                format!("{base0}{prefix}")
            };
            match http_json("POST", &format!("{base}/session"), Some(&body), 300) {
                Ok(resp) => {
                    let sid = extract_session_id(&resp).ok_or_else(|| {
                        LighError::NotReady(format!("WDA session id missing: {resp}"))
                    })?;
                    let mut sess = Self {
                        base,
                        sid,
                        width: 390.0,
                        height: 844.0,
                    };
                    if let Ok((w, h)) = sess.window_size() {
                        sess.width = w;
                        sess.height = h;
                    }
                    return Ok(sess);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn path(&self, rel: &str) -> String {
        format!("{}/session/{}{}", self.base, self.sid, rel)
    }

    fn window_size(&self) -> Result<(f64, f64), LighError> {
        let v = http_json("GET", &self.path("/window/rect"), None, 30)?;
        let val = v.get("value").cloned().unwrap_or(v);
        let w = val
            .get("width")
            .and_then(|x| x.as_f64())
            .ok_or_else(|| LighError::NotReady("WDA window width".into()))?;
        let h = val
            .get("height")
            .and_then(|x| x.as_f64())
            .ok_or_else(|| LighError::NotReady("WDA window height".into()))?;
        Ok((w, h))
    }

    fn execute(&self, script: &str, args: Value) -> Result<Value, LighError> {
        http_json(
            "POST",
            &self.path("/execute/sync"),
            Some(&json!({ "script": script, "args": args })),
            60,
        )
    }

    fn tap_xy(&self, x: f64, y: f64) -> Result<(), LighError> {
        // Prefer mobile: tap — reliable on XCUITest.
        match self.execute("mobile: tap", json!([{ "x": x, "y": y }])) {
            Ok(_) => return Ok(()),
            Err(e) => warn!(error=%e, "mobile: tap failed; trying W3C actions"),
        }
        self.w3c_pointer(&[
            ("pointerMove", x, y, 0),
            ("pointerDown", x, y, 0),
            ("pause", x, y, 80),
            ("pointerUp", x, y, 0),
        ])
    }

    fn tap_hold_xy(&self, x: f64, y: f64, hold_ms: f64) -> Result<(), LighError> {
        let hold = hold_ms.max(200.0);
        match self.execute(
            "mobile: touchAndHold",
            json!([{ "x": x, "y": y, "duration": hold / 1000.0 }]),
        ) {
            Ok(_) => Ok(()),
            Err(_) => self.w3c_pointer(&[
                ("pointerMove", x, y, 0),
                ("pointerDown", x, y, 0),
                ("pause", x, y, hold as u64),
                ("pointerUp", x, y, 0),
            ]),
        }
    }

    fn swipe_xy(
        &self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        duration_ms: f64,
    ) -> Result<(), LighError> {
        let dur = (duration_ms.max(80.0) / 1000.0).max(0.08);
        match self.execute(
            "mobile: dragFromToForDuration",
            json!([{
                "fromX": x1, "fromY": y1,
                "toX": x2, "toY": y2,
                "duration": dur,
            }]),
        ) {
            Ok(_) => Ok(()),
            Err(_) => {
                let steps = 12u64;
                let mut acts = vec![
                    ("pointerMove", x1, y1, 0u64),
                    ("pointerDown", x1, y1, 0),
                ];
                for i in 1..=steps {
                    let u = i as f64 / steps as f64;
                    let x = x1 + (x2 - x1) * u;
                    let y = y1 + (y2 - y1) * u;
                    let t = ((duration_ms / steps as f64) as u64).max(8);
                    acts.push(("pointerMove", x, y, t));
                }
                acts.push(("pointerUp", x2, y2, 0));
                self.w3c_pointer(&acts)
            }
        }
    }

    fn w3c_pointer(&self, steps: &[(&str, f64, f64, u64)]) -> Result<(), LighError> {
        let mut actions = Vec::new();
        for (kind, x, y, ms) in steps {
            match *kind {
                "pointerMove" => {
                    let mut a = json!({
                        "type": "pointerMove",
                        "duration": ms,
                        "x": x,
                        "y": y,
                    });
                    if actions.is_empty() {
                        a["origin"] = json!("viewport");
                    }
                    actions.push(a);
                }
                "pointerDown" => actions.push(json!({ "type": "pointerDown", "button": 0 })),
                "pointerUp" => actions.push(json!({ "type": "pointerUp", "button": 0 })),
                "pause" => actions.push(json!({ "type": "pause", "duration": ms })),
                _ => {}
            }
        }
        http_json(
            "POST",
            &self.path("/actions"),
            Some(&json!({
                "actions": [{
                    "type": "pointer",
                    "id": "finger1",
                    "parameters": { "pointerType": "touch" },
                    "actions": actions,
                }]
            })),
            60,
        )?;
        let _ = http_json("DELETE", &self.path("/actions"), None, 15);
        Ok(())
    }

    fn gesture_norm(&self, points: &[Value]) -> Result<(), LighError> {
        if points.is_empty() {
            return Ok(());
        }
        let mut acts: Vec<(&str, f64, f64, u64)> = Vec::new();
        let mut prev_t = 0.0f64;
        for (i, p) in points.iter().enumerate() {
            let nx = p.get("nx").and_then(|v| v.as_f64()).unwrap_or(0.5);
            let ny = p.get("ny").and_then(|v| v.as_f64()).unwrap_or(0.5);
            let x = nx.clamp(0.0, 1.0) * self.width;
            let y = ny.clamp(0.0, 1.0) * self.height;
            let t = p
                .get("t_ms")
                .or_else(|| p.get("t"))
                .and_then(|v| v.as_f64())
                .unwrap_or(prev_t);
            let dt = ((t - prev_t).max(0.0) as u64).min(2000);
            prev_t = t;
            let phase = p
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or(if i == 0 {
                    "began"
                } else if i + 1 == points.len() {
                    "ended"
                } else {
                    "moved"
                });
            match phase {
                "began" | "down" => {
                    acts.push(("pointerMove", x, y, 0));
                    acts.push(("pointerDown", x, y, 0));
                }
                "ended" | "up" => {
                    acts.push(("pointerMove", x, y, dt));
                    acts.push(("pointerUp", x, y, 0));
                }
                "cancelled" | "cancel" => {
                    acts.push(("pointerUp", x, y, 0));
                }
                _ => acts.push(("pointerMove", x, y, dt.max(8))),
            }
        }
        // Convert to owned for w3c — rebuild as Value path via helper
        let owned: Vec<(String, f64, f64, u64)> = acts
            .into_iter()
            .map(|(k, x, y, t)| (k.to_string(), x, y, t))
            .collect();
        let refs: Vec<(&str, f64, f64, u64)> = owned
            .iter()
            .map(|(k, x, y, t)| (k.as_str(), *x, *y, *t))
            .collect();
        self.w3c_pointer(&refs)
    }

    fn find(&self, using: &str, value: &str) -> Result<String, LighError> {
        let resp = http_json(
            "POST",
            &self.path("/element"),
            Some(&json!({ "using": using, "value": value })),
            30,
        )?;
        let val = resp.get("value").cloned().unwrap_or(resp);
        if let Some(id) = val.get("ELEMENT").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
        if let Some(id) = val
            .get("element-6066-11e4-a52e-4f735466cecf")
            .and_then(|v| v.as_str())
        {
            return Ok(id.to_string());
        }
        Err(LighError::NotReady(format!("WDA element not found: {value}")))
    }

    fn click_element(&self, eid: &str) -> Result<(), LighError> {
        http_json(
            "POST",
            &self.path(&format!("/element/{eid}/click")),
            Some(&json!({})),
            30,
        )?;
        Ok(())
    }

    fn click_label(&self, label: &str) -> Result<(), LighError> {
        let escaped = label.replace('\\', "\\\\").replace('\'', "\\'");
        // Prefer exact, then contains (VoiceOver-style "Name, tab, 1 of 5").
        let preds = [
            format!("name == '{escaped}' OR label == '{escaped}'"),
            format!("name CONTAINS '{escaped}' OR label CONTAINS '{escaped}'"),
        ];
        let mut last = LighError::NotReady(format!("WDA label not found: {label}"));
        for pred in preds {
            match self.find("-ios predicate string", &pred) {
                Ok(eid) => return self.click_element(&eid),
                Err(e) => last = e,
            }
        }
        // Fallback: coordinate from accessibility — not available; fail.
        Err(last)
    }

    fn click_id(&self, id: &str) -> Result<(), LighError> {
        let escaped = id.replace('\\', "\\\\").replace('\'', "\\'");
        let pred = format!("name == '{escaped}' OR label == '{escaped}' OR value == '{escaped}'");
        let eid = self.find("-ios predicate string", &pred)?;
        self.click_element(&eid)
    }

    fn type_text(&self, text: &str) -> Result<(), LighError> {
        match self.execute("mobile: type", json!([{ "text": text }])) {
            Ok(_) => Ok(()),
            Err(_) => {
                http_json(
                    "POST",
                    &self.path("/keys"),
                    Some(&json!({ "value": text.chars().map(|c| c.to_string()).collect::<Vec<_>>() })),
                    30,
                )?;
                Ok(())
            }
        }
    }

    fn clear(&self, count: u32) -> Result<(), LighError> {
        for _ in 0..count.max(1) {
            let _ = self.execute("mobile: keys", json!([{ "keys": ["\u{0008}"] }]));
        }
        Ok(())
    }

    fn key_named(&self, name: &str) -> Result<(), LighError> {
        let key = match name.to_ascii_lowercase().as_str() {
            "return" | "enter" => "\n",
            "space" => " ",
            "delete" | "backspace" => "\u{0008}",
            "tab" => "\t",
            "escape" => "\u{001b}",
            other => {
                return self.execute("mobile: pressButton", json!([{ "name": other }])).map(|_| ());
            }
        };
        self.type_text(key)
    }

    fn home(&self) -> Result<(), LighError> {
        self.execute("mobile: pressButton", json!([{ "name": "home" }]))
            .map(|_| ())
    }
}

fn extract_session_id(resp: &Value) -> Option<String> {
    if let Some(s) = resp.get("sessionId").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    let val = resp.get("value")?;
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    if let Some(s) = val.get("sessionId").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    None
}

fn http_json(
    method: &str,
    url: &str,
    body: Option<&Value>,
    timeout_secs: u64,
) -> Result<Value, LighError> {
    let (host, port, path) = parse_http_url(url)?;
    let payload = body.map(|b| b.to_string());
    let addr = format!("{host}:{port}");
    let stream = TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| LighError::NotReady(format!("WDA addr: {e}")))?,
        Duration::from_secs(timeout_secs.min(30).max(3)),
    )
    .map_err(|e| LighError::NotReady(format!("WDA connect {addr}: {e}")))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout_secs)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .ok();

    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if let Some(ref p) = payload {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", p.len()));
    }
    req.push_str("\r\n");
    if let Some(p) = payload {
        req.push_str(&p);
    }

    let mut stream = stream;
    stream
        .write_all(req.as_bytes())
        .map_err(|e| LighError::NotReady(format!("WDA write: {e}")))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| LighError::NotReady(format!("WDA read: {e}")))?;
    let text = String::from_utf8_lossy(&buf);
    let Some(pos) = text.find("\r\n\r\n") else {
        return Err(LighError::NotReady(format!(
            "WDA bad HTTP response: {}",
            text.chars().take(200).collect::<String>()
        )));
    };
    let (head, body_s) = text.split_at(pos + 4);
    let status = head
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("0");
    let status_n: u16 = status.parse().unwrap_or(0);
    let body_trim = body_s.trim();
    if body_trim.is_empty() {
        if (200..300).contains(&status_n) {
            return Ok(json!({}));
        }
        return Err(LighError::NotReady(format!("WDA HTTP {status} empty")));
    }
    let val: Value = serde_json::from_str(body_trim).map_err(|e| {
        LighError::NotReady(format!(
            "WDA JSON ({status}): {e} — {}",
            body_trim.chars().take(240).collect::<String>()
        ))
    })?;
    if !(200..300).contains(&status_n) {
        return Err(LighError::NotReady(format!("WDA HTTP {status}: {val}")));
    }
    if let Some(err) = val.get("value").and_then(|v| v.get("error")) {
        return Err(LighError::NotReady(format!("WDA error: {err} / {val}")));
    }
    Ok(val)
}

pub(crate) fn load_wda_dotenv() {
    let path = dirs::home_dir()
        .map(|h| h.join(".ligh/wda.env"))
        .unwrap_or_else(|| std::path::PathBuf::from(".ligh/wda.env"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }
}

fn parse_http_url(url: &str) -> Result<(String, u16, String), LighError> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| LighError::NotReady(format!("WDA url must be http(s): {url}")))?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = if let Some((h, p)) = hostport.rsplit_once(':') {
        (
            h.to_string(),
            p.parse()
                .map_err(|_| LighError::NotReady(format!("bad port in {url}")))?,
        )
    } else {
        (hostport.to_string(), 80u16)
    };
    Ok((host, port, path.to_string()))
}
