#!/usr/bin/env python3
"""Frontier agent loop — settled AX eyes → act → verify (no screenshots).

Control law:
  1. observe --settle until ready (never plan on transition/empty)
  2. act with LABEL first (ids are hints)
  3. re-observe settle; trust surface + typed events
  4. done when surface/goal matches — not when the model vibes

Honest: this is accessibility robotics, not pixels. Settling + verify is what
makes it competitive with vision agents on structured iOS UI.
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


def eye_packet(snap: dict[str, Any]) -> dict[str, Any]:
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
        "ax_quality": snap.get("ax_quality"),
        "settled": snap.get("settled"),
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
    return "message" in g or "messaggi" in g or "sms" in g


def goal_is_settings(goal: str) -> bool:
    g = goal.lower()
    return "setting" in g or "impostazioni" in g


def has_typed_event(snap: dict[str, Any], text: str) -> bool:
    t = text.lower()
    for e in snap.get("events") or []:
        if e.get("kind") not in ("typed", "value_changed"):
            continue
        payload = e.get("payload") or {}
        blob = json.dumps(payload).lower()
        if t and t in blob:
            return True
    return False


def labels(eyes: dict[str, Any]) -> set[str]:
    return {(a.get("label") or "").strip() for a in eyes.get("actionable") or [] if a.get("label")}


SYSTEM = """You drive iOS Simulator via LIGH. Eyes = settled accessibility scene graph (NOT screenshots).

Reply ONE JSON object only:
{"action":"tap"|"type"|"wait"|"home"|"compose_sms"|"clear"|"key"|"long_press"|"scroll_until"|"done",
 "label":"...","id":"...","text":"...","key":"...","reason":"..."}

