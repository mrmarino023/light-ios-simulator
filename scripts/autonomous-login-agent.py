#!/usr/bin/env python3
"""Autonomous login-fix agent — vague prompt, source edit + LIGH verify (no scripted fix).

Uses OpenAI + ligh_mcp.call_tool. The harness injects a bug; the model must diagnose from faults.
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
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from ligh_mcp import call_tool  # noqa: E402

APP = os.environ.get(
    "LIGH_APP_PATH",
    os.path.join(ROOT, "fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"),
)
BUNDLE_ID = os.environ.get("LIGH_APP_BUNDLE_ID", "com.himali.XCUITestDemo")
DEMO_ROOT = os.path.join(ROOT, "fixtures/third-party/XCUITestDemo")
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
MAX_STEPS = int(os.environ.get("LIGH_AUTONOMOUS_MAX_STEPS", "16"))

KNOWN_FILES = [
    "fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift",
    "fixtures/third-party/XCUITestDemo/XCUITestDemo/LoginViewModel.swift",
]

ACCEPTANCE_STEPS = [
    {"op": "wait", "id": "usernameTextField"},
    {"op": "tap", "id": "usernameTextField"},
    {"op": "type", "text": "alice"},
    {"op": "tap", "id": "passwordSecureField"},
    {"op": "type", "text": "secret"},
    {"op": "tap", "id": "loginButton"},
    {"op": "wait", "id": "homeTitle"},
]

SYSTEM = """You are a coding agent fixing an iOS SwiftUI app on a Mac with LIGH Simulator control.

Respond with a single JSON object each turn:
{
  "action": "read_file" | "write_file" | "build_app" | "ligh_observe" | "ligh_ready" | "ligh_dismiss_overlay" | "ligh_reach" | "ligh_app_job" | "ligh_app_goal" | "done",
  "path": "fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift",
  "content": "...",                                   // write_file only
  "summary": "..."                                    // done only
}

Known source files (use these exact paths):
- fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift
- fixtures/third-party/XCUITestDemo/XCUITestDemo/LoginViewModel.swift

