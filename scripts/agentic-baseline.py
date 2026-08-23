#!/usr/bin/env python3
"""Agentic baseline — LIGH structured agent vs simctl+screenshot+vision.

Same task, same app, same model. Publishes docs/assets/agentic-baseline-latest.json.

Arm ligh: accessibility JSON + ligh_cap_app_job / observe / reach (no screenshots).
Arm vision: simctl screenshot + vision LLM + dumb HID (ligh tap/type only — no AX/cap).

Requires: OPENAI_API_KEY, release ligh, built XCUITestDemo.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))


def _load_dotenv() -> None:
    if os.environ.get("OPENAI_API_KEY", "").strip():
        return
    for path in (
        os.environ.get("LIGH_ENV_FILE"),
        os.path.join(ROOT, ".env"),
    ):
        if not path or not os.path.isfile(path):
            continue
        with open(path, encoding="utf-8") as f:
            for line in f:
                s = line.strip()
                if not s or s.startswith("#") or "=" not in s:
                    continue
                k, _, v = s.partition("=")
                k, v = k.strip(), v.strip().strip('"').strip("'")
                if k and v and k not in os.environ:
                    os.environ[k] = v
        if os.environ.get("OPENAI_API_KEY", "").strip():
            return


_load_dotenv()

from ligh_mcp import call_tool  # noqa: E402

LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
APP = os.environ.get(
    "LIGH_APP_PATH",
    os.path.join(ROOT, "fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"),
)
BUNDLE_ID = os.environ.get("LIGH_APP_BUNDLE_ID", "com.himali.XCUITestDemo")
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
OUT = os.environ.get(
    "LIGH_AGENTIC_BASELINE_OUT",
    os.path.join(ROOT, "docs", "assets", "agentic-baseline-latest.json"),
)
MAX_STEPS = int(os.environ.get("LIGH_AGENTIC_MAX_STEPS", "14"))

GOAL = (
    "Log into the app with username alice and password secret, then verify the home screen is visible."
)

LOGIN_STEPS = [
    {"op": "wait", "id": "usernameTextField"},
    {"op": "type", "text": "alice", "id": "usernameTextField"},
    {"op": "wait", "id": "passwordSecureField"},
    {"op": "type", "text": "secret", "id": "passwordSecureField"},
    {"op": "dismiss_overlay"},
    {"op": "tap", "id": "loginButton", "until_id": "homeTitle"},
]

SYSTEM_LIGH = """You control an iOS Simulator app via LIGH (accessibility JSON — NOT screenshots).

Goal: log in with alice / secret and verify home screen (identifier homeTitle).

Reply ONE JSON object:
{"action":"observe"|"ready"|"dismiss_overlay"|"reach"|"app_job"|"done",
 "id":"...","steps":[...],"reason":"..."}

Known accessibility ids: usernameTextField, passwordSecureField, loginButton, homeTitle.
Prefer app_job with wait/tap/type/dismiss_overlay steps when ids are known.
On fault target_missing read evidence; use reach or dismiss_overlay before retry.
Call done only after app_job returns ok:true or observe shows homeTitle in actionable_topk.
"""

SYSTEM_VISION = """You control iOS Simulator using SCREENSHOTS ONLY (no accessibility tree).

Goal: log in with username alice, password secret, verify home screen visible.

Reply ONE JSON object:
{"action":"tap"|"type"|"dismiss_keyboard"|"wait"|"done","x":0.0-1.0,"y":0.0-1.0,"text":"...","reason":"..."}

