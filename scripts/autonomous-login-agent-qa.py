#!/usr/bin/env python3
"""Autonomous login-fix agent — QA layer (perceive/attempt/ux_hint), no app_job shortcut.

Uses ligh_attempt with expect + reads evidence.hypotheses. Records uxgraph in LIGH_WORKSPACE.
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

WORKSPACE = os.environ.get("LIGH_WORKSPACE", ROOT)
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

SYSTEM = """You fix an iOS SwiftUI app using LIGH QA layer on a Mac.

Respond with ONE JSON object:
{
  "action": "read_file" | "write_file" | "build_app" | "ligh_ready" | "ligh_verify_login" | "ligh_ux_hint" | "done",
  "path": "fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift",
  "content": "...",
  "fingerprint": "fp_...",
  "summary": "..."
}

Flow:
1. ligh_verify_login — runs install+login via ligh_attempt steps; read intent_met + evidence.hypotheses
2. read_file Swift sources when hypotheses suggest a11y_id_mismatch or silent_tap
3. write_file fix, build_app, ligh_ux_hint(fingerprint from evidence, path edited)
4. ligh_verify_login again until intent_met
5. done

Rules:
- Sources only under fixtures/third-party/XCUITestDemo/
- Do not use screenshots
- done only after ligh_verify_login returns intent_met:true
"""


def openai_chat(messages: list[dict[str, Any]]) -> dict[str, Any]:
    import tempfile

    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
    body = {"model": MODEL, "response_format": {"type": "json_object"}, "messages": messages}
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
    if rel.startswith("/"):
        p = os.path.normpath(rel)
    else:
        p = os.path.normpath(os.path.join(ROOT, rel))
    if not p.startswith(DEMO_ROOT):
        base = os.path.basename(rel)
        for k in KNOWN_FILES:
            if k.endswith("/" + base) or k.endswith(base):
                return os.path.normpath(os.path.join(ROOT, k))
        raise ValueError(f"path must be under XCUITestDemo tree: {rel}")
    return p


def verify_login() -> dict[str, Any]:
    """Install app + login flow via attempt chain with final expect."""
    t0 = time.time()
    ws = {"workspace": WORKSPACE}
    call_tool("ligh_up", {})
    call_tool("ligh_ready", {"settle_ms": 2500})
    call_tool(
        "ligh_cap_run_app",
        {"app": APP, "bundle_id": BUNDLE_ID, "settle_ms": 3500, "timeout_ms": 12000},
    )
    steps = [
        ("wait", {"id": "usernameTextField"}, None),
        ("tap", {"id": "usernameTextField"}, None),
        ("type", {"text": "alice"}, None),
        ("tap", {"id": "passwordSecureField"}, None),
        ("type", {"text": "secret"}, None),
        ("tap", {"id": "loginButton"}, {"see_id": "homeTitle"}),
    ]
    trace = []
    last: dict[str, Any] = {"ok": False}
    for intent, args, expect in steps:
        payload = {
            "intent": intent,
            "settle_ms": 2500,
            "timeout_ms": 12000,
            **ws,
            **args,
        }
        if expect:
            payload["expect"] = expect
        last = call_tool("ligh_attempt", payload)
        trace.append({"intent": intent, "args": args, "expect": expect, "result": last})
        if not last.get("ok") and intent == "tap" and expect:
            break
        if intent == "tap" and expect and not last.get("intent_met", last.get("ok")):
            break
    intent_met = bool(last.get("intent_met")) if "intent_met" in last else bool(
        last.get("ok") and (last.get("evidence") or {}).get("missing") == []
    )
    return {
        "ok": intent_met,
        "intent_met": intent_met,
        "evidence": last.get("evidence"),
        "hypotheses": (last.get("evidence") or {}).get("hypotheses") if isinstance(last.get("evidence"), dict) else None,
        "perceive_after": last.get("perceive_after"),
        "ms": int((time.time() - t0) * 1000),
        "trace": trace,
        "last": last,
    }


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    try:
        if action == "read_file":
            p = safe_path(str(act.get("path") or ""))
            with open(p, encoding="utf-8") as f:
                text = f.read()
            return {"ok": True, "path": p, "content": text[:12000]}

        if action == "write_file":
            p = safe_path(str(act.get("path") or ""))
            content = act.get("content")
            if not isinstance(content, str):
                return {"ok": False, "error": "content required"}
            with open(p, "w", encoding="utf-8") as f:
                f.write(content)
            return {"ok": True, "path": p}

        if action == "build_app":
            r = subprocess.run(
                [os.path.join(ROOT, "scripts/build-xcuitestdemo.sh")],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=300,
            )
            return {"ok": r.returncode == 0, "tail": (r.stdout or r.stderr or "")[-1200:]}

        if action == "ligh_ready":
            return call_tool("ligh_ready", {"settle_ms": 2500})

        if action == "ligh_verify_login":
            return verify_login()

        if action == "ligh_ux_hint":
            fp = str(act.get("fingerprint") or "")
            path = str(act.get("path") or act.get("source_path") or "")
            if not fp or not path:
                return {"ok": False, "error": "fingerprint and path required"}
            rel = os.path.relpath(safe_path(path), ROOT)
            return call_tool(
                "ligh_ux_hint",
                {"fingerprint": fp, "source_path": rel, "workspace": WORKSPACE},
            )

        if action == "done":
            return {"ok": True, "done": True, "summary": act.get("summary") or ""}

        return {"ok": False, "error": f"unknown action: {action}"}
    except Exception as e:
        return {"ok": False, "error": str(e)}


def main() -> int:
    goal = os.environ.get(
        "LIGH_AUTONOMOUS_GOAL",
        "XCUITestDemo login is broken — users cannot reach home. Fix Swift source; verify with LIGH QA layer.",
    )
    t0 = time.time()
    messages = [
        {"role": "system", "content": SYSTEM},
        {
            "role": "user",
            "content": goal + "\n\nStart with ligh_verify_login. Read evidence.hypotheses on failure.",
        },
    ]
    trace: list[dict[str, Any]] = []
    tokens_in = tokens_out = 0
    verified = False
    turns = 0

    for step in range(1, MAX_STEPS + 1):
        turns += 1
        chat = openai_chat(messages)
        act = chat["act"]
        tokens_in += chat["usage"]["prompt_tokens"]
        tokens_out += chat["usage"]["completion_tokens"]
        result = run_action(act)
        trace.append({"step": step, "action": act, "result": result})
        print(json.dumps({"step": step, "action": act.get("action"), "ok": result.get("ok")}), flush=True)

        if act.get("action") == "done":
            verified = any(
                t.get("action", {}).get("action") == "ligh_verify_login"
                and (t.get("result") or {}).get("intent_met")
                for t in trace
            )
            break

        if act.get("action") == "ligh_verify_login" and result.get("intent_met"):
            verified = True
            break

        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append({"role": "user", "content": json.dumps({"tool_result": result, "step": step})})

    doc = {
        "gate": "autonomous_agent_qa",
        "claim": "QA layer agent — ligh_attempt evidence + ux_hint (not app_job shortcut)",
        "goal": goal,
        "model": MODEL,
        "workspace": WORKSPACE,
        "steps_used": len(trace),
        "llm_turns": turns,
        "verified": verified,
        "claim_pass": verified,
        "tokens": {"in": tokens_in, "out": tokens_out},
        "total_ms": int((time.time() - t0) * 1000),
        "trace": trace,
    }
    out = os.environ.get(
        "LIGH_AUTONOMOUS_QA_OUT",
        os.path.join(ROOT, "docs/assets/autonomous-agent-qa-latest.json"),
    )
    with open(out, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")
    print(json.dumps({"claim_pass": verified, "turns": turns, "out": out}, indent=2))
    return 0 if verified else 1


if __name__ == "__main__":
    raise SystemExit(main())