Rules:
- App sources live under fixtures/third-party/XCUITestDemo/
- write_file is allowed only under that tree (.swift files)
- Call ligh_observe when a fault is unclear — read candidates in evidence
- Use ligh_dismiss_overlay before retrying taps under keyboard
- Use ligh_app_goal or ligh_reach when a wait/tap step fails with target_missing
- Use ligh_app_job after builds to run and verify the login flow (acceptance: alice/secret → homeTitle)
- Read LIGH results: ok, fault, detail, evidence.candidates — turn structured faults into code fixes
- If build_app fails, read the error tail and fix the source before retrying
- Call done only after ligh_app_job returns ok:true
- Do not ask the user questions; fix the app
"""


def openai_chat(messages: list[dict[str, Any]]) -> dict[str, Any]:
    import tempfile

    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
    body = {
        "model": MODEL,
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


def safe_path(rel: str) -> str:
    rel = (rel or "").strip()
    if not rel:
        raise ValueError("path required")
    if rel.startswith("/"):
        p = os.path.normpath(rel)
    else:
        p = os.path.normpath(os.path.join(ROOT, rel))
    if not p.startswith(DEMO_ROOT):
        # Common LLM mistake: bare filename
        base = os.path.basename(rel)
        for k in KNOWN_FILES:
            if k.endswith("/" + base) or k.endswith(base):
                return os.path.normpath(os.path.join(ROOT, k))
        raise ValueError(f"path must be under fixtures/third-party/XCUITestDemo (got {rel})")
    return p


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    try:
        return _run_action_inner(action, act)
    except ValueError as e:
        return {"ok": False, "error": str(e)}
    except Exception as e:
        return {"ok": False, "error": f"{type(e).__name__}: {e}"}


def _run_action_inner(action: str, act: dict[str, Any]) -> dict[str, Any]:
    if action == "read_file":
        p = safe_path(str(act.get("path") or ""))
        if not os.path.isfile(p):
            return {"ok": False, "error": f"not found: {p}"}
        with open(p, encoding="utf-8") as f:
            text = f.read()
        return {"ok": True, "path": p, "lines": len(text.splitlines()), "content": text[:12000]}

    if action == "write_file":
        p = safe_path(str(act.get("path") or ""))
        if not p.endswith(".swift"):
            return {"ok": False, "error": "only .swift files"}
        content = act.get("content")
        if not isinstance(content, str):
            return {"ok": False, "error": "content required"}
        with open(p, "w", encoding="utf-8") as f:
            f.write(content)
        return {"ok": True, "path": p, "bytes": len(content.encode())}

    if action == "build_app":
        t0 = time.time()
        r = subprocess.run(
            [os.path.join(ROOT, "scripts/build-xcuitestdemo.sh")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=300,
        )
        return {
            "ok": r.returncode == 0,
            "ms": int((time.time() - t0) * 1000),
            "tail": (r.stdout or r.stderr or "")[-1500:],
        }

    if action == "ligh_ready":
        t0 = time.time()
        r = call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
        return {"ok": bool(r.get("ok")), "ms": int((time.time() - t0) * 1000), "result": r}

    if action == "ligh_observe":
        t0 = time.time()
        r = call_tool("ligh_observe", {"settle_ms": act.get("settle_ms") or 2500})
        return {"ok": bool(r.get("ok")), "ms": int((time.time() - t0) * 1000), "result": r}

    if action == "ligh_dismiss_overlay":
        t0 = time.time()
        r = call_tool("ligh_cap_dismiss_overlay", {})
        return {"ok": bool(r.get("ok")), "ms": int((time.time() - t0) * 1000), "result": r}

    if action == "ligh_reach":
        t0 = time.time()
        r = call_tool(
            "ligh_cap_reach",
            {"id": act.get("id"), "label": act.get("label"), "timeout_ms": act.get("timeout_ms") or 12000},
        )
        return {
            "ok": bool(r.get("ok")),
            "fault": r.get("fault"),
            "evidence": r.get("evidence"),
            "ms": int((time.time() - t0) * 1000),
            "compact": r,
        }

    if action == "ligh_app_goal":
        t0 = time.time()
        r = call_tool(
            "ligh_cap_app_goal",
            {
                "app": APP,
                "bundle_id": BUNDLE_ID,
                "setup": act.get("setup") or [],
                "postconditions": act.get("postconditions")
                or [{"wait_id": "homeTitle"}],
            },
        )
        return {
            "ok": bool(r.get("ok")),
            "fault": r.get("fault"),
            "detail": r.get("detail"),
            "evidence": r.get("evidence"),
            "ms": int((time.time() - t0) * 1000),
        }

    if action == "ligh_app_job":
        t0 = time.time()
        r = call_tool(
            "ligh_cap_app_job",
            {
                "app": APP,
                "bundle_id": BUNDLE_ID,
                "steps": ACCEPTANCE_STEPS,
                "settle_ms": 3500,
                "timeout_ms": 15000,
            },
        )
        return {
            "ok": bool(r.get("ok")),
            "fault": r.get("fault"),
            "detail": r.get("detail"),
            "ms": int((time.time() - t0) * 1000),
            "compact": {
                "ok": r.get("ok"),
                "fault": r.get("fault"),
                "step": (r.get("detail") or {}).get("step") if isinstance(r.get("detail"), dict) else None,
                "op": (r.get("detail") or {}).get("op") if isinstance(r.get("detail"), dict) else None,
            },
        }

    if action == "done":
        return {"ok": True, "done": True, "summary": act.get("summary") or ""}

    return {"ok": False, "error": f"unknown action: {action}"}


def classify_failure(trace: list[dict[str, Any]], verified: bool) -> str | None:
    if verified:
        return None
    unknown = [
        t for t in trace
        if "unknown action" in str((t.get("result") or {}).get("error", "")).lower()
    ]
    if unknown:
        return "hallucination"
    jobs = [t for t in trace if (t.get("action") or {}).get("action") == "ligh_app_job"]
    writes = [t for t in trace if (t.get("action") or {}).get("action") == "write_file"]
    builds = [t for t in trace if (t.get("action") or {}).get("action") == "build_app"]
    if jobs:
        last = jobs[-1].get("result") or {}
        if not last.get("ok"):
            fault = str(last.get("fault") or last.get("compact", {}).get("fault") or "")
            if "timeout" in fault.lower():
                return "timeout"
            if writes:
                return "wrong_fix_test_still_fails"
            return "ligh_fault_unresolved"
    if builds and not (builds[-1].get("result") or {}).get("ok"):
        return "build_failed"
    if (trace and (trace[-1].get("action") or {}).get("action") == "done"):
        return "premature_done"
    if len(trace) >= MAX_STEPS:
        return "max_steps"
    return "unknown"


def main() -> int:
    goal = os.environ.get(
        "LIGH_AUTONOMOUS_GOAL",
        "The login flow is broken. Find out why and fix it.",
    )
    t0 = time.time()
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": SYSTEM},
        {"role": "user", "content": goal},
    ]
    trace: list[dict[str, Any]] = []
    tokens_in = tokens_out = 0
    verified = False

    for step in range(1, MAX_STEPS + 1):
        chat = openai_chat(messages)
        act = chat["act"]
        tokens_in += chat["usage"]["prompt_tokens"]
        tokens_out += chat["usage"]["completion_tokens"]
        result = run_action(act)
        row = {"step": step, "action": act, "result": result}
        trace.append(row)
        print(json.dumps({"step": step, "action": act.get("action"), "result_ok": result.get("ok")}), flush=True)

        if act.get("action") == "done":
            verified = bool(
                trace
                and any(
                    t.get("action", {}).get("action") == "ligh_app_job"
                    and (t.get("result") or {}).get("ok")
                    for t in trace
                )
            )
            break

        if act.get("action") == "ligh_app_job" and result.get("ok"):
            verified = True
            trace.append({"step": step, "action": {"action": "done"}, "result": {"ok": True, "auto": True}})
            break

        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append({"role": "user", "content": json.dumps({"tool_result": result, "step": step})})

    build_fails = sum(
        1 for t in trace
        if (t.get("action") or {}).get("action") == "build_app" and not (t.get("result") or {}).get("ok")
    )
    failure_mode = classify_failure(trace, verified)
    doc = {
        "gate": "autonomous_agent",
        "claim": "Vague prompt → inspect/hypothesize/edit/build/LIGH loop without scripted fix",
        "goal": goal,
        "driver": "openai",
        "model": MODEL,
        "steps_used": len(trace),
        "verified": verified,
        "failure_mode": failure_mode,
        "build_fail_events": build_fails,
        "tokens": {"in": tokens_in, "out": tokens_out},
        "total_ms": int((time.time() - t0) * 1000),
        "trace": trace,
    }
    out = os.environ.get(
        "LIGH_AUTONOMOUS_OUT",
        os.path.join(ROOT, "docs/assets/autonomous-agent-latest.json"),
    )
    with open(out, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")
    print(json.dumps({"claim_pass": verified, "steps": len(trace), "out": out}, indent=2))
    return 0 if verified else 1


if __name__ == "__main__":
    raise SystemExit(main())