Coordinates are normalized 0-1 relative to the screenshot (top-left origin).
Typical login flow: tap username field → type alice → tap password → type secret → dismiss keyboard → tap login → done when home visible.
"""


def run_ligh(*args: str, timeout: float = 120) -> tuple[int, str, str]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout or "").strip(), (p.stderr or "").strip()


def session_udid() -> str:
    for p in (
        os.path.expanduser("~/.ligh/session.json"),
        os.path.expanduser("~/Library/Application Support/ligh/session.json"),
    ):
        if os.path.isfile(p):
            u = json.load(open(p)).get("udid") or ""
            if u:
                return u
    _, out, _ = run_ligh("status", timeout=30)
    try:
        return json.loads(out).get("udid") or "booted"
    except Exception:
        return "booted"


def openai_json(messages: list[dict[str, Any]], *, vision: bool = False) -> tuple[dict[str, Any], dict[str, int]]:
    import tempfile

    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
    body: dict[str, Any] = {"model": MODEL, "messages": messages}
    if not vision:
        body["response_format"] = {"type": "json_object"}
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
        raise RuntimeError(r.stderr[:400] or "curl failed")
    payload = json.loads(r.stdout)
    if "error" in payload:
        raise RuntimeError(str(payload["error"]))
    content = (payload["choices"][0]["message"]["content"] or "").strip()
    if content.startswith("```"):
        content = re.sub(r"^```(?:json)?\s*", "", content)
        content = re.sub(r"\s*```$", "", content)
    m = re.search(r"\{.*\}", content, re.S)
    act = json.loads(m.group(0) if m else content)
    usage = payload.get("usage") or {}
    return act, {
        "prompt_tokens": int(usage.get("prompt_tokens") or 0),
        "completion_tokens": int(usage.get("completion_tokens") or 0),
    }


def prep_session() -> None:
    call_tool("ligh_ready", {"settle_ms": 4000, "recover_homes": 6})


def launch_app() -> None:
    prep_session()
    call_tool(
        "ligh_cap_run_app",
        {"app": APP, "bundle_id": BUNDLE_ID, "settle_ms": 4000, "timeout_ms": 18000},
    )


def simctl_screenshot(path: str) -> None:
    udid = session_udid()
    target = udid if udid != "booted" else "booted"
    subprocess.run(
        ["xcrun", "simctl", "io", target, "screenshot", path],
        check=True,
        capture_output=True,
        timeout=30,
    )


def home_visible_ligh() -> bool:
    obs = call_tool("ligh_observe", {"settle_ms": 2500})
    for n in obs.get("actionable_topk") or []:
        if (n.get("id") or "") == "homeTitle":
            return True
    return False


def arm_ligh_scripted() -> dict[str, Any]:
    t0 = time.time()
    prep_session()
    r = call_tool(
        "ligh_cap_app_job",
        {
            "app": APP,
            "bundle_id": BUNDLE_ID,
            "steps": LOGIN_STEPS,
            "settle_ms": 4000,
            "timeout_ms": 25000,
            "no_install": True,
        },
    )
    if not r.get("ok") and r.get("fault") in ("eyes_unusable", "infra"):
        prep_session()
        r = call_tool(
            "ligh_cap_app_job",
            {
                "app": APP,
                "bundle_id": BUNDLE_ID,
                "steps": LOGIN_STEPS,
                "settle_ms": 4000,
                "timeout_ms": 25000,
                "no_install": True,
            },
        )
    ok = bool(r.get("ok"))
    return {
        "stack": "ligh structured (scripted app_job)",
        "completed": ok,
        "time_to_green_s": round(time.time() - t0, 2),
        "tool_calls": 1,
        "human_interventions": 0,
        "recovery_attempts": 0,
        "tokens": {"in": 0, "out": 0},
        "failure_mode": None if ok else str(r.get("fault") or "app_job_failed"),
        "one_sentence": "Single app_job with known a11y ids" if ok else f"Failed: {r.get('fault')}",
        "detail": {"ok": r.get("ok"), "fault": r.get("fault"), "step": (r.get("detail") or {}).get("step")},
    }


def arm_ligh_agent() -> dict[str, Any]:
    t0 = time.time()
    launch_app()
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": SYSTEM_LIGH},
        {"role": "user", "content": GOAL + f"\nApp ids: usernameTextField, passwordSecureField, loginButton, homeTitle"},
    ]
    tool_calls = 0
    recovery = 0
    tokens_in = tokens_out = 0
    failure_mode: str | None = None
    completed = False
    trace: list[dict[str, Any]] = []

    for step in range(1, MAX_STEPS + 1):
        if home_visible_ligh():
            completed = True
            break
        act, usage = openai_json(messages, vision=False)
        tokens_in += usage["prompt_tokens"]
        tokens_out += usage["completion_tokens"]
        tool_calls += 1
        action = (act.get("action") or "").lower()
        result: dict[str, Any]

        if action == "done":
            completed = home_visible_ligh()
            failure_mode = None if completed else "done_without_home"
            trace.append({"step": step, "action": act, "completed": completed})
            break
        if action == "observe":
            result = call_tool("ligh_observe", {"settle_ms": 2500})
        elif action == "ready":
            recovery += 1
            result = call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
        elif action == "dismiss_overlay":
            recovery += 1
            result = call_tool("ligh_cap_dismiss_overlay", {})
        elif action == "reach":
            result = call_tool(
                "ligh_cap_reach",
                {"id": act.get("id"), "label": act.get("label"), "timeout_ms": 12000},
            )
        elif action == "app_job":
            steps = act.get("steps") or LOGIN_STEPS
            result = call_tool(
                "ligh_cap_app_job",
                {
                    "app": APP,
                    "bundle_id": BUNDLE_ID,
                    "steps": steps,
                    "settle_ms": 3500,
                    "timeout_ms": 22000,
                },
            )
            if result.get("ok"):
                completed = True
        else:
            result = {"ok": False, "error": f"unknown action {action}"}

        trace.append({"step": step, "action": act, "result_ok": result.get("ok"), "fault": result.get("fault")})
        if result.get("ok") and action == "app_job":
            break
        if not result.get("ok") and action in ("ready", "dismiss_overlay", "reach"):
            recovery += 1

        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append({"role": "user", "content": json.dumps({"tool_result": result, "step": step})})

    if not completed and not failure_mode:
        failure_mode = "max_steps" if tool_calls >= MAX_STEPS else "incomplete"

    return {
        "stack": "ligh structured (LLM + MCP tools)",
        "completed": completed,
        "time_to_green_s": round(time.time() - t0, 2),
        "tool_calls": tool_calls,
        "human_interventions": 0,
        "recovery_attempts": recovery,
        "tokens": {"in": tokens_in, "out": tokens_out},
        "failure_mode": failure_mode,
        "one_sentence": "Structured faults + app_job" if completed else f"Stopped: {failure_mode}",
        "trace_tail": trace[-4:],
    }


def arm_vision_agent() -> dict[str, Any]:
    t0 = time.time()
    launch_app()
    png = "/tmp/ligh-agentic-baseline.png"
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": SYSTEM_VISION},
        {"role": "user", "content": GOAL},
    ]
    tool_calls = 0
    recovery = 0
    tokens_in = tokens_out = 0
    failure_mode: str | None = None
    completed = False
    history: list[str] = []
    trace: list[dict[str, Any]] = []

    for step in range(1, MAX_STEPS + 1):
        simctl_screenshot(png)
        b64 = base64.b64encode(open(png, "rb").read()).decode()
        user_content: list[dict[str, Any]] = [
            {"type": "text", "text": json.dumps({"goal": GOAL, "step": step, "recent": history[-6:]})},
            {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}},
        ]
        act, usage = openai_json(
            [{"role": "system", "content": SYSTEM_VISION}, {"role": "user", "content": user_content}],
            vision=True,
        )
        tokens_in += usage["prompt_tokens"]
        tokens_out += usage["completion_tokens"]
        tool_calls += 1
        action = (act.get("action") or "").lower()
        result = ""

        if action == "done":
            completed = True
            trace.append({"step": step, "action": act})
            break
        if action == "wait":
            time.sleep(0.5)
            result = "wait"
        elif action == "type":
            text = act.get("text") or ""
            code, _, err = run_ligh("type", "--text", text)
            result = f"type:rc={code}"
            if code != 0:
                recovery += 1
                failure_mode = err[:120] or "type_failed"
        elif action == "dismiss_keyboard":
            code, _, _ = run_ligh("key", "--name", "return")
            time.sleep(0.3)
            code2, _, _ = run_ligh("tap", "--x", "0.5", "--y", "0.2")
            result = f"dismiss:rc={code}/{code2}"
            recovery += 1
        elif action == "tap":
            x = float(act.get("x") or 0.5)
            y = float(act.get("y") or 0.5)
            code, _, err = run_ligh("tap", "--x", str(x), "--y", str(y))
            result = f"tap:{x:.2f},{y:.2f}:rc={code}"
            if code != 0:
                recovery += 1
        else:
            result = f"unknown:{action}"
            recovery += 1

        history.append(f"{step}:{action}->{result}")
        trace.append({"step": step, "action": act, "result": result})
        time.sleep(0.4)

    if not completed and not failure_mode:
        failure_mode = "max_steps"

    return {
        "stack": "simctl screenshot + vision LLM + coordinate HID",
        "completed": completed,
        "time_to_green_s": round(time.time() - t0, 2),
        "tool_calls": tool_calls,
        "human_interventions": 0,
        "recovery_attempts": recovery,
        "tokens": {"in": tokens_in, "out": tokens_out},
        "failure_mode": failure_mode,
        "one_sentence": "Vision loop reached done" if completed else f"Stopped: {failure_mode}",
        "trace_tail": trace[-4:],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", choices=("ligh", "vision", "both", "scripted"), default="both")
    ap.add_argument("--no-llm", action="store_true", help="ligh arm only: scripted app_job")
    args = ap.parse_args()

    if not os.path.isfile(LIGH):
        print(f"error: missing {LIGH}", file=sys.stderr)
        return 1
    if run_ligh("daemon", "status")[0] != 0:
        print("error: lighd not running — ligh daemon start", file=sys.stderr)
        return 1
    if not os.path.isdir(APP):
        print(f"error: app missing {APP} — run scripts/build-xcuitestdemo.sh", file=sys.stderr)
        return 1

    doc: dict[str, Any] = {
        "schema": "agentic-baseline-v1",
        "date": time.strftime("%Y-%m-%d"),
        "claim": "LIGH structured agent vs simctl+vision baseline — same task, same app",
        "agent": "agentic-baseline.py",
        "model": MODEL,
        "app": {"path": APP, "bundle_id": BUNDLE_ID},
        "task": GOAL,
        "arms": {},
    }

    use_llm = not args.no_llm and bool(os.environ.get("OPENAI_API_KEY"))

    if args.arm in ("ligh", "both", "scripted"):
        if use_llm and args.arm != "scripted":
            print("▶ arm ligh (LLM)", flush=True)
            doc["arms"]["ligh"] = arm_ligh_agent()
        else:
            print("▶ arm ligh (scripted app_job)", flush=True)
            doc["arms"]["ligh"] = arm_ligh_scripted()

    if args.arm in ("vision", "both"):
        if not use_llm:
            doc["arms"]["baseline"] = {
                "stack": "simctl+vision",
                "completed": False,
                "failure_mode": "OPENAI_API_KEY missing",
            }
        else:
            print("▶ arm vision (simctl+screenshot+LLM)", flush=True)
            launch_app()
            doc["arms"]["baseline"] = arm_vision_agent()

    ligh = doc["arms"].get("ligh") or {}
    base = doc["arms"].get("baseline") or {}
    if ligh.get("completed") and base.get("completed"):
        doc["preference"] = "both_completed_compare_metrics"
    elif ligh.get("completed"):
        doc["preference"] = "ligh"
    elif base.get("completed"):
        doc["preference"] = "baseline"
    else:
        doc["preference"] = "neither"

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")

    print(json.dumps({"out": OUT, "preference": doc["preference"], "arms": doc["arms"]}, indent=2))
    return 0 if ligh.get("completed") else 1


if __name__ == "__main__":
    raise SystemExit(main())
