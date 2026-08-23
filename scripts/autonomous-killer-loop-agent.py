#!/usr/bin/env python3
"""Killer loop agent — task-driven code → build → verify loop on frozen OSS apps.

Arms (LIGH_KILLER_ARM):
  ligh      — perceive / attempt (default)
  hybrid    — AX-first routed perceive; vision only on eyes_unusable escalation
  baseline  — screenshot + vision taps (same edit/build/harness)

Agent receives task.json prompt only. ground-truth.json is never loaded here.
"""

from __future__ import annotations

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

from killer_loop_task import load_task, list_swift_sources, safe_source_path  # noqa: E402
from killer_loop_verify import establish_initial_state, strict_verify  # noqa: E402
from ligh_mcp import call_tool, ligh_result_path  # noqa: E402

ARM = os.environ.get("LIGH_KILLER_ARM", "ligh").lower()
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
MAX_STEPS = int(os.environ.get("LIGH_KILLER_MAX_STEPS", "28"))
TASK = load_task()

APP = os.environ.get("LIGH_APP_PATH", TASK["app_path"])
BUNDLE_ID = TASK["bundle_id"]
BUILD_SCRIPT = TASK["build_script"]
PROTOCOL_VERSION = TASK.get("protocol_version", 1)


def system_prompt() -> str:
    sources = "\n".join(f"- {p}" for p in list_swift_sources(TASK["source_root"])[:12])
    if ARM == "baseline":
        ui = """UI control: screenshot + vision coordinates ONLY (no accessibility tree for planning).
Actions: screenshot, vision_tap, vision_type, dismiss (keyboard)."""
    elif ARM == "hybrid":
        ui = """UI control: AX-first routed perceive (+ attempt/find/dismiss). Vision only on escalation.
Actions: perceive, attempt, find, dismiss, vision_tap, vision_type (vision only when perceive returns channel=vision).
perceive returns channel:
  ax     — plan from affordances; use attempt (not vision_tap)
  vision — screenshot attached; use vision_tap once, then perceive again
  none   — bootstrap_app or fix session; do not spam screenshots"""
    else:
        ui = """UI control: perceive + attempt (+ find/dismiss). Use action name "attempt", not "ligh_attempt". No screenshots for planning."""

    return f"""You fix and verify a real iOS app on a Mac.

Each turn reply with ONE JSON object.

Code actions (both arms):
  read_file, write_file, build_app, bootstrap_app, verify, done

{ui}

Swift sources (under frozen upstream tree):
{sources}

Rules:
- Prefer a SURGICAL fix. Do not rewrite whole files, move enums, add typealiases, or redesign onboarding pages.
- Look for the finish/dismiss path of onboarding (what should hide the overlay after the last step).
- After build_app succeeds: bootstrap_app → exercise the flow with attempt taps → verify.
- Call verify before done. done triggers the same strict harness (setup → exercise → postconditions).
- Seeing "Hello, world!" alone is NOT success if the onboarding overlay is still visible.
Never ask the user questions."""


