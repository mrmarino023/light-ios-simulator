#!/usr/bin/env python3
"""Frontier agent loop — settled AX → act → settled observe (no screenshots).

Modes:
  --policy host  : settle/surface shortcuts for Settings + Messages compose (narrow demo)
  --policy llm   : LLM plans every step; host only blocks transition/empty + screenshots
                  (use for breadth gates — no assumed answers)
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
SETTLE_MS = os.environ.get("LIGH_SETTLE_MS", "2500")


def run_ligh(*args: str, timeout: float = 90) -> tuple[int, str, str]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


def observe_settled() -> dict[str, Any]:
    code, out, err = run_ligh("--json", "observe", "--settle-ms", SETTLE_MS)
    if code != 0:
        raise RuntimeError(f"observe failed: {err or out}")
    return json.loads(out)


def eye_packet(snap: dict[str, Any]) -> dict[str, Any] | None:
    """Return None if eyes must not be shown to the model (transition/empty)."""
    aq = snap.get("ax_quality") or ""
    settled = bool(snap.get("settled"))
    if snap.get("eyes_unusable") or aq in ("empty", "transition", "error") or not settled:
        return None
    scene = snap.get("scene") or {}
    top = []
    for n in (snap.get("actionable_topk") or [])[:36]:
        top.append(
            {
                "id": n.get("id"),
                "label": n.get("label") or n.get("text"),
                "role": n.get("role"),
                "value": n.get("value"),
                "focused": n.get("focused"),
                "hittable": n.get("hittable"),
                "center_norm": n.get("center_norm"),
            }
        )
    return {
        "ax_quality": aq,
        "settled": settled,
        "phase": snap.get("phase"),
        "eyes_unusable": False,
        "surface": scene.get("surface"),
        "keyboard_visible": scene.get("keyboard_visible"),
        "screen_title": scene.get("screen_title"),
        "events": [
            {"kind": e.get("kind"), "payload": e.get("payload")}
            for e in (snap.get("events") or [])[-10:]
        ],
        "actionable": top,
    }


def goal_type_text(goal: str) -> str:
    m = re.search(r"type:\s*[\"']?([^\"',\n]+)", goal, re.I)
    return m.group(1).strip() if m else ""


def goal_is_messages(goal: str) -> bool:
    g = goal.lower()
    return ("message" in g or "messaggi" in g or "sms" in g) and "type:" in g.lower()


def goal_is_settings_open(goal: str) -> bool:
    g = goal.lower()
    return ("setting" in g or "impostazioni" in g) and "bluetooth" not in g and "safari" not in g


def has_fresh_host_typed(snap: dict[str, Any], text: str, since_seq: int) -> bool:
    """Only count typed events newer than since_seq (this-run freshness)."""
    t = (text or "").lower()
    for e in snap.get("events") or []:
        if e.get("kind") != "typed":
            continue
        seq = int(e.get("seq") or e.get("id") or 0)
        if since_seq and seq and seq <= since_seq:
            continue
        payload = e.get("payload") or {}
        if payload.get("verified") == "host_accepted":
            if not t or t in json.dumps(payload).lower():
                return True
    return False


def max_event_seq(snap: dict[str, Any]) -> int:
    m = 0
    for e in snap.get("events") or []:
        try:
            m = max(m, int(e.get("seq") or e.get("id") or 0))
        except (TypeError, ValueError):
            pass
    return m


def labels(eyes: dict[str, Any]) -> set[str]:
    return {(a.get("label") or "").strip() for a in eyes.get("actionable") or [] if a.get("label")}


SYSTEM = """You drive iOS Simulator via LIGH control plane (local Mac).
Input is settled accessibility JSON — NOT screenshots. Never ask for images.

Reply ONE JSON object only:
{"action":"ensure_ready"|"open_settings"|"settings_search"|"tap"|"type"|"wait"|"home"|"compose_sms"|"clear"|"key"|"long_press"|"scroll_until"|"assert_surface"|"done",
 "label":"...","id":"...","text":"...","query":"...","surface":"...","key":"...","reason":"..."}

