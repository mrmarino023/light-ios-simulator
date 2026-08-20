#!/usr/bin/env python3
"""Same semantic agent workflow as `ligh bench agent`, via Appium XCUITest (WDA).

Env: UDID (required), STEPS=40, APPIUM_URL=http://127.0.0.1:4723
Prints one JSON object to stdout.
"""
from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request
from typing import Any

UDID = os.environ.get("UDID")
if not UDID:
    print(json.dumps({"ok": False, "error": "UDID required", "path": "appium_xcuitest_wda"}))
    sys.exit(2)

STEPS = max(20, min(60, int(os.environ.get("STEPS", "40"))))
BASE = os.environ.get("APPIUM_URL", "http://127.0.0.1:4723").rstrip("/")


def req(method: str, path: str, body: dict | None = None, timeout: float = 180.0) -> dict:
    data = None if body is None else json.dumps(body).encode()
    r = urllib.request.Request(
        f"{BASE}{path}",
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(r, timeout=timeout) as resp:
        raw = resp.read().decode()
        return json.loads(raw) if raw else {}


def val(resp: dict) -> Any:
    return resp.get("value")


class Driver:
    def __init__(self, session_id: str):
        self.sid = session_id

    def delete(self) -> None:
        try:
            req("DELETE", f"/session/{self.sid}", timeout=30)
        except Exception:
            pass

    def execute(self, script: str, args: list) -> Any:
        return val(
            req(
                "POST",
                f"/session/{self.sid}/execute/sync",
                {"script": script, "args": args},
            )
        )

    def page_source(self) -> str:
        return val(req("GET", f"/session/{self.sid}/source")) or ""

    def screenshot(self) -> str:
        return val(req("GET", f"/session/{self.sid}/screenshot")) or ""

    def find(self, using: str, value: str) -> str | None:
        try:
            el = val(
                req(
                    "POST",
                    f"/session/{self.sid}/element",
                    {"using": using, "value": value},
                )
            )
            if isinstance(el, dict):
                return el.get("ELEMENT") or el.get("element-6066-11e4-a52e-4f735466cecf")
        except Exception:
            return None
        return None

    def click(self, element_id: str) -> None:
        req("POST", f"/session/{self.sid}/element/{element_id}/click", {})

    def keys(self, text: str) -> None:
        # W3C actions keyboard is heavy; use mobile: type if available, else element sendKeys
        try:
            self.execute("mobile: type", [{"text": text}])
        except Exception:
            req(
                "POST",
                f"/session/{self.sid}/keys",
                {"value": list(text)},
            )

    def exists_name(self, name: str) -> bool:
        pred = f"name == '{name}' OR label == '{name}'"
        return self.find("-ios predicate string", pred) is not None

    def wait_name(self, name: str, timeout_ms: int) -> bool:
        deadline = time.time() + timeout_ms / 1000.0
        while time.time() < deadline:
            if self.exists_name(name):
                return True
            time.sleep(0.15)
        return False

    def click_name(self, name: str, timeout_ms: int = 4000) -> bool:
        if not self.wait_name(name, timeout_ms):
            return False
        pred = f"name == '{name}' OR label == '{name}'"
        eid = self.find("-ios predicate string", pred)
        if not eid:
            return False
        self.click(eid)
        return True


def create_session() -> Driver:
    caps = {
        "capabilities": {
            "alwaysMatch": {
                "platformName": "iOS",
                "appium:automationName": "XCUITest",
                "appium:udid": UDID,
                "appium:deviceName": "iPhone",
                "appium:noReset": True,
                "appium:newCommandTimeout": 180,
                "appium:wdaLaunchTimeout": 180000,
                "appium:wdaConnectionTimeout": 180000,
                "appium:skipLogCapture": True,
            }
        }
    }
    # Appium 2 may want /session on base; some proxies use /wd/hub
    last_err = None
    for prefix in ("", "/wd/hub"):
        try:
            resp = req("POST", f"{prefix}/session", caps, timeout=300)
            sid = val(resp)
            if isinstance(sid, dict):
                sid = sid.get("sessionId") or sid.get("session_id")
            if not sid and "sessionId" in resp:
                sid = resp["sessionId"]
            if sid:
                # stash prefix on driver via closure — rebuild BASE
                global BASE
                if prefix:
                    BASE = BASE + prefix if not BASE.endswith(prefix) else BASE
                return Driver(str(sid))
        except Exception as e:
            last_err = e
    raise RuntimeError(f"create session failed: {last_err}")


def main() -> int:
    labels = {
        "settings": os.environ.get("LABEL_SETTINGS", "Impostazioni"),
        "general": os.environ.get("LABEL_GENERAL", "Generali"),
        "search": os.environ.get("LABEL_SEARCH", "Cerca"),
        "safari": os.environ.get("LABEL_SAFARI", "Safari"),
    }
    steps: list[dict] = []
    by_class: dict[str, list[float]] = {}
    hard_fail = None
    cycle = 0
    wall0 = time.time()

    def push(op: str, ok: bool, ms: float, detail: str) -> None:
        by_class.setdefault(op, []).append(ms)
        steps.append({"i": len(steps) + 1, "op": op, "ok": ok, "ms": ms, "detail": detail})

    def timed(op: str, fn) -> bool:
        t0 = time.time()
        try:
            detail = fn() or ""
            push(op, True, (time.time() - t0) * 1000.0, str(detail))
            return True
        except Exception as e:
            push(op, False, (time.time() - t0) * 1000.0, str(e))
            return False

    try:
        d = create_session()
    except Exception as e:
        print(
            json.dumps(
                {
                    "ok": False,
                    "path": "appium_xcuitest_wda",
                    "error": str(e),
                    "wall_ms": None,
                    "note": "Could not create Appium/WDA session",
                }
            )
        )
        return 1

    try:
        if d.exists_name("Settings"):
            labels = {
                "settings": "Settings",
                "general": "General",
                "search": "Search",
                "safari": "Safari",
            }

        while len(steps) < STEPS and hard_fail is None:
            cycle += 1

            if not timed(
                "home",
                lambda: (
                    d.execute("mobile: pressButton", [{"name": "home"}]),
                    time.sleep(0.2),
                    d.execute("mobile: pressButton", [{"name": "home"}]),
                    time.sleep(0.2),
                    f"cycle={cycle}",
                )[-1],
            ):
                hard_fail = "home"
                break

            def springboard():
                if d.exists_name(labels["safari"]) or d.exists_name(labels["settings"]):
                    return "springboard"
                if d.wait_name(labels["settings"], 2500):
                    return labels["settings"]
                raise TimeoutError("not on SpringBoard")

            ok = timed("wait", springboard)
            if not ok and cycle > 1:
                hard_fail = "springboard"
                break

            timed("observe", lambda: f"{len(d.page_source())}B")
            if len(steps) >= STEPS:
                break

            if not timed(
                "wait",
                lambda: labels["settings"]
                if d.wait_name(labels["settings"], 4000)
                else (_ for _ in ()).throw(TimeoutError(labels["settings"])),
            ):
                hard_fail = f"wait {labels['settings']}"
                break

            def open_settings():
                try:
                    d.execute("mobile: launchApp", [{"bundleId": "com.apple.Preferences"}])
                    time.sleep(0.5)
                    return "launchApp Preferences"
                except Exception:
                    if not d.click_name(labels["settings"], 3000):
                        raise TimeoutError("open settings")
                    time.sleep(0.4)
                    return "click icon"

            if not timed("tap_label", open_settings):
                hard_fail = "open Settings"
                break

            def root():
                if d.exists_name(labels["general"]) or d.exists_name("Bluetooth"):
                    return "root"
                if d.wait_name(labels["general"], 8000):
                    return labels["general"]
                raise TimeoutError("settings root")

            if not timed("wait", root):
                hard_fail = f"wait {labels['general']}"
                break

            timed("observe", lambda: f"{len(d.page_source())}B")

            if not timed(
                "tap_label",
                lambda: labels["search"]
                if d.click_name(labels["search"], 4000)
                else (_ for _ in ()).throw(TimeoutError(labels["search"])),
            ):
                hard_fail = f"tap {labels['search']}"
                break
            time.sleep(0.15)

            if not timed(
                "type",
                lambda: (d.keys("ligh"), time.sleep(0.15), "ligh")[-1],
            ):
                hard_fail = "type"
                break

            def assert_typed():
                src = d.page_source().lower()
                return "has_ligh" if "ligh" in src else "no_ligh_token_in_source"

            timed("assert", assert_typed)
            timed("screenshot", lambda: ("wda_screenshot", d.screenshot()[:16])[0])

    finally:
        d.delete()

    wall_ms = (time.time() - wall0) * 1000.0
    passed = sum(1 for s in steps if s["ok"])
    failed = sum(1 for s in steps if not s["ok"])
    per_op = {}
    for k, arr in by_class.items():
        xs = sorted(arr)
        def pct(p):
            if not xs:
                return None
            return xs[int(round((len(xs) - 1) * p))]
        per_op[k] = {"n": len(xs), "p50_ms": pct(0.5), "p95_ms": pct(0.95)}

    print(
        json.dumps(
            {
                "name": "WDA/Appium XCUITest — same semantic workflow",
                "path": "appium_xcuitest_wda",
                "ok": hard_fail is None and failed == 0,
                "hard_fail": hard_fail,
                "wall_ms": wall_ms,
                "steps_run": len(steps),
                "cycles": cycle,
                "passed": passed,
                "failed": failed,
                "failure_rate": (failed / len(steps)) if steps else 1.0,
                "locale_labels": labels,
                "per_op": per_op,
                "steps": steps,
                "note": "External best-in-class baseline. Not MCP carp.",
            }
        )
    )
    return 0 if hard_fail is None and failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
