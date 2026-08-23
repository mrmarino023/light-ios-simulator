#!/usr/bin/env python3
"""Unified LIGH agent loop — multi-turn LLM + full MCP surface (no screenshots).

Usage:
  LIGH_AGENT_GOAL="Verify login on fixture" python3 scripts/agent-unified-loop.py
  LIGH_APP_PATH=…/App.app LIGH_APP_BUNDLE_ID=com.you python3 scripts/agent-unified-loop.py

Requires OPENAI_API_KEY for LLM mode. Without key, runs scripted app_goal smoke only.
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

MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
MAX_STEPS = int(os.environ.get("LIGH_AGENT_MAX_STEPS", "20"))
OUT = os.environ.get(
    "LIGH_AGENT_LOOP_OUT",
    os.path.join(ROOT, "docs/assets/agent-unified-loop-latest.json"),
)

APP = os.environ.get(
    "LIGH_APP_PATH",
    os.path.join(ROOT, "fixtures/LighFixture/build/LighFixture.app"),
)
BUNDLE_ID = os.environ.get("LIGH_APP_BUNDLE_ID", "dev.ligh.Fixture")

SYSTEM = """You control iOS Simulator via LIGH MCP (accessibility JSON — NOT screenshots).

Each turn reply with ONE JSON object:
{
  "action": "ready" | "observe" | "reach" | "explore" | "dismiss_overlay"
          | "app_goal" | "app_job" | "launch" | "done",
  "id": "...",
  "label": "...",
  "labels": ["Indirizzo", "Address"],
  "app": "path/to/App.app",
  "bundle_id": "...",
  "setup": [{"op":"launch","bundle_id":"..."}, {"op":"wait","labels":[...]}, ...],
  "postconditions": [{"wait_id":"..."} | {"wait_label":"..."}],
  "steps": [...],
  "summary": "..."
}

Rules:
- Call ready if eyes_unusable; observe when fault unclear.
- Prefer app_goal over tap loops. Use explore when target_missing or motor_no_effect.
- Read evidence.candidates and evidence.probes_tried on faults.
- Never claim success without ligh ok:true. No screenshots.
- setup ops: launch, wait, tap, type, key, dismiss_overlay, scroll_until, explore.
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


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    if action == "observe":
        return call_tool("ligh_observe", {"settle_ms": act.get("settle_ms") or 2500})
    if action == "ready":
        return call_tool("ligh_ready", {"settle_ms": 3500, "recover_homes": 6})
    if action == "reach":
        return call_tool(
            "ligh_cap_reach",
            {
                "id": act.get("id"),
                "label": act.get("label"),
                "timeout_ms": act.get("timeout_ms") or 14000,
                "max_swipes": act.get("max_swipes") or 12,
            },
        )
    if action == "explore":
        return call_tool(
            "ligh_cap_explore",
            {
                "id": act.get("id"),
                "label": act.get("label"),
                "timeout_ms": act.get("timeout_ms") or 18000,
                "max_probes": act.get("max_probes") or 4,
                "max_swipes": act.get("max_swipes") or 10,
            },
        )
    if action == "dismiss_overlay":
        return call_tool("ligh_cap_dismiss_overlay", {})
    if action == "launch":
        bid = act.get("bundle_id") or BUNDLE_ID
        return call_tool("ligh_launch", {"bundle_id": bid})
    if action == "app_goal":
        return call_tool(
            "ligh_cap_app_goal",
            {
                "app": act.get("app"),
                "bundle_id": act.get("bundle_id") or BUNDLE_ID,
                "setup": act.get("setup") or [],
                "postconditions": act.get("postconditions") or [],
                "timeout_ms": act.get("timeout_ms") or 20000,
            },
        )
    if action == "app_job":
        return call_tool(
            "ligh_cap_app_job",
            {
                "app": act.get("app") or APP,
                "bundle_id": act.get("bundle_id") or BUNDLE_ID,
                "steps": act.get("steps") or [],
                "timeout_ms": act.get("timeout_ms") or 20000,
            },
        )
    if action == "done":
        return {"ok": True, "done": True, "summary": act.get("summary") or ""}
    return {"ok": False, "error": f"unknown action {action}"}