Rules:
- If eyes_unusable or phase is degraded/dead: action ensure_ready (not invent UI).
- Prefer capabilities: open_settings, settings_search (query=Bluetooth), assert_surface.
- Prefer bare labels (Messaggi, Impostazioni, Settings, Messages, Messaggio, Cerca, Safari, Bluetooth, Generali, Mappe).
- id is optional hint only.
- surface: springboard|settings|messages_composer|app|transition
- typed/host_accepted means keystrokes were sent — Messages may not show body in AX value.
- Never screenshot.
"""


TOKEN_IN = 0
TOKEN_OUT = 0


def openai_chat(body: dict) -> dict:
    import tempfile

    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
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
            timeout=90,
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
    return payload


def llm(goal: str, history: list[str], eyes: dict[str, Any], step: int, model: str) -> dict[str, Any]:
    global TOKEN_IN, TOKEN_OUT
    body = {
        "model": model,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": SYSTEM},
            {
                "role": "user",
                "content": json.dumps(
                    {"goal": goal, "step": step, "recent": history[-8:], "eyes": eyes}
                ),
            },
        ],
    }
    payload = openai_chat(body)
    usage = payload.get("usage") or {}
    TOKEN_IN += int(usage.get("prompt_tokens") or 0)
    TOKEN_OUT += int(usage.get("completion_tokens") or 0)
    content = payload["choices"][0]["message"]["content"].strip()
    if content.startswith("```"):
        content = re.sub(r"^```(?:json)?\s*", "", content)
        content = re.sub(r"\s*```$", "", content)
    return json.loads(content)


def apply(act: dict[str, Any]) -> str:
    action = (act.get("action") or "").lower()
    label = re.sub(r"\s*\[AX[^\]]*\]\s*$", "", (act.get("label") or "").strip())
    eid = (act.get("id") or "").strip()
    text = act.get("text") or ""
    key = act.get("key") or ""

    if action == "done":
        return "done"
    if action == "ensure_ready":
        code, out, err = run_ligh(
            "--json", "ready", "--settle-ms", SETTLE_MS, "--recover-homes", "6"
        )
        return f"ensure_ready:rc={code}"
    if action == "open_settings":
        code, _, _ = run_ligh("--json", "cap", "open-settings", "--settle-ms", SETTLE_MS)
        return f"open_settings:rc={code}"
    if action == "settings_search":
        q = act.get("query") or text or "Bluetooth"
        code, _, _ = run_ligh(
            "--json", "cap", "settings-search", q, "--settle-ms", SETTLE_MS
        )
        return f"settings_search:rc={code}"
    if action == "assert_surface":
        surf = act.get("surface") or "settings"
        code, _, _ = run_ligh(
            "--json", "cap", "assert-surface", surf, "--settle-ms", SETTLE_MS
        )
        return f"assert_surface:rc={code}"
    if action == "wait":
        time.sleep(0.4)
        return "wait"
    if action == "home":
        run_ligh("home")
        time.sleep(0.3)
        run_ligh("home")
        return "home"
    if action == "compose_sms":
        udid = ""
        for p in (
            os.path.expanduser("~/.ligh/session.json"),
            os.path.expanduser("~/Library/Application Support/ligh/session.json"),
        ):
            if os.path.isfile(p):
                udid = json.load(open(p)).get("udid") or ""
                break
        if not udid:
            return "compose_sms-no-udid"
        subprocess.run(["xcrun", "simctl", "openurl", udid, "sms:"], capture_output=True, timeout=30)
        time.sleep(0.9)
        return "compose_sms"
    if action == "tap":
        if label:
            code, _, _ = run_ligh(
                "--json",
                "cap",
                "tap",
                "--label",
                label,
                "--settle-ms",
                SETTLE_MS,
                "--timeout-ms",
                "7000",
            )
            if code != 0:
                code, _, _ = run_ligh("tap", "--label", label, "--timeout-ms", "7000")
            return f"tap:{label}:rc={code}"
        if eid:
            code, _, _ = run_ligh(
                "--json", "cap", "tap", "--id", eid, "--settle-ms", SETTLE_MS
            )
            return f"tapid:rc={code}"
        return "tap-missing"
    if action == "type":
        if not text:
            return "type-missing"
        code, _, _ = run_ligh(
            "--json", "cap", "type", "--text", text, "--settle-ms", SETTLE_MS
        )
        if code != 0:
            code, _, _ = run_ligh("type", "--text", text)
        return f"type:rc={code}:host_typed"
    if action == "clear":
        run_ligh("clear", "--count", str(act.get("count") or 40))
        return "clear"
    if action == "key":
        run_ligh("key", "--name", key or "return")
        return f"key:{key}"
    if action == "long_press":
        args = ["long-press", "--hold-ms", "600"]
        args += ["--label", label] if label else ["--id", eid]
        code, _, _ = run_ligh(*args)
        return f"long_press:rc={code}"
    if action == "scroll_until":
        args = ["scroll-until", "--max-swipes", "8"]
        args += ["--label", label] if label else ["--id", eid]
        code, _, _ = run_ligh(*args)
        return f"scroll_until:rc={code}"
    return f"unknown:{action}"


def host_policy(goal: str, eyes: dict[str, Any], want: str) -> dict[str, Any] | None:
    """Narrow demos only. Return None to fall through to LLM."""
    surface = eyes.get("surface") or ""
    labs = labels(eyes)
    if goal_is_settings_open(goal) and surface == "settings":
        return {"action": "done", "reason": "surface=settings"}
    if goal_is_messages(goal) and surface == "messages_composer" and want:
        return {"action": "type", "text": want, "reason": "composer — type (host_typed)"}
    if goal_is_messages(goal) and surface == "springboard":
        if "Messaggi" in labs or "Messages" in labs:
            return {
                "action": "tap",
                "label": "Messaggi" if "Messaggi" in labs else "Messages",
                "reason": "open Messages",
            }
    if goal_is_settings_open(goal) and surface == "springboard":
        if "Impostazioni" in labs or "Settings" in labs:
            return {
                "action": "tap",
                "label": "Impostazioni" if "Impostazioni" in labs else "Settings",
                "reason": "open Settings",
            }
    return None


def success(goal: str, snap: dict[str, Any], want: str, host_typed_ok: bool) -> str | None:
    eyes = eye_packet(snap)
    surface = ((snap.get("scene") or {}).get("surface")) or ""
    if goal_is_settings_open(goal) and surface == "settings":
        return "surface=settings"
    # Messages: require a fresh type in THIS run (no stale host_typed from prior sessions)
    if goal_is_messages(goal) and want and host_typed_ok:
        return "host_typed (HID accepted — AX body may be empty)"
    # Breadth: Bluetooth
    if "bluetooth" in goal.lower() and eyes:
        labs = {x.lower() for x in labels(eyes)}
        if any("bluetooth" in x for x in labs) and surface == "settings":
            return "bluetooth visible in settings"
    # Breadth: Safari
    if "safari" in goal.lower() and eyes:
        labs = labels(eyes)
        if any(x in labs for x in ("Indirizzo", "Address", "URL", "TabBarItemTitle", "Caps Lock")):
            return "safari chrome visible"
        if surface == "app" and any("Safari" in (a.get("label") or "") for a in eyes["actionable"]):
            return "safari"
    # Breadth: Settings → General → back to root
    g = goal.lower()
    if ("generali" in g or "general" in g) and "back" in g and eyes and surface == "settings":
        labs = labels(eyes)
        if "Generali" in labs or "General" in labs:
            return "settings root with Generali/General row"
    # App under test: Maps (Calculator often disabled on slim profiles)
    if ("maps" in g or "mappe" in g) and eyes:
        labs = labels(eyes)
        mapish = any(
            x in labs
            for x in (
                "Mappe",
                "Maps",
                "Mappa",
                "Cerca",
                "Search",
                "Indicazioni",
                "Directions",
                "Modalità mappa",
            )
        )
        if surface != "springboard" and mapish:
            return "maps chrome visible"
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--goal", default="Open Messages, type: hello from ligh")
    ap.add_argument("--steps", type=int, default=16)
    ap.add_argument("--model", default=os.environ.get("OPENAI_MODEL", MODEL))
    ap.add_argument("--policy", choices=("host", "llm"), default="host")
    args = ap.parse_args()
    want = goal_type_text(args.goal)

    if not os.path.isfile(LIGH):
        print(f"error: missing {LIGH}", file=sys.stderr)
        return 1
    if run_ligh("daemon", "status")[0] != 0:
        print("error: lighd not running", file=sys.stderr)
        return 1

    print(f"model={args.model} policy={args.policy}")
    print(f"goal={args.goal}")
    print(f"want_text={want!r} settle_ms={SETTLE_MS}")
    print("mode=structured-control (no screenshots)")
    print("═" * 40)

    # Control-plane: never start planning on dead eyes
    code, out, err = run_ligh(
        "--json", "ready", "--settle-ms", SETTLE_MS, "--recover-homes", "6"
    )
    if code != 0:
        print(f"✗ ensure_ready failed (infra) rc={code} {(err or out)[:200]}", file=sys.stderr)
        return 2
    print("✓ ensure_ready")

    history: list[str] = []
    host_typed_ok = False
    typed_since_seq = 0

    for i in range(1, args.steps + 1):
        snap = observe_settled()
        eyes = eye_packet(snap)
        scene = (snap.get("scene") or {}).get("surface")
        print(
            f"\n▶ step {i}/{args.steps}  ax={snap.get('ax_quality')} settled={snap.get('settled')} "
            f"surface={scene} actionable={len((eyes or {}).get('actionable') or [])}"
        )

        why = success(args.goal, snap, want, host_typed_ok)
        if why:
            print(f"\n✓ done — {why}")
            print(f"tokens in={TOKEN_IN} out={TOKEN_OUT} steps={i}")
            return 0

        if eyes is None:
            act = {"action": "ensure_ready", "reason": "eyes_unusable — control-plane recover"}
            print("  eyes: BLOCKED (eyes_unusable) — ensure_ready")
        elif args.policy == "host":
            act = host_policy(args.goal, eyes, want)
            if act is None:
                act = llm(args.goal, history, eyes, i, args.model)
            if eyes.get("actionable"):
                print("  see:", ", ".join((a.get("label") or "?") for a in eyes["actionable"][:5]))
        else:
            # llm-only: no assumed answers
            if eyes.get("actionable"):
                print("  see:", ", ".join((a.get("label") or "?") for a in eyes["actionable"][:5]))
            act = llm(args.goal, history, eyes, i, args.model)

        if (act.get("action") or "").lower() in ("screenshot", "vision", "image"):
            act = {"action": "home", "reason": "no screenshots"}
        if (act.get("action") or "").lower() == "type" and want:
            act = dict(act)
            act["text"] = want

        print("  plan:", json.dumps(act, ensure_ascii=False))
        if (act.get("action") or "").lower() == "done":
            print("\n✓ done")
            print(f"tokens in={TOKEN_IN} out={TOKEN_OUT} steps={i}")
            return 0

        if (act.get("action") or "").lower() == "type":
            typed_since_seq = max_event_seq(snap)

        result = apply(act)
        print("  act:", result)
        history.append(f"{i}:{act.get('action')}->{result}")

        # Always re-settle after act — never trust pre-act eyes
        time.sleep(0.15)
        snap2 = observe_settled()
        if "host_typed" in result and "rc=0" in result:
            # Freshness: apply result alone is enough for this-run; also accept new typed events
            host_typed_ok = True
        elif want and has_fresh_host_typed(snap2, want, typed_since_seq):
            host_typed_ok = True
        why = success(args.goal, snap2, want, host_typed_ok)
        if why:
            print(f"\n✓ done after settle — {why}")
            print(f"tokens in={TOKEN_IN} out={TOKEN_OUT} steps={i}")
            return 0

    print("\n✗ max steps", file=sys.stderr)
    print(f"tokens in={TOKEN_IN} out={TOKEN_OUT} steps={args.steps}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.argv = [a for a in sys.argv if not a.startswith("sk-")]
    raise SystemExit(main())
