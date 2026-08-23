#!/usr/bin/env python3
"""Autonomous UX-graph agent — perceive/attempt/explore only, no scripted navigation.

The harness installs the app and sets a vague goal. The LLM must discover affordances
via ligh_perceive and act via ligh_attempt / ligh_find / ligh_dismiss. Success is
verified independently (success accessibility id present on final perceive), not by
agent self-report alone.
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

WORKSPACE = os.environ.get("LIGH_WORKSPACE", os.path.join(ROOT, "fixtures/LighOnboard"))
APP = os.environ.get(
    "LIGH_APP_PATH",
    os.path.join(ROOT, "fixtures/LighOnboard/build/LighOnboard.app"),
)
BUNDLE_ID = os.environ.get("LIGH_APP_BUNDLE_ID", "dev.ligh.Onboard")
SUCCESS_ID = os.environ.get("LIGH_UX_SUCCESS_ID", "HomeReady")
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
MAX_STEPS = int(os.environ.get("LIGH_UX_MAX_STEPS", "18"))

GOAL = os.environ.get(
    "LIGH_UX_GOAL",
    "You are dropped into an iOS onboarding app cold. Reach the final home-ready screen "
    "using only the QA/UX tools. Build a UX graph as you go (perceive records screens; "
    "attempt records transitions). Do not guess accessibility ids — read them from perceive.",
)

SYSTEM = """You control iOS Simulator through LIGH QA + UX graph tools (accessibility JSON only).

Each turn reply with ONE JSON object:
{
  "action": "ready" | "perceive" | "attempt" | "find" | "dismiss"
          | "ux_status" | "ux_baseline" | "ux_regress" | "ux_explore" | "ux_hint" | "done",
  "intent": "tap" | "type" | "wait" | "key",
  "id": "...",
  "label": "...",
  "text": "...",
  "key": "return",
  "expect": {"see_id": "..."} | {"see_label": "..."},
  "baseline": "name",
  "fingerprint": "fp_...",
  "source_path": "fixtures/LighOnboard/LighOnboard/ContentView.swift",
  "summary": "..."
}

Rules:
- Start with perceive after ready. Read affordances (id/label) before every attempt.
- attempt: always pass expect when you believe the screen should change.
- find: scroll until an off-screen control appears; dismiss: keyboard/sheet/alert.
- ux_status / ux_baseline / ux_explore: optional; graph auto-records on perceive/attempt.
- Never use app_job, app_goal, raw tap loops, or screenshots.
- On fault target_missing → find or perceive again; motor_no_effect → different affordance.
- Call done only when perceive shows you are on the final home screen.
- If eyes_unusable → ready, then perceive again.
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


def ws_args() -> dict[str, str]:
    return {"workspace": WORKSPACE}


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    ws = ws_args()

    if action == "ready":
        return call_tool("ligh_ready", {"settle_ms": act.get("settle_ms") or 2500, "recover_homes": 4})

    if action == "perceive":
        return call_tool(
            "ligh_perceive",
            {"settle_ms": act.get("settle_ms") or 2500, **ws},
        )

    if action == "attempt":
        intent = str(act.get("intent") or "tap")
        payload: dict[str, Any] = {
            "intent": intent,
            "settle_ms": act.get("settle_ms") or 2500,
            "timeout_ms": act.get("timeout_ms") or 12000,
            **ws,
        }
        if act.get("label"):
            payload["label"] = act["label"]
        if act.get("id"):
            payload["id"] = act["id"]
        if act.get("text"):
            payload["text"] = act["text"]
        if act.get("key"):
            payload["key"] = act["key"]
        if act.get("expect"):
            payload["expect"] = act["expect"]
        return call_tool("ligh_attempt", payload)

    if action == "find":
        payload = {
            "settle_ms": act.get("settle_ms") or 2500,
            "timeout_ms": act.get("timeout_ms") or 14000,
            "max_swipes": act.get("max_swipes") or 10,
        }
        if act.get("label"):
            payload["label"] = act["label"]
        if act.get("id"):
            payload["id"] = act["id"]
        if not payload.get("label") and not payload.get("id"):
            return {"ok": False, "fault": "target_missing", "error": "find needs id or label"}
        return call_tool("ligh_find", payload)

    if action == "dismiss":
        return call_tool("ligh_dismiss", {"settle_ms": act.get("settle_ms") or 2500})

    if action == "ux_status":
        return call_tool("ligh_ux_status", ws)

    if action == "ux_baseline":
        name = str(act.get("baseline") or act.get("name") or "agent-baseline")
        return call_tool(
            "ligh_ux_baseline",
            {"name": name, "settle_ms": act.get("settle_ms") or 2500, **ws},
        )

    if action == "ux_regress":
        base = str(act.get("baseline") or "")
        if not base:
            return {"ok": False, "error": "baseline name required"}
        return call_tool(
            "ligh_ux_regress",
            {"baseline": base, "settle_ms": act.get("settle_ms") or 2500, **ws},
        )

    if action == "ux_explore":
        return call_tool(
            "ligh_ux_explore",
            {
                "settle_ms": act.get("settle_ms") or 2500,
                "timeout_ms": act.get("timeout_ms") or 8000,
                "max_steps": act.get("max_steps") or 4,
                "max_depth": act.get("max_depth") or 2,
                **ws,
            },
        )

    if action == "ux_hint":
        fp = str(act.get("fingerprint") or "")
        sp = str(act.get("source_path") or "")
        if not fp or not sp:
            return {"ok": False, "error": "fingerprint and source_path required"}
        return call_tool("ligh_ux_hint", {"fingerprint": fp, "source_path": sp, **ws})

    if action == "done":
        return {"ok": True, "done": True, "summary": act.get("summary") or ""}

    return {"ok": False, "error": f"unknown action: {action}"}