def openai_chat(messages: list[dict[str, Any]], vision_image_b64: str | None = None) -> dict[str, Any]:
    import tempfile

    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
    body: dict[str, Any] = {
        "model": MODEL,
        "response_format": {"type": "json_object"},
        "messages": messages,
    }
    if vision_image_b64 and messages:
        last = messages[-1]
        if last.get("role") == "user" and isinstance(last.get("content"), str):
            last = {
                "role": "user",
                "content": [
                    {"type": "text", "text": last["content"]},
                    {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{vision_image_b64}"}},
                ],
            }
            body["messages"] = messages[:-1] + [last]
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
            timeout=180,
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
    content = payload["choices"][0]["message"]["content"].strip()
    if content.startswith("```"):
        content = re.sub(r"^```(?:json)?\s*", "", content)
        content = re.sub(r"\s*```$", "", content)
    usage = payload.get("usage") or {}
    return {
        "act": json.loads(content),
        "usage": {
            "prompt_tokens": int(usage.get("prompt_tokens") or 0),
            "completion_tokens": int(usage.get("completion_tokens") or 0),
        },
    }


def affordance_keys(perceive: dict[str, Any]) -> set[str]:
    keys: set[str] = set()
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        for k in ("id", "label", "identifier", "text"):
            if a.get(k):
                keys.add(str(a[k]))
    return keys


def harness_verify() -> dict[str, Any]:
    return strict_verify(TASK, app=APP, bundle_id=BUNDLE_ID)


def bootstrap_app() -> dict[str, Any]:
    call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
    boot = call_tool(
        "ligh_cap_run_app",
        {"app": APP, "bundle_id": BUNDLE_ID, "settle_ms": 3500, "timeout_ms": 15000},
    )
    app_label = os.path.basename(APP).replace(".app", "")
    for attempt in range(1, 6):
        p = call_tool("ligh_perceive", {"settle_ms": 2500})
        perceive = p.get("perceive") or {}
        keys = affordance_keys(perceive)
        if any(x in keys for x in ["Show Onboarding", "Get Started", "Hello, world!"]):
            return {**boot, "foreground_ok": True, "attempt": attempt}
        if any(a.get("identifier") == app_label or a.get("label") == app_label for a in (perceive.get("affordances") or []) if isinstance(a, dict)):
            call_tool("ligh_launch", {"bundle_id": BUNDLE_ID})
            time.sleep(1.0)
            call_tool("ligh_attempt", {"intent": "tap", "label": app_label, "settle_ms": 2000, "timeout_ms": 8000})
            time.sleep(1.0)
            continue
    return {**boot, "foreground_ok": False}


def screenshot_b64(path: str | None = None) -> str:
    r = call_tool("ligh_screenshot", {"path": path} if path else {})
    shot_path = path or ligh_result_path(r)
    if not shot_path or not os.path.isfile(shot_path):
        raise RuntimeError("screenshot failed")
    with open(shot_path, "rb") as f:
        return base64.b64encode(f.read()).decode("ascii")


def _attach_vision_b64(result: dict[str, Any]) -> None:
    path = result.get("screenshot_path") or ligh_result_path(result)
    if path and os.path.isfile(path):
        with open(path, "rb") as f:
            result["_b64"] = base64.b64encode(f.read()).decode("ascii")
            result["image_b64_len"] = len(result["_b64"])


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    try:
        if action == "read_file":
            p = safe_source_path(TASK, str(act.get("path") or ""))
            with open(p, encoding="utf-8") as f:
                text = f.read()
            return {"ok": True, "path": os.path.relpath(p, ROOT), "content": text[:16000]}

        if action == "write_file":
            p = safe_source_path(TASK, str(act.get("path") or ""))
            content = act.get("content")
            if not isinstance(content, str):
                return {"ok": False, "error": "content required"}
            with open(p, "w", encoding="utf-8") as f:
                f.write(content)
            return {
                "ok": True,
                "path": os.path.relpath(p, ROOT),
                "bytes": len(content.encode()),
                "preview": content[:400],
            }

        if action == "build_app":
            t0 = time.time()
            r = subprocess.run([BUILD_SCRIPT], cwd=ROOT, capture_output=True, text=True, timeout=360)
            return {
                "ok": r.returncode == 0,
                "ms": int((time.time() - t0) * 1000),
                "tail": (r.stdout or r.stderr or "")[-1800:],
            }

        if action == "bootstrap_app":
            return bootstrap_app()

        if action == "perceive":
            settle = act.get("settle_ms") or 2500
            if ARM == "hybrid":
                result = call_tool(
                    "ligh_perceive_routed",
                    {"settle_ms": settle, "recover_homes": 4, "vision_fallback": True},
                )
                if result.get("channel") == "vision":
                    _attach_vision_b64(result)
                return result
            return call_tool("ligh_perceive", {"settle_ms": settle})

        if action == "attempt":
            payload: dict[str, Any] = {
                "intent": act.get("intent") or "tap",
                "settle_ms": 2500,
                "timeout_ms": 12000,
            }
            for k in ("id", "label", "text", "key"):
                if act.get(k):
                    payload[k] = act[k]
            if act.get("expect"):
                payload["expect"] = act["expect"]
            return call_tool("ligh_attempt", payload)

        if action == "find":
            return call_tool(
                "ligh_find",
                {
                    "settle_ms": 2500,
                    "timeout_ms": 14000,
                    "max_swipes": 8,
                    **{k: act[k] for k in ("id", "label") if act.get(k)},
                },
            )

        if action == "dismiss":
            return call_tool("ligh_dismiss", {"settle_ms": 2500})

        if action == "screenshot":
            b64 = screenshot_b64()
            return {"ok": True, "image_b64_len": len(b64), "_b64": b64}

        if action == "vision_tap":
            x, y = float(act.get("x", 0.5)), float(act.get("y", 0.5))
            return call_tool("ligh_tap", {"x": x, "y": y, "settle_ms": 2000})

        if action == "vision_type":
            text = str(act.get("text") or "")
            if text:
                call_tool("ligh_type", {"text": text})
            return {"ok": True, "typed": text}

        if action == "verify":
            v = harness_verify()
            return {
                "ok": bool(v.get("verified")),
                "verified": bool(v.get("verified")),
                "reason": v.get("reason"),
                "false_success": v.get("false_success"),
                "evidence": v.get("evidence"),
                "setup_trace": v.get("setup_trace"),
                "exercise_trace": v.get("exercise_trace"),
                "preconditions": v.get("preconditions"),
            }

        if action == "done":
            return {"ok": True, "done": True, "summary": act.get("summary") or ""}

        return {"ok": False, "error": f"unknown action: {action}"}
    except Exception as e:
        return {"ok": False, "error": str(e)}


def summarize_trace(trace: list[dict[str, Any]]) -> dict[str, Any]:
    actions = []
    faults = []
    code_changes = []
    builds = 0
    verifications = 0
    perception = {"ax": 0, "vision": 0, "none": 0, "vision_escalations": 0}
    for row in trace:
        act = row.get("action") or {}
        name = act.get("action")
        res = row.get("result") or {}
        actions.append({"step": row.get("step"), "action": name, "ok": res.get("ok")})
        if res.get("fault"):
            faults.append({"step": row.get("step"), "fault": res.get("fault"), "action": name})
        if name == "write_file" and res.get("ok"):
            code_changes.append({"path": res.get("path"), "bytes": res.get("bytes"), "preview": res.get("preview")})
        if name == "build_app":
            builds += 1
        if name in ("perceive", "attempt", "screenshot", "vision_tap") and row.get("step"):
            verifications += 1
        if name == "perceive" and res.get("channel"):
            ch = str(res.get("channel"))
            if ch in perception:
                perception[ch] += 1
            if res.get("vision_escalated"):
                perception["vision_escalations"] += 1
    return {
        "agent_actions": actions,
        "faults": faults,
        "code_changes": code_changes,
        "build_attempts": builds,
        "verification_attempts": verifications,
        "human_interventions": 0,
        "perception_channels": perception,
    }


def main() -> int:
    goal = TASK["agent_prompt"]
    t0 = time.time()
    initial = establish_initial_state(TASK, app=APP, bundle_id=BUNDLE_ID)
    trace: list[dict[str, Any]] = [{"step": 0, "phase": "initial_state", "result": initial}]
    if not initial.get("ok"):
        doc = _result_doc(
            goal=goal,
            t0=t0,
            trace=trace,
            verified=False,
            verify={"verified": False, "reason": "initial_state_failed", "evidence": initial},
            tokens_in=0,
            tokens_out=0,
        )
        _write_out(doc)
        print(json.dumps({"arm": ARM, "claim_pass": False, "reason": "initial_state_failed"}))
        return 1

    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system_prompt()},
        {"role": "user", "content": goal + "\n\nStart by inspecting the repo or bootstrapping the app."},
    ]
    tokens_in = tokens_out = 0
    verified = False
    verify: dict[str, Any] = {}
    last_b64: str | None = None

    for step in range(1, MAX_STEPS + 1):
        use_vision = ARM == "baseline" or (ARM == "hybrid" and last_b64)
        chat = openai_chat(messages, last_b64 if use_vision else None)
        last_b64 = None
        act = chat["act"]
        tokens_in += chat["usage"]["prompt_tokens"]
        tokens_out += chat["usage"]["completion_tokens"]
        result = run_action(act)
        if result.get("_b64"):
            last_b64 = result.pop("_b64")
        trace.append({"step": step, "action": act, "result": {k: v for k, v in result.items() if k != "_b64"}})
        print(json.dumps({"step": step, "arm": ARM, "action": act.get("action"), "ok": result.get("ok")}), flush=True)

        if act.get("action") == "verify" and result.get("verified"):
            verified = True
            verify = {
                "verified": True,
                "reason": result.get("reason") or "verified",
                "false_success": result.get("false_success"),
                "evidence": result.get("evidence"),
                "setup_trace": result.get("setup_trace"),
                "exercise_trace": result.get("exercise_trace"),
                "preconditions": result.get("preconditions"),
            }
            trace.append({"step": step, "strict_verify": verify})
            break

        if act.get("action") == "done":
            verify = harness_verify()
            verified = bool(verify.get("verified"))
            trace.append({"step": step, "strict_verify": verify})
            if verified:
                break
            reject = {
                "ok": False,
                "done_rejected": True,
                "verified": False,
                "reason": verify.get("reason"),
                "evidence": verify.get("evidence"),
                "false_success": verify.get("false_success"),
                "suggestion": "Surgical fix only — restore overlay dismiss after finish; rebuild; verify again.",
            }
            messages.append({"role": "assistant", "content": json.dumps(act)})
            messages.append({"role": "user", "content": json.dumps({"tool_result": reject, "step": step})})
            continue

        fault = result.get("fault") or ""
        hint = {}
        if fault or result.get("error") or (act.get("action") == "verify" and not result.get("verified")):
            hint = {
                "suggestion": (
                    "Minimal edit only. Find finish/dismiss handler, restore overlay hide, "
                    "build_app, bootstrap_app, then verify. Do not rewrite enums/pages."
                )
            }

        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append({"role": "user", "content": json.dumps({"tool_result": {**result, **hint}, "step": step})})

    if not verified:
        verify = harness_verify()
        verified = bool(verify.get("verified"))
        trace.append({"step": "final", "strict_verify": verify})

    false_success = bool(verify.get("false_success"))
    claim_pass = verified and not false_success
    summary = summarize_trace(trace)
    doc = {
        "gate": "killer_loop",
        "protocol_version": PROTOCOL_VERSION,
        "arm": ARM,
        "task": TASK["id"],
        "task_prompt": goal,
        "app_id": TASK["app_id"],
        "app_commit": TASK["upstream_commit"],
        "upstream_url": TASK["upstream_url"],
        "initial_state": TASK.get("initial_state", "broken"),
        "initial_state_setup": trace[0].get("result") if trace else None,
        "final_state": "verified" if claim_pass else "failed",
        "verified": claim_pass,
        "claim_pass": claim_pass,
        "false_success": false_success,
        "verification_reason": verify.get("reason"),
        "verification_evidence": verify.get("evidence"),
        "legacy_weak_pass": verify.get("legacy_weak_pass"),
        "exercise_executed": verify.get("exercise_trace"),
        "model": MODEL,
        "wall_time_ms": int((time.time() - t0) * 1000),
        "llm_tokens": tokens_in + tokens_out,
        "tokens": {"in": tokens_in, "out": tokens_out, "total": tokens_in + tokens_out},
        "steps_used": len(trace),
        "strict_verify": verify,
        **summary,
        "trace": trace[-24:],
    }
    _write_out(doc)
    print(json.dumps({"arm": ARM, "claim_pass": claim_pass, "false_success": false_success, "reason": verify.get("reason")}))
    return 0 if claim_pass else 1


