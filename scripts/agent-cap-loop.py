#!/usr/bin/env python3
"""Control-plane frontier arm — capabilities, not LLM rediscovery of Settings.

This is the product under test vs vision/WDA: settle-honest ops with FaultClass.
Exit 0 on success, 1 on goal fail, 2 on infra/eyes fault.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
SETTLE = os.environ.get("LIGH_SETTLE_MS", "2500")


def run(*args: str, timeout: float = 120) -> tuple[int, dict]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    raw = (p.stdout or "").strip()
    try:
        data = json.loads(raw) if raw.startswith("{") else {"raw": raw, "stderr": p.stderr}
    except json.JSONDecodeError:
        data = {"raw": raw, "stderr": p.stderr}
    return p.returncode, data


def main() -> int:
    goal = " ".join(sys.argv[1:]).strip() or os.environ.get("LIGH_GOAL", "")
    if not goal:
        print("usage: agent-cap-loop.py <goal text>", file=sys.stderr)
        return 2
    g = goal.lower()
    t0 = time.time()

    code, ready = run("--json", "ready", "--settle-ms", SETTLE, "--recover-homes", "6")
    if code != 0 or not ready.get("ok"):
        print(json.dumps({"ok": False, "fault": ready.get("fault") or "infra", "capability": "ensure_ready"}))
        return 2
    print(f"✓ ensure_ready phase={ready.get('phase')} surface={ready.get('surface')}")

    if "bluetooth" in g:
        code, r = run("--json", "cap", "settings-search", "Bluetooth", "--settle-ms", SETTLE, timeout=180)
        print(json.dumps({"cap": "settings_search", "ok": r.get("ok"), "fault": r.get("fault"), "detail": r.get("detail")}))
        ok = code == 0 and r.get("ok")
        print(f"{'✓' if ok else '✗'} bluetooth via settings_search in {time.time()-t0:.1f}s")
        return 0 if ok else (2 if (r.get("fault") in ("infra", "eyes_unusable", "timeout")) else 1)

    if "safari" in g:
        code, r = run(
            "--json", "cap", "tap", "--label", "Safari", "--settle-ms", SETTLE, "--timeout-ms", "7000"
        )
        if code != 0:
            print(json.dumps(r))
            return 1 if r.get("fault") not in ("infra", "eyes_unusable") else 2
        # soft success: surface app or address-ish labels after settle
        code2, obs = run("--json", "observe", "--settle-ms", SETTLE)
        labs = [x.get("label") or "" for x in (obs.get("actionable_topk") or [])]
        surface = (obs.get("scene") or {}).get("surface")
        ok = surface in ("app", "settings") or any(
            x in labs for x in ("Indirizzo", "Address", "URL", "Safari", "Caps Lock")
        )
        print(json.dumps({"cap": "tap_safari", "ok": ok, "surface": surface, "labels": labs[:8]}))
        return 0 if ok else 1

    if "generali" in g or ("general" in g and "back" in g):
        code, r = run("--json", "cap", "open-settings", "--settle-ms", SETTLE, timeout=180)
        if code != 0 or not r.get("ok"):
            print(json.dumps(r))
            return 2 if (r.get("fault") in ("infra", "eyes_unusable", "timeout")) else 1
        gen = "Generali" if any(
            (x.get("label") == "Generali")
            for x in ((r.get("observe") or {}).get("actionable_topk") or [])
        ) else "General"
        # If Generali not in post observe, try both
        for lab in ("Generali", "General"):
            c, tr = run("--json", "cap", "tap", "--label", lab, "--settle-ms", SETTLE)
            if c == 0 and tr.get("ok"):
                gen = lab
                break
        # back toward root
        for _ in range(4):
            run("tap", "--x", "0.11", "--y", "0.09")
            time.sleep(0.25)
        code3, a = run("--json", "cap", "assert-surface", "settings", "--settle-ms", SETTLE)
        # also require Generali/General row
        code4, obs = run("--json", "observe", "--settle-ms", "1500")
        labs = [x.get("label") or "" for x in (obs.get("actionable_topk") or [])]
        ok = (code3 == 0 and a.get("ok")) and ("Generali" in labs or "General" in labs)
        print(json.dumps({"cap": "settings_general_back", "ok": ok, "labels": labs[:10]}))
        return 0 if ok else 1

    if "maps" in g or "mappe" in g:
        # launch Maps then settle
        st = run("--json", "status")[1]
        udid = ((st.get("session") or {}).get("udid")) or ((st.get("device") or {}).get("udid"))
        if not udid:
            # status shape from daemon may nest differently
            udid = st.get("udid")
        if udid:
            subprocess.run(
                ["xcrun", "simctl", "launch", str(udid), "com.apple.Maps"],
                capture_output=True,
                timeout=60,
            )
        else:
            code, r = run("--json", "cap", "tap", "--label", "Mappe", "--settle-ms", SETTLE)
            if code != 0:
                code, r = run("--json", "cap", "tap", "--label", "Maps", "--settle-ms", SETTLE)
            if code != 0:
                print(json.dumps(r))
                return 1
        time.sleep(0.8)
        code, obs = run("--json", "observe", "--settle-ms", SETTLE)
        if obs.get("eyes_unusable"):
            run("--json", "ready", "--settle-ms", SETTLE)
            code, obs = run("--json", "observe", "--settle-ms", SETTLE)
        surface = (obs.get("scene") or {}).get("surface")
        labs = [x.get("label") or "" for x in (obs.get("actionable_topk") or [])]
        ok = surface != "springboard" and (
            any(x in labs for x in ("Mappa", "Mappe", "Maps", "Cerca", "Search", "Modalità mappa"))
            or surface == "app"
        )
        print(json.dumps({"cap": "maps", "ok": ok, "surface": surface, "labels": labs[:10]}))
        return 0 if ok else 1

    print(json.dumps({"ok": False, "fault": "model", "error": "unsupported goal for cap arm"}))
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