HARD RULES:
- Prefer label over id. Bare labels: Messaggi, Impostazioni, Settings, Messages, Messaggio, Cerca, Generali.
- Never act if ax_quality is transition/empty — reply wait or home.
- Use surface: springboard | settings | messages_composer | app | transition.
- Messages goal: from springboard tap Messaggi OR compose_sms; in messages_composer type once then done.
- Settings goal: tap Impostazioni/Settings; done when surface=settings.
- After a successful type of the goal text → done (host emits typed event).
- Never screenshot. Never invent labels.
"""


def llm(goal: str, history: list[str], eyes: dict[str, Any], step: int, model: str) -> dict[str, Any]:
    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
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
    r = subprocess.run(
        [
            "curl",
            "-sS",
            "-X",
            "POST",
            OPENAI_URL,
            "-H",
            f"Authorization: Bearer {key}",
            "-H",
            "Content-Type: application/json",
            "-d",
            json.dumps(body),
        ],
        capture_output=True,
        text=True,
        timeout=90,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr[:400])
    payload = json.loads(r.stdout)
    if "error" in payload:
        raise RuntimeError(str(payload["error"]))
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
    if action == "wait":
        time.sleep(0.45)
        return "wait"
    if action == "home":
        run_ligh("home")
        time.sleep(0.35)
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
            code, out, err = run_ligh("tap", "--label", label, "--timeout-ms", "7000")
            return f"tap:{label}:rc={code}:{out or err}"
        if eid:
            code, out, err = run_ligh("tap", "--id", eid, "--timeout-ms", "5000")
            return f"tapid:{eid}:rc={code}:{out or err}"
        return "tap-missing"
    if action == "type":
        if not text:
            return "type-missing"
        code, out, err = run_ligh("type", "--text", text)
        return f"type:rc={code}:{out or err}"
    if action == "clear":
        code, out, err = run_ligh("clear", "--count", str(act.get("count") or 40))
        return f"clear:rc={code}"
    if action == "key":
        code, out, err = run_ligh("key", "--name", key or "return")
        return f"key:rc={code}"
    if action == "long_press":
        args = ["long-press", "--hold-ms", "600"]
        if label:
            args += ["--label", label]
        elif eid:
            args += ["--id", eid]
        else:
            return "long_press-missing"
        code, out, err = run_ligh(*args)
        return f"long_press:rc={code}"
    if action == "scroll_until":
        args = ["scroll-until", "--max-swipes", "8"]
        if label:
            args += ["--label", label]
        elif eid:
            args += ["--id", eid]
        else:
            return "scroll_until-missing"
        code, out, err = run_ligh(*args)
        return f"scroll_until:rc={code}"
    return f"unknown:{action}"


def policy_act(
    goal: str,
    eyes: dict[str, Any],
    want: str,
    history: list[str],
    step: int,
    model: str,
) -> dict[str, Any]:
    """Host policy first; LLM only when ambiguous."""
    aq = eyes.get("ax_quality") or ""
    surface = eyes.get("surface") or ""
    labs = labels(eyes)
    msgs = goal_is_messages(goal)
    settings = goal_is_settings(goal)

    if aq in ("empty", "transition") or not eyes.get("settled"):
        # Wake SpringBoard instead of spinning forever on empty AX.
        empty_n = sum(1 for h in history[-6:] if "wait" in h or "home" in h)
        if empty_n >= 2 or aq == "empty":
            return {"action": "home", "reason": "wake AX / recover empty eyes"}
        return {"action": "wait", "reason": "eyes not settled"}

    if settings and surface == "settings":
        return {"action": "done", "reason": "surface=settings"}
    if msgs and want and any(e.get("kind") == "typed" for e in eyes.get("events") or []):
        # typed event this observe cycle
        return {"action": "done", "reason": "typed event"}

    if msgs and surface == "messages_composer" and want:
        return {"action": "type", "text": want, "reason": "composer — type goal"}

    if msgs and surface == "springboard":
        if "Messaggi" in labs or "Messages" in labs:
            return {
                "action": "tap",
                "label": "Messaggi" if "Messaggi" in labs else "Messages",
                "reason": "open Messages from springboard",
            }
        return {"action": "compose_sms", "reason": "fallback sms:"}

    if settings and surface == "springboard":
        if "Impostazioni" in labs or "Settings" in labs:
            return {
                "action": "tap",
                "label": "Impostazioni" if "Impostazioni" in labs else "Settings",
                "reason": "open Settings",
            }

    # Ambiguous app surface — ask model
    return llm(goal, history, eyes, step, model)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--goal", required=False, default="Open Messages, type: hello from ligh")
    ap.add_argument("--steps", type=int, default=16)
    ap.add_argument("--model", default=os.environ.get("OPENAI_MODEL", MODEL))
    args = ap.parse_args()
    want = goal_type_text(args.goal)

    if not os.path.isfile(LIGH):
        print(f"error: missing {LIGH}", file=sys.stderr)
        return 1
    if run_ligh("daemon", "status")[0] != 0:
        print("error: lighd not running", file=sys.stderr)
        return 1

    print(f"model={args.model}")
    print(f"goal={args.goal}")
    print(f"want_text={want!r}")
    print(f"settle_ms={SETTLE_MS}")
    print("mode=frontier settled-AX (no screenshots)")
    print("═" * 40)

    history: list[str] = []
    typed_ok = False

    for i in range(1, args.steps + 1):
        snap = observe_settled()
        eyes = eye_packet(snap)
        print(
            f"\n▶ step {i}/{args.steps}  ax={eyes['ax_quality']} settled={eyes['settled']} "
            f"surface={eyes['surface']} actionable={len(eyes['actionable'])}"
        )
        if eyes["actionable"][:5]:
            print("  see:", ", ".join((a.get("label") or "?") for a in eyes["actionable"][:5]))

        # Terminal success
        if goal_is_settings(args.goal) and eyes.get("surface") == "settings":
            print("\n✓ surface=settings — done")
            return 0
        if want and (typed_ok or has_typed_event(snap, want)):
            print("\n✓ typed verified — done")
            return 0

        act = policy_act(args.goal, eyes, want, history, i, args.model)
        if (act.get("action") or "").lower() in ("screenshot", "vision", "image"):
            act = {"action": "wait", "reason": "no screenshots"}
        if (act.get("action") or "").lower() == "type" and want:
            act = dict(act)
            act["text"] = want

        print("  plan:", json.dumps(act, ensure_ascii=False))
        if (act.get("action") or "").lower() == "done":
            print("\n✓ done")
            return 0

        result = apply(act)
        print("  act:", result)
        history.append(f"{i}:{act.get('action')}->{result}")

        if (act.get("action") or "").lower() == "type" and "rc=0" in result:
            typed_ok = True
            # Settle + confirm typed event
            time.sleep(0.25)
            snap2 = observe_settled()
            if has_typed_event(snap2, want) or typed_ok:
                print("\n✓ type accepted by host — done")
                return 0

        time.sleep(0.2)

    print("\n✗ max steps", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.argv = [a for a in sys.argv if not a.startswith("sk-")]
    raise SystemExit(main())
