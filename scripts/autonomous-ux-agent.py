#!/usr/bin/env python3
"""Autonomous UX-graph agent — perceive/attempt only, no scripted navigation.

Arms (LIGH_UX_ARM):
  control  — QA only, no workspace / no graph persistence, no ux_* tools
  discover — build uxgraph while completing the goal (default)
  replay   — same graph as prior discover; must use ux_status first

Harness verifies success_id independently (agent prompt never names it).
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from ligh_mcp import call_tool  # noqa: E402

ARM = os.environ.get("LIGH_UX_ARM", "control").lower()
WORKSPACE = os.environ.get("LIGH_WORKSPACE", os.path.join(ROOT, "fixtures/LighOnboard"))
APP = os.environ.get(
    "LIGH_APP_PATH",
    os.path.join(ROOT, "fixtures/LighOnboard/build/LighOnboard.app"),
)
BUNDLE_ID = os.environ.get("LIGH_APP_BUNDLE_ID", "dev.ligh.Onboard")
SUCCESS_ID = os.environ.get("LIGH_UX_SUCCESS_ID", "HomeReady")
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
MAX_STEPS = int(os.environ.get("LIGH_UX_MAX_STEPS", "20"))

GOAL = os.environ.get(
    "LIGH_UX_GOAL",
    "You are dropped into an iOS app cold. Complete the user flow until you reach the "
    "final success screen. Use perceive to read accessibility ids before every action. "
    "Never guess ids. Never use app_job or app_goal.",
)

SYSTEM_FEW_SHOT = """
Winning patterns (from green runs — adapt ids to what perceive returns):

Onboarding:
  perceive → {"action":"attempt","intent":"tap","id":"OnboardSkip","expect":{"see_id":"HomeReady"},...}
  OR: tap Continue → type name field → tap Next/Finish, with expect after each navigation tap.
  perceive → confirm success screen → done

Login (OSS apps):
  perceive → read usernameTextField, passwordSecureField, loginButton ids
  attempt tap field → type username → tap password field → type password → dismiss keyboard if needed
  attempt tap loginButton with expect {"see_id":"<success id from goal>"} or {"see_label":"..."}
  perceive → done only when success screen is visible

Recovery:
  intent_unmet after tap often means expect was wrong, not that tap failed — perceive again and adjust expect.
  One perceive before each attempt; ~4 perceives for a 4-step login is normal.
"""

SYSTEM_BASE = """You control iOS Simulator through LIGH QA tools (accessibility JSON only — no screenshots).

Each turn reply with ONE JSON object:
{
  "action": "ready" | "perceive" | "attempt" | "find" | "dismiss" | "done",
  "intent": "tap" | "type" | "wait" | "key",
  "id": "...",
  "label": "...",
  "text": "...",
  "key": "return",
  "expect": {"see_id": "..."} | {"see_label": "..."},
  "summary": "..."
}

Rules:
- Start with perceive. Read affordances (id/label) before every attempt.
- attempt: pass expect when you believe the screen should change.
- find: scroll for off-screen controls; dismiss: keyboard/sheet/alert.
- Never use app_job, app_goal, or raw tap loops.
- On target_missing → find or perceive; motor_no_effect → different affordance.
- Call done only when perceive shows the final success screen (login flow complete).
- If done is rejected, you are NOT on the success screen — perceive and continue.
- If perceive shows SpringBoard/home screen icons, launch the app (attempt tap app icon) or wait for app surface.
""" + SYSTEM_FEW_SHOT

SYSTEM_GRAPH = SYSTEM_BASE + """
UX graph tools (also allowed):
  ux_status — summary of known screens/transitions recorded in this workspace
  ux_explore — safe BFS when stuck (optional)

