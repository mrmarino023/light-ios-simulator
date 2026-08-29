#!/usr/bin/env python3
"""Session truth — crash/dead process hard-gates for discover / test / TRAIL.

Competitive contract:
  dead + recent .ips  → app_crashed   (never discover_no_chrome)
  dead, no crash      → app_not_running
  alive               → None (proceed)

Agents open DiagnosticReports / atos from the hint — LIGH does not invent Swift blame.
"""

from __future__ import annotations

import json
import os
import subprocess
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
CRASH_RECENT_SECS = 180

# Faults that mean: do NOT TRAIL / do NOT treat as missing chrome.
SESSION_REFUSE_TRAIL = frozenset({"app_crashed", "app_not_running", "eyes_unusable", "sim_boot_hung"})


def _parse_json_blob(blob: str) -> dict[str, Any]:
    blob = (blob or "").strip()
    if blob.startswith("{"):
        try:
            return json.loads(blob)
        except json.JSONDecodeError:
            pass
    for i in range(len(blob) - 1, -1, -1):
        if blob[i] != "{":
            continue
        try:
            return json.loads(blob[i:])
        except json.JSONDecodeError:
            continue
    return {}


def observe_snapshot(*, settle_ms: int = 1500, timeout: float = 90) -> dict[str, Any]:
    p = subprocess.run(
        [LIGH, "--json", "observe", "--settle-ms", str(settle_ms)],
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    blob = (p.stdout or "") + (p.stderr or "")
    raw = _parse_json_blob(blob)
    if isinstance(raw.get("observe"), dict):
        return raw["observe"]
    if isinstance(raw.get("data"), dict) and (
        raw["data"].get("process_health") is not None or raw["data"].get("ax_quality")
    ):
        return raw["data"]
    return raw if isinstance(raw, dict) else {}


def classify_process_health(ph: dict[str, Any] | None) -> str | None:
    """Return fault string or None if session may proceed."""
    if not isinstance(ph, dict) or not ph.get("bundle_id"):
        return None
    if ph.get("running"):
        return None
    if ph.get("crashed_recently"):
        return "app_crashed"
    return "app_not_running"


def gate_from_health(ph: dict[str, Any] | None) -> dict[str, Any] | None:
    """Fail-closed payload if process is dead/crashed; else None."""
    fault = classify_process_health(ph)
    if not fault:
        return None
    hint = (ph or {}).get("hint") or (
        "app_crashed: open DiagnosticReports / atos — not discover_no_chrome"
        if fault == "app_crashed"
        else "app_not_running: relaunch before discover/test/TRAIL"
    )
    return {
        "ok": False,
        "fault": fault,
        "fault_owner": "app",
        "process_health": ph,
        "error": hint,
        "detail": {
            "phase": "session_gate",
            "hint": hint,
            "crash_report_path": (ph or {}).get("crash_report_path"),
            "crash_signal": (ph or {}).get("crash_signal"),
        },
        "repair_allowed": False,
        "trail_allowed": False,
    }


def _launchctl_running(udid: str, bundle_id: str) -> tuple[bool, int | None]:
    """Guest process present in sim launchctl for this bundle."""
    if not udid or not bundle_id:
        return False, None
    try:
        p = subprocess.run(
            ["xcrun", "simctl", "spawn", udid, "launchctl", "list"],
            capture_output=True,
            text=True,
            timeout=20,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False, None
    needle = f"UIKitApplication:{bundle_id}"
    for line in (p.stdout or "").splitlines():
        if needle not in line and bundle_id not in line:
            continue
        parts = line.split()
        pid = None
        if parts:
            try:
                v = int(parts[0])
                if v > 0:
                    pid = v
            except ValueError:
                pass
        if needle in line or pid is not None:
            return True, pid
    return False, None


def _eyes_imply_alive(snap: dict[str, Any], bundle_id: str) -> bool:
    """AX shows app chrome — never invent app_not_running over live eyes."""
    if snap.get("ax_quality") != "ready":
        return False
    if not snap.get("actionable_topk"):
        return False
    if snap.get("app_bundle_id") == bundle_id or snap.get("expected_bundle_id") == bundle_id:
        return True
    label = (snap.get("observed_app_label") or "").lower()
    bid = bundle_id.lower()
    if "mastodon" in bid and "mastodon" in label:
        return True
    # Generic: penultimate or last DNS label appears in observed app name.
    parts = [p for p in bid.split(".") if p]
    for token in reversed(parts[-2:] if len(parts) >= 2 else parts):
        if len(token) >= 4 and token in label:
            return True
    surface = ((snap.get("scene") or {}) if isinstance(snap.get("scene"), dict) else {}).get(
        "surface"
    )
    return surface == "app" and bool(label)


def resolve_process_health(
    snap: dict[str, Any],
    *,
    bundle_id: str | None = None,
) -> dict[str, Any] | None:
    """Prefer daemon process_health; else launchctl + eyes. Never invent death blindly."""
    ph = snap.get("process_health") if isinstance(snap.get("process_health"), dict) else None
    if isinstance(ph, dict) and ph.get("bundle_id"):
        return ph
    if not bundle_id:
        return ph if isinstance(ph, dict) else None
    udid = str(snap.get("udid") or "")
    running, pid = _launchctl_running(udid, bundle_id)
    if not running and _eyes_imply_alive(snap, bundle_id):
        running = True
    return {
        "bundle_id": bundle_id,
        "running": running,
        "pid": pid,
        "crashed_recently": False,
        "hint": None
        if running
        else "app_not_running: expected bundle absent from sim launchctl — relaunch before discover",
        "source": "session_gate_probe",
    }


def gate_session(*, bundle_id: str | None = None, settle_ms: int = 1500) -> dict[str, Any] | None:
    """Observe once and refuse if expected app is dead/crashed."""
    snap = observe_snapshot(settle_ms=settle_ms)
    ph = resolve_process_health(snap, bundle_id=bundle_id)
    blocked = gate_from_health(ph)
    if blocked:
        blocked["observe"] = {
            "ax_quality": snap.get("ax_quality"),
            "overlay": snap.get("overlay"),
            "ax_source": snap.get("ax_source"),
            "system_surface": snap.get("system_surface"),
            "screen_sig": snap.get("screen_sig"),
            "observed_app_label": snap.get("observed_app_label"),
        }
    return blocked


def trail_allowed(fault: str | None, *, process_health: dict[str, Any] | None = None) -> bool:
    """TRAIL may edit Swift only when the app process is alive and fault is app-owned."""
    if fault in SESSION_REFUSE_TRAIL:
        return False
    if classify_process_health(process_health) is not None:
        return False
    return True


def write_certify_artifact(
    workspace: str,
    payload: dict[str, Any],
) -> str:
    """Always write `.ligh/last-certify.json` — the competitive product surface."""
    ligh = os.path.join(os.path.abspath(workspace), ".ligh")
    os.makedirs(ligh, exist_ok=True)
    path = os.path.join(ligh, "last-certify.json")
    doc = {
        "schema": 1,
        "capability": payload.get("capability") or "ligh_test",
        "ok": bool(payload.get("ok")),
        "fault": payload.get("fault"),
        "fault_owner": payload.get("fault_owner"),
        "mode": payload.get("mode"),
        "workspace": os.path.abspath(workspace),
        "app": payload.get("app"),
        "bundle_id": payload.get("bundle_id"),
        "process_health": payload.get("process_health"),
        "system_surface": payload.get("system_surface"),
        "overlay": payload.get("overlay"),
        "screen_sig": payload.get("screen_sig"),
        "ax_source": payload.get("ax_source"),
        "detail": payload.get("detail"),
        "trail_allowed": payload.get("trail_allowed"),
        "repair_allowed": payload.get("repair_allowed"),
        "ts": int(time.time()),
    }
    # Drop Nones for cleaner diffs
    doc = {k: v for k, v in doc.items() if v is not None}
    with open(path, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    return path


def enrich_certify_from_observe(out: dict[str, Any], snap: dict[str, Any] | None) -> dict[str, Any]:
    if not isinstance(snap, dict):
        return out
    for key in ("process_health", "system_surface", "overlay", "screen_sig", "ax_source"):
        if snap.get(key) is not None and out.get(key) is None:
            out[key] = snap[key]
    return out
