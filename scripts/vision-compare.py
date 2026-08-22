#!/usr/bin/env python3
"""Compare LIGH structured path vs vision-only (screenshot) on the same goals.

Requires: OPENAI_API_KEY, lighd+booted sim, LIGH_VISION_COMPARE=1

Reports tokens (approx from usage) and pass/fail for each arm.
Does not update README claims unless both arms finish — write JSON only.
"""

from __future__ import annotations

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
OUT = os.path.join(ROOT, "docs", "assets", "vision-compare-latest.json")

GOALS = [
    "Open Settings (Impostazioni or Settings), wait until list/search visible, done",
    "Open Messages, new message, type: vision-cmp-ok, done",
]


def run_ligh(*args: str) -> tuple[int, str, str]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=90)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


def openai_raw(body: dict) -> dict:
    """POST chat completions; body via tempfile so PNG base64 never hits argv limits."""
    import tempfile

    key = os.environ["OPENAI_API_KEY"].strip()
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
    return payload


def openai(messages: list[dict], model: str) -> tuple[dict, dict]:
    body = {
        "model": model,
        "response_format": {"type": "json_object"},
        "messages": messages,
    }
    payload = openai_raw(body)
    content = payload["choices"][0]["message"]["content"]
    usage = payload.get("usage") or {}
    return json.loads(content), usage


def arm_ligh(goal: str, model: str, steps: int = 12) -> dict[str, Any]:
    t0 = time.time()
    # Reuse agent loop; capture is coarse — estimate tokens via one observe call size * steps
    p = subprocess.run(
        [
            sys.executable,
            os.path.join(ROOT, "scripts", "agent-llm-loop.py"),
            "--policy",
            "llm",
            "--model",
            model,
            "--steps",
            str(steps),
            "--goal",
            goal,
        ],
        capture_output=True,
        text=True,
        timeout=180,
    )
    # Token estimate: run one observe and multiply (API usage not plumbed through loop yet)
    obs = json.loads(run_ligh("--json", "observe", "--settle-ms", "2000")[1])
    eyes = {
        "actionable": obs.get("actionable_topk") or [],
        "surface": (obs.get("scene") or {}).get("surface"),
        "events": obs.get("events") or [],
    }
    approx_in = max(1, len(json.dumps(eyes)) // 4) * min(steps, 6)
    return {
        "ok": p.returncode == 0,
        "seconds": time.time() - t0,
        "approx_input_tokens": approx_in,
        "log_tail": (p.stdout or "")[-400:],
    }


def arm_vision(goal: str, model: str, steps: int = 10) -> dict[str, Any]:
    """Screenshot → model → parse action → ligh act (PNG every step)."""
    t0 = time.time()
    usage_in = 0
    usage_out = 0
    history: list[str] = []
    ok = False
    png_path = "/tmp/ligh-vision-cmp.png"

    sys_msg = (
        "You control iOS Simulator. You receive a screenshot. "
        'Reply JSON only: {"action":"tap_norm"|"type"|"home"|"done","x":0-1,"y":0-1,"text":"...","reason":"..."}'
    )

    for step in range(1, steps + 1):
        run_ligh("screenshot", "-o", png_path)
        import base64

        b64 = base64.b64encode(open(png_path, "rb").read()).decode()
        user = {
            "goal": goal,
            "step": step,
            "recent": history[-4:],
        }
        messages = [
            {"role": "system", "content": sys_msg},
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": json.dumps(user)},
                    {
                        "type": "image_url",
                        "image_url": {"url": f"data:image/png;base64,{b64}"},
                    },
                ],
            },
        ]
        # vision models may reject response_format — try without
        try:
            payload = openai_raw({"model": model, "messages": messages})
        except RuntimeError as e:
            return {
                "ok": False,
                "seconds": time.time() - t0,
                "input_tokens": usage_in,
                "output_tokens": usage_out,
                "error": str(e),
            }
        usage = payload.get("usage") or {}
        usage_in += int(usage.get("prompt_tokens") or 0)
        usage_out += int(usage.get("completion_tokens") or 0)
        content = payload["choices"][0]["message"]["content"]
        content = content.strip()
        if content.startswith("```"):
            content = re.sub(r"^```(?:json)?\s*", "", content)
            content = re.sub(r"\s*```$", "", content)
        # extract JSON object
        m = re.search(r"\{.*\}", content, re.S)
        act = json.loads(m.group(0) if m else content)
        action = (act.get("action") or "").lower()
        if action == "done":
            ok = True
            break
        if action == "home":
            run_ligh("home")
            history.append("home")
        elif action == "type":
            run_ligh("type", "--text", act.get("text") or "")
            history.append("type")
            if "vision-cmp-ok" in goal and act.get("text"):
                ok = True
                break
        elif action in ("tap_norm", "tap"):
            x = float(act.get("x") or 0.5)
            y = float(act.get("y") or 0.5)
            run_ligh("tap", "--x", str(x), "--y", str(y))
            history.append(f"tap {x:.2f},{y:.2f}")
        else:
            history.append(f"skip:{action}")
        time.sleep(0.4)

    # Heuristic success for settings goal
    if not ok and ("Settings" in goal or "Impostazioni" in goal):
        obs = json.loads(run_ligh("--json", "observe", "--settle-ms", "2000")[1])
        if (obs.get("scene") or {}).get("surface") == "settings":
            ok = True

    return {
        "ok": ok,
        "seconds": time.time() - t0,
        "input_tokens": usage_in,
        "output_tokens": usage_out,
        "steps": len(history),
    }


def main() -> int:
    if os.environ.get("LIGH_VISION_COMPARE") != "1":
        report = {
            "status": "skipped",
            "reason": "set LIGH_VISION_COMPARE=1 and OPENAI_API_KEY to run",
            "ligh_path": "structured_observe_no_png",
            "vision_path": "screenshot_each_step",
        }
        open(OUT, "w").write(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
        return 0

    if not os.environ.get("OPENAI_API_KEY"):
        print("OPENAI_API_KEY required", file=sys.stderr)
        return 1
    if run_ligh("daemon", "status")[0] != 0:
        print("lighd not running", file=sys.stderr)
        return 1

    n = int(os.environ.get("LIGH_VISION_N", "2"))
    results = {"ts": time.time(), "model": MODEL, "n": n, "goals": [], "status": "ok"}
    for goal in GOALS:
        entry: dict[str, Any] = {"goal": goal, "ligh": [], "vision": []}
        for _ in range(n):
            run_ligh("home")
            time.sleep(0.5)
            run_ligh("home")
            time.sleep(0.6)
            entry["ligh"].append(arm_ligh(goal, MODEL))
            run_ligh("home")
            time.sleep(0.5)
            run_ligh("home")
            time.sleep(0.6)
            entry["vision"].append(arm_vision(goal, MODEL))
        results["goals"].append(entry)

    # Summarize
    def rate(arm: str) -> tuple[int, int]:
        p = t = 0
        for g in results["goals"]:
            for r in g[arm]:
                t += 1
                if r.get("ok"):
                    p += 1
        return p, t

    lp, lt = rate("ligh")
    vp, vt = rate("vision")
    results["summary"] = {
        "ligh_pass": lp,
        "ligh_total": lt,
        "vision_pass": vp,
        "vision_total": vt,
        "policy": "llm",
        "note": "Both arms: no host shortcuts. LIGH tokens approximate; vision uses API usage.prompt_tokens",
    }
    open(OUT, "w").write(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