Graph auto-records on perceive/attempt when workspace is set.
"""

SYSTEM_REPLAY = SYSTEM_GRAPH + """
REPLAY MODE: A UX graph already exists from a prior successful run.
You MUST call ux_status on step 1 before perceive.
Use known fingerprints and affordance labels from ux_status to choose attempts faster.
Do not re-discover from scratch if the graph already lists the next transition.
"""


def system_prompt() -> str:
    if ARM == "control":
        return SYSTEM_BASE
    if ARM == "replay":
        return SYSTEM_REPLAY
    return SYSTEM_GRAPH


def openai_chat(messages: list[dict[str, Any]]) -> dict[str, Any]:
    import subprocess
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
    if ARM == "control":
        return {}
    return {"workspace": WORKSPACE}


def ux_tools_allowed() -> bool:
    return ARM in ("discover", "replay")


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    ws = ws_args()

    if action == "ready":
        return call_tool("ligh_ready", {"settle_ms": act.get("settle_ms") or 2500, "recover_homes": 4})

    if action == "perceive":
        return call_tool("ligh_perceive", {"settle_ms": act.get("settle_ms") or 2500, **ws})

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

    if not ux_tools_allowed():
        return {"ok": False, "error": f"action {action} not allowed in arm={ARM}"}

    if action == "ux_status":
        return call_tool("ligh_ux_status", ws)

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
    r = call_tool("ligh_perceive", {"settle_ms": 2500, **ws_args()})
    perceive = r.get("perceive") or {}
    ids = affordance_ids(perceive)
    found = success_id in ids
    node_count = edge_count = None
    if ux_tools_allowed():
        ux = call_tool("ligh_ux_status", ws_args())
        detail = ux.get("detail") if isinstance(ux, dict) else {}
        summary = detail.get("summary") if isinstance(detail, dict) else {}
        if isinstance(summary, dict):
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


def affordance_keys(perceive: dict[str, Any]) -> set[str]:
    keys: set[str] = set()
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        for k in ("id", "label", "identifier"):
            if a.get(k):
                keys.add(str(a[k]))
    loc = perceive.get("location") or {}
    if loc.get("title"):
        keys.add(str(loc["title"]))
    return keys


def on_springboard(perceive: dict[str, Any], app_label: str) -> bool:
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        ident = a.get("identifier") or a.get("label") or ""
        if ident == app_label and a.get("hittable", True):
            return True
    return False


def foreground_ready(
    perceive: dict[str, Any], bundle_id: str, app_label: str, markers: list[str]
) -> bool:
    keys = affordance_keys(perceive)
    if markers and any(m in keys for m in markers):
        return True
    if on_springboard(perceive, app_label):
        return False
    loc = perceive.get("location") or {}
    return loc.get("bundle_id") == bundle_id and loc.get("surface") == "app"


def bootstrap_app() -> dict[str, Any]:
    call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
    boot = call_tool(
        "ligh_cap_run_app",
        {"app": APP, "bundle_id": BUNDLE_ID, "settle_ms": 3500, "timeout_ms": 15000},
    )
    # Ensure app foreground — run_app can install without bringing app to front.
    app_label = os.path.basename(APP).replace(".app", "")
    markers = [m.strip() for m in os.environ.get("LIGH_UX_IN_APP_MARKERS", SUCCESS_ID).split(",") if m.strip()]
    for attempt in range(1, 6):
        p = call_tool("ligh_perceive", {"settle_ms": 2500, **ws_args()})
        perceive = (p.get("perceive") or {})
        if foreground_ready(perceive, BUNDLE_ID, app_label, markers):
            return {**boot, "foreground_attempt": attempt, "foreground_ok": True}
        call_tool("ligh_launch", {"bundle_id": BUNDLE_ID})
        time.sleep(1.2)
        call_tool(
            "ligh_attempt",
            {
                "intent": "tap",
                "label": app_label,
                "settle_ms": 2000,
                "timeout_ms": 8000,
                **ws_args(),
            },
        )
        time.sleep(1.0)
    return {**boot, "foreground_ok": False, "warning": "app may still be on SpringBoard"}


def count_actions(trace: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    intent_met = intent_fail = 0
    for row in trace:
        act = row.get("action") or {}
        name = act.get("action") or "?"
        counts[name] = counts.get(name, 0) + 1
        res = row.get("result") or {}
        if name == "attempt":
            if res.get("intent_met"):
                intent_met += 1
            elif "intent_met" in res:
                intent_fail += 1
    counts["attempt_intent_met"] = intent_met
    counts["attempt_intent_fail"] = intent_fail
    return counts


def run_agent() -> dict[str, Any]:
    t0 = time.time()
    bootstrap = bootstrap_app()
    user_extra = ""
    if ARM == "replay":
        user_extra = "\n\nREPLAY: call ux_status first, then complete the flow using graph memory."

    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system_prompt()},
        {
            "role": "user",
            "content": GOAL + f"\n\nArm: {ARM}. Bundle: {BUNDLE_ID}. Bootstrap ok: {bootstrap.get('ok')}.{user_extra}",
        },
    ]
    trace: list[dict[str, Any]] = []
    tokens_in = tokens_out = 0

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
                    "arm": ARM,
                    "step": step,
                    "action": act.get("action"),
                    "ok": result.get("ok"),
                    "intent_met": result.get("intent_met"),
                }
            ),
            flush=True,
        )

        if act.get("action") == "done":
            pre = harness_verify(SUCCESS_ID)
            if pre.get("ok"):
                trace.append({"step": step, "action": act, "result": {"ok": True, "done": True, "harness_pre": pre}})
                break
            result = {
                "ok": False,
                "done_rejected": True,
                "error": "harness: success screen not visible — keep going",
                "harness_pre": pre,
            }
            trace.append({"step": step, "action": act, "result": result})
            messages.append({"role": "assistant", "content": json.dumps(act)})
            messages.append(
                {
                    "role": "user",
                    "content": json.dumps(
                        {
                            "tool_result": result,
                            "step": step,
                            "suggestion": "perceive again; use attempt tap/type on login fields; dismiss keyboard if needed",
                        }
                    ),
                }
            )
            continue

        fault = result.get("fault") or ""
        hint = {}
        if fault in ("target_missing", "motor_no_effect", "intent_unmet"):
            hint = {
                "suggestion": (
                    "perceive again; read affordances. "
                    "If intent_unmet after tap, expect may be wrong — perceive and retry with updated expect."
                )
            }
        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append(
            {"role": "user", "content": json.dumps({"tool_result": {**result, **hint}, "step": step})}
        )

    verify = harness_verify(SUCCESS_ID)
    verified = bool(verify.get("ok"))
    g = verify.get("ux_graph") or {}
    graph_grew = (g.get("node_count") or 0) >= 2 and (g.get("edge_count") or 0) >= 1

    return {
        "arm": ARM,
        "verified": verified,
        "graph_grew": graph_grew if ux_tools_allowed() else None,
        "harness_verify": verify,
        "action_counts": count_actions(trace),
        "steps_used": len(trace),
        "tokens": {"in": tokens_in, "out": tokens_out, "total": tokens_in + tokens_out},
        "total_ms": int((time.time() - t0) * 1000),
        "bootstrap": bootstrap,
        "trace": trace[-16:],
    }


def main() -> int:
    if not os.environ.get("OPENAI_API_KEY", "").strip():
        print(json.dumps({"ok": False, "error": "OPENAI_API_KEY missing"}), file=sys.stderr)
        return 1

    doc = {
        "gate": "autonomous_ux",
        "arm": ARM,
        "app": APP,
        "bundle_id": BUNDLE_ID,
        "workspace": WORKSPACE if ARM != "control" else None,
        "goal": GOAL,
        "success_id": SUCCESS_ID,
        "model": MODEL,
        **run_agent(),
    }
    bootstrap = doc.get("bootstrap") or {}
    if bootstrap.get("foreground_ok") is False:
        doc["claim_pass"] = False
        doc["fail_reason"] = "app_never_foregrounded"
    elif ARM == "control":
        doc["claim_pass"] = doc["verified"]
    else:
        nodes = (doc.get("harness_verify") or {}).get("ux_graph", {}).get("node_count") or 0
        edges = (doc.get("harness_verify") or {}).get("ux_graph", {}).get("edge_count") or 0
        doc["claim_pass"] = doc["verified"] and nodes >= 2 and edges >= 1

    out = os.environ.get(
        "LIGH_UX_AGENT_OUT",
        os.path.join(ROOT, "docs/assets/autonomous-ux-latest.json"),
    )
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")
    print(json.dumps({"arm": ARM, "claim_pass": doc["claim_pass"], "verified": doc["verified"], "out": out}))
    return 0 if doc["claim_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