def scripted_smoke() -> tuple[bool, list[dict[str, Any]]]:
    """No LLM — prove MCP loop on fixture."""
    trace: list[dict[str, Any]] = []
    steps = [
        {"op": "wait", "id": "LighHome"},
        {"op": "wait", "id": "NameField"},
        {"op": "type", "text": "agent"},
        {"op": "tap", "id": "GoNext"},
        {"op": "wait", "id": "LighDone"},
    ]
    for prep in (
        lambda: call_tool("ligh_ready", {"settle_ms": 3500, "recover_homes": 6}),
        lambda: call_tool("ligh_observe", {}),
    ):
        r = prep()
        trace.append({"action": prep.__name__, "result": r})
    r = call_tool(
        "ligh_cap_app_job",
        {"app": APP, "bundle_id": BUNDLE_ID, "steps": steps, "timeout_ms": 22000},
    )
    trace.append({"action": "app_job", "result": r})
    return bool(r.get("ok")), trace


def llm_loop(goal: str) -> tuple[bool, list[dict[str, Any]], int, int]:
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": SYSTEM},
        {"role": "user", "content": goal},
    ]
    trace: list[dict[str, Any]] = []
    tokens_in = tokens_out = 0
    verified = False

    call_tool("ligh_ready", {"settle_ms": 3500, "recover_homes": 6})

    for step in range(1, MAX_STEPS + 1):
        chat = openai_chat(messages)
        act = chat["act"]
        tokens_in += chat["usage"]["prompt_tokens"]
        tokens_out += chat["usage"]["completion_tokens"]
        result = run_action(act)
        row = {"step": step, "action": act, "result": result}
        trace.append(row)
        print(json.dumps({"step": step, "action": act.get("action"), "ok": result.get("ok")}), flush=True)

        if act.get("action") == "done":
            verified = any(
                (t.get("result") or {}).get("ok")
                for t in trace
                if (t.get("action") or {}).get("action") in ("app_goal", "app_job")
            )
            break

        if act.get("action") in ("app_goal", "app_job") and result.get("ok"):
            verified = True
            break

        fault = result.get("fault") or ""
        if fault in ("target_missing", "motor_no_effect") and act.get("action") not in ("explore", "reach"):
            hint = {"suggestion": "try explore or reach", "prior_fault": fault}
            result = {**result, **hint}

        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append({"role": "user", "content": json.dumps({"tool_result": result, "step": step})})

    return verified, trace, tokens_in, tokens_out


def main() -> int:
    goal = os.environ.get(
        "LIGH_AGENT_GOAL",
        f"Verify the Debug app at {APP} (bundle {BUNDLE_ID}): fill name field and reach LighDone screen.",
    )
    t0 = time.time()
    driver = "scripted"
    verified = False
    trace: list[dict[str, Any]] = []
    tokens_in = tokens_out = 0

    if os.environ.get("OPENAI_API_KEY", "").strip():
        driver = "openai"
        try:
            verified, trace, tokens_in, tokens_out = llm_loop(goal)
        except Exception as e:
            doc = {
                "gate": "agent_unified_loop",
                "driver": driver,
                "ok": False,
                "error": str(e),
                "goal": goal,
                "total_ms": int((time.time() - t0) * 1000),
            }
            with open(OUT, "w", encoding="utf-8") as f:
                f.write(json.dumps(doc, indent=2) + "\n")
            print(json.dumps(doc, indent=2))
            return 1
    else:
        verified, trace = scripted_smoke()

    doc = {
        "gate": "agent_unified_loop",
        "driver": driver,
        "model": MODEL if driver == "openai" else None,
        "goal": goal,
        "ok": verified,
        "steps_used": len(trace),
        "tokens": {"in": tokens_in, "out": tokens_out},
        "total_ms": int((time.time() - t0) * 1000),
        "trace": trace[-12:],
    }
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")
    print(json.dumps({"ok": verified, "driver": driver, "out": OUT}, indent=2))
    return 0 if verified else 1


if __name__ == "__main__":
    raise SystemExit(main())