def affordance_ids(perceive: dict[str, Any]) -> set[str]:
    out: set[str] = set()
    for a in perceive.get("affordances") or []:
        if isinstance(a, dict):
            if a.get("id"):
                out.add(str(a["id"]))
            if a.get("label"):
                out.add(str(a["label"]))
    loc = perceive.get("location") or {}
    if loc.get("title"):
        out.add(str(loc["title"]))
    return out


def harness_verify(success_id: str) -> dict[str, Any]:
    """Independent check — not visible to the LLM system prompt."""
    r = call_tool("ligh_perceive", {"settle_ms": 2500, **ws_args()})
    perceive = r.get("perceive") or {}
    ids = affordance_ids(perceive)
    found = success_id in ids
    ux = call_tool("ligh_ux_status", ws_args())
    detail = ux.get("detail") if isinstance(ux, dict) else {}
    summary = detail.get("summary") if isinstance(detail, dict) else {}
    if not isinstance(summary, dict):
        summary = {}
    node_count = summary.get("node_count")
    edge_count = summary.get("edge_count")
    return {
        "ok": found,
        "success_id": success_id,
        "seen_ids": sorted(ids)[:24],
        "fingerprint": (perceive.get("location") or {}).get("fingerprint"),
        "ux_graph": {"node_count": node_count, "edge_count": edge_count},
        "perceive_ok": bool(r.get("ok")),
    }


def bootstrap_app() -> dict[str, Any]:
    call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
    return call_tool(
        "ligh_cap_run_app",
        {"app": APP, "bundle_id": BUNDLE_ID, "settle_ms": 3500, "timeout_ms": 15000},
    )


def main() -> int:
    if not os.environ.get("OPENAI_API_KEY", "").strip():
        print(json.dumps({"ok": False, "error": "OPENAI_API_KEY missing"}), file=sys.stderr)
        return 1

    t0 = time.time()
    bootstrap = bootstrap_app()
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": SYSTEM},
        {
            "role": "user",
            "content": GOAL + f"\n\nApp bundle: {BUNDLE_ID}. Workspace: {WORKSPACE}. Bootstrap: {bootstrap.get('ok')}",
        },
    ]
    trace: list[dict[str, Any]] = []
    tokens_in = tokens_out = 0
    agent_claimed_done = False

    for step in range(1, MAX_STEPS + 1):
        chat = openai_chat(messages)
        act = chat["act"]
        tokens_in += chat["usage"]["prompt_tokens"]
        tokens_out += chat["usage"]["completion_tokens"]
        result = run_action(act)
        trace.append({"step": step, "action": act, "result": result})
        print(
            json.dumps(
                {
                    "step": step,
                    "action": act.get("action"),
                    "ok": result.get("ok"),
                    "intent_met": result.get("intent_met"),
                }
            ),
            flush=True,
        )

        if act.get("action") == "done":
            agent_claimed_done = True
            break

        fault = result.get("fault") or ""
        hint = {}
        if fault in ("target_missing", "motor_no_effect", "intent_unmet"):
            hint = {"suggestion": "perceive again or find/dismiss; read affordances"}
        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append(
            {
                "role": "user",
                "content": json.dumps({"tool_result": {**result, **hint}, "step": step}),
            }
        )

    verify = harness_verify(SUCCESS_ID)
    verified = bool(verify.get("ok"))
    graph_nodes = (verify.get("ux_graph") or {}).get("node_count") or 0
    graph_edges = (verify.get("ux_graph") or {}).get("edge_count") or 0
    graph_grew = (graph_nodes or 0) >= 2 and (graph_edges or 0) >= 1

    doc = {
        "gate": "autonomous_ux",
        "claim": "Autonomous QA/UX agent — no scripted navigation; harness verifies success id",
        "app": APP,
        "bundle_id": BUNDLE_ID,
        "workspace": WORKSPACE,
        "goal": GOAL,
        "success_id": SUCCESS_ID,
        "model": MODEL,
        "bootstrap": bootstrap,
        "agent_claimed_done": agent_claimed_done,
        "verified": verified,
        "graph_grew": graph_grew,
        "claim_pass": verified and graph_grew,
        "harness_verify": verify,
        "steps_used": len(trace),
        "tokens": {"in": tokens_in, "out": tokens_out},
        "total_ms": int((time.time() - t0) * 1000),
        "trace": trace[-14:],
    }
    out = os.environ.get(
        "LIGH_UX_AGENT_OUT",
        os.path.join(ROOT, "docs/assets/autonomous-ux-latest.json"),
    )
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")
    print(json.dumps({"claim_pass": doc["claim_pass"], "verified": verified, "out": out}, indent=2))
    return 0 if doc["claim_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