def _write_out(doc: dict[str, Any]) -> None:
    out = os.environ.get(
        "LIGH_KILLER_OUT",
        os.path.join(ROOT, f"docs/assets/killer-loop-{ARM}-latest.json"),
    )
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        json.dump(doc, indent=2, fp=f)
        f.write("\n")
    doc["_out_path"] = out


def _result_doc(
    *,
    goal: str,
    t0: float,
    trace: list[dict[str, Any]],
    verified: bool,
    verify: dict[str, Any],
    tokens_in: int,
    tokens_out: int,
) -> dict[str, Any]:
    summary = summarize_trace(trace)
    return {
        "gate": "killer_loop",
        "protocol_version": PROTOCOL_VERSION,
        "arm": ARM,
        "task": TASK["id"],
        "task_prompt": goal,
        "app_id": TASK["app_id"],
        "app_commit": TASK["upstream_commit"],
        "upstream_url": TASK["upstream_url"],
        "initial_state": TASK.get("initial_state", "broken"),
        "final_state": "failed",
        "verified": verified,
        "claim_pass": verified,
        "false_success": False,
        "verification_reason": verify.get("reason"),
        "verification_evidence": verify.get("evidence"),
        "model": MODEL,
        "wall_time_ms": int((time.time() - t0) * 1000),
        "llm_tokens": tokens_in + tokens_out,
        "tokens": {"in": tokens_in, "out": tokens_out, "total": tokens_in + tokens_out},
        "steps_used": len(trace),
        "strict_verify": verify,
        **summary,
        "trace": trace,
    }


if __name__ == "__main__":
    raise SystemExit(main())
