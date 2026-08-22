#!/usr/bin/env python3
"""WDA/Appium LLM arm for frontier bakeoff — same goals as LIGH, no host shortcuts.

Env:
  OPENAI_API_KEY, UDID (or read from `ligh --json status`)
  APPIUM_URL=http://127.0.0.1:4723
  OPENAI_MODEL=gpt-5-mini

Exit 0 on goal success, 1 on fail, 2 if Appium/session unavailable.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
BASE = os.environ.get("APPIUM_URL", "http://127.0.0.1:4723").rstrip("/")


def req(method: str, path: str, body: dict | None = None, timeout: float = 60.0) -> dict:
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


def resolve_udid() -> str | None:
    if os.environ.get("UDID"):
        return os.environ["UDID"]
    try:
        p = subprocess.run(
            [LIGH, "--json", "status"], capture_output=True, text=True, timeout=30
        )
        d = json.loads(p.stdout)
        return (d.get("session") or {}).get("udid") or (d.get("device") or {}).get("udid")
    except Exception:
        return None


def openai_json(messages: list[dict], model: str) -> dict:
    key = os.environ["OPENAI_API_KEY"].strip()
    body = {
        "model": model,
        "response_format": {"type": "json_object"},
        "messages": messages,
    }
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(body, f)
        path = f.name
    try:
        r = subprocess.run(
            [
                "curl", "-sS", "-X", "POST", OPENAI_URL,
                "-H", f"Authorization: Bearer {key}",
                "-H", "Content-Type: application/json",
                "-d", f"@{path}",
            ],
            capture_output=True,
            text=True,
            timeout=120,
        )
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass
    if r.returncode != 0:
        raise RuntimeError(r.stderr[:400])
    payload = json.loads(r.stdout)
    if "error" in payload:
        raise RuntimeError(str(payload["error"]))
    return json.loads(payload["choices"][0]["message"]["content"])


class Wda:
    def __init__(self, sid: str):
        self.sid = sid

    def delete(self) -> None:
        try:
            req("DELETE", f"/session/{self.sid}", timeout=20)
        except Exception:
            pass

    def source(self) -> str:
        try:
            return (val(req("GET", f"/session/{self.sid}/source")) or "")[:12000]
        except Exception:
            return ""

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

    def click(self, eid: str) -> None:
        req("POST", f"/session/{self.sid}/element/{eid}/click", {})

    def tap_name(self, name: str) -> bool:
        pred = f"name == '{name}' OR label == '{name}'"
        eid = self.find("-ios predicate string", pred)
        if not eid:
            return False
        self.click(eid)
        return True

    def type_text(self, text: str) -> None:
        try:
            req(
                "POST",
                f"/session/{self.sid}/execute/sync",
                {"script": "mobile: type", "args": [{"text": text}]},
            )
        except Exception:
            req("POST", f"/session/{self.sid}/keys", {"value": list(text)})

    def home(self) -> None:
        try:
            req(
                "POST",
                f"/session/{self.sid}/execute/sync",
                {"script": "mobile: pressButton", "args": [{"name": "home"}]},
            )
        except Exception:
            subprocess.run(
                [LIGH, "home"], capture_output=True, timeout=10
            )


def compact_source(xml: str) -> list[dict[str, str]]:
    """Pull a few name/label attrs for the model (not full XML dump)."""
    names = re.findall(r'(?:name|label)="([^"]{1,40})"', xml)
    out = []
    seen = set()
    for n in names:
        if n in seen or n.startswith("AX"):
            continue
        seen.add(n)
        out.append({"label": n})
        if len(out) >= 40:
            break
    return out


def goal_ok(goal: str, labels: set[str], typed_ok: bool) -> str | None:
    g = goal.lower()
    labs = labels
    if "bluetooth" in g and any("bluetooth" in x.lower() for x in labs):
        return "bluetooth"
    if "safari" in g and any(
        x in labs for x in ("Indirizzo", "Address", "URL", "Safari", "Caps Lock")
    ):
        return "safari"
    if ("generali" in g or "general" in g) and "back" in g:
        if "Generali" in labs or "General" in labs:
            return "settings_root"
    if "maps" in g or "mappe" in g:
        if any(x in labs for x in ("Mappe", "Maps", "Cerca", "Search", "Indicazioni", "Directions")):
            return "maps"
    if "calculator" in g or "calcolatrice" in g:
        if any(x in labs for x in ("1", "2", "+", "=")) or "Calcolatrice" in labs:
            return "calculator"
    if "setting" in g and "bluetooth" not in g and ("Impostazioni" in labs or "Settings" in labs or "Generali" in labs or "General" in labs):
        return "settings"
    if typed_ok and "type:" in g:
        return "typed"
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--goal", required=True)
    ap.add_argument("--steps", type=int, default=16)
    ap.add_argument("--model", default=MODEL)
    args = ap.parse_args()

    if not os.environ.get("OPENAI_API_KEY"):
        print("OPENAI_API_KEY required", file=sys.stderr)
        return 2
    udid = resolve_udid()
    if not udid:
        print("UDID unresolved", file=sys.stderr)
        return 2
    try:
        req("GET", "/status", timeout=5)
    except Exception as e:
        print(f"appium unavailable: {e}", file=sys.stderr)
        return 2

    caps = {
        "capabilities": {
            "alwaysMatch": {
                "platformName": "iOS",
                "appium:automationName": "XCUITest",
                "appium:udid": udid,
                "appium:deviceName": "iPhone",
                "appium:noReset": True,
                "appium:newCommandTimeout": 120,
            }
        }
    }
    try:
        sess = req("POST", "/session", caps, timeout=120)
        sid = sess.get("value", {}).get("sessionId") or sess.get("sessionId")
        if not sid:
            print(f"no session: {sess}", file=sys.stderr)
            return 2
    except Exception as e:
        print(f"session failed: {e}", file=sys.stderr)
        return 2

    d = Wda(sid)
    typed_ok = False
    history: list[str] = []
    print(f"model={args.model} path=wda goal={args.goal}")

    try:
        for i in range(1, args.steps + 1):
            xml = d.source()
            actionable = compact_source(xml)
            labs = {a["label"] for a in actionable}
            why = goal_ok(args.goal, labs, typed_ok)
            if why:
                print(f"✓ done — {why}")
                return 0
            eyes = {"actionable": actionable, "step": i, "path": "wda"}
            act = openai_json(
                [
                    {
                        "role": "system",
                        "content": (
                            "You drive iOS Simulator via Appium/WDA labels. "
                            'Reply JSON only: {"action":"tap"|"type"|"home"|"done","label":"...","text":"...","reason":"..."}'
                        ),
                    },
                    {
                        "role": "user",
                        "content": json.dumps(
                            {"goal": args.goal, "history": history[-6:], "eyes": eyes},
                            ensure_ascii=False,
                        ),
                    },
                ],
                args.model,
            )
            action = (act.get("action") or "").lower()
            print(f"▶ {i} plan={act}")
            if action == "done":
                return 0
            if action == "home":
                d.home()
                history.append("home")
            elif action == "type":
                text = act.get("text") or ""
                d.type_text(text)
                typed_ok = True
                history.append(f"type:{text[:20]}")
            elif action == "tap":
                label = act.get("label") or ""
                ok = d.tap_name(label) if label else False
                history.append(f"tap:{label}:{ok}")
            else:
                history.append(f"skip:{action}")
            time.sleep(0.5)
        print("✗ max steps", file=sys.stderr)
        return 1
    finally:
        d.delete()


if __name__ == "__main__":
    raise SystemExit(main())
