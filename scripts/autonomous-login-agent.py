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
MAX_STEPS = int(os.environ.get("LIGH_AUTONOMOUS_MAX_STEPS", "14"))

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
  "action": "read_file" | "write_file" | "build_app" | "ligh_ready" | "ligh_app_job" | "done",
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
- Start with ligh_app_job to capture the fault, then read_file
- After source edits always build_app then ligh_app_job to verify
- ligh_app_job runs the product acceptance job (alice/secret login → homeTitle)
- Read LIGH results: ok, fault, detail.step, detail.op — use them to find the bug
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


def main() -> int:
    goal = os.environ.get(
        "LIGH_AUTONOMOUS_GOAL",
        "The XCUITestDemo login flow is broken — users cannot reach the home screen after login. "
        "Find and fix the problem in the Swift source, then verify with LIGH.",
    )
    t0 = time.time()
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": SYSTEM},
        {
            "role": "user",
            "content": goal
            + "\n\nFirst: run ligh_app_job to capture the fault, then inspect Swift sources.",
        },
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
        messages.append(
            {
                "role": "user",
                "content": json.dumps({"tool_result": result, "step": step, "hint": "continue or done after ok:true"}),
            }
        )

    doc = {
        "gate": "autonomous_agent",
        "claim": "Vague prompt → inspect/hypothesize/edit/build/LIGH loop without scripted fix",
        "goal": goal,
        "driver": "openai",
        "model": MODEL,
        "steps_used": len(trace),
        "verified": verified,
        "claim_pass": verified,
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
