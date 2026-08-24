#!/usr/bin/env python3
"""LIGH MCP server for coding agents (stdio JSON-RPC).

Product surface: settled AX observe → label-first act → verify.
Screenshots are debug-only. Local Mac only (`lighd` Unix socket).

Cursor mcp.json example:
  {
    "mcpServers": {
      "ligh": {
        "command": "python3",
        "args": ["/absolute/path/to/light-simulatior-ios/scripts/ligh_mcp.py"],
        "env": {
          "LIGH_BIN": "/absolute/path/to/light-simulatior-ios/target/release/ligh"
        }
      }
    }
  }
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
AGENT_MD = os.path.join(ROOT, "docs", "AGENT.md")


def ligh(*args: str, timeout: float = 120) -> dict[str, Any]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    out = (p.stdout or "").strip()
    err = (p.stderr or "").strip()
    for blob in (out, err):
        if blob.startswith("{") or blob.startswith("["):
            try:
                data = json.loads(blob)
                if isinstance(data, dict):
                    # Cap/observe JSON is valid even when ok:false (fail-closed fault).
                    return {"ok": True, "data": data}
                return {"ok": True, "data": data}
            except json.JSONDecodeError:
                pass
    if p.returncode != 0:
        return {"ok": False, "error": err or out or f"exit {p.returncode}"}
    if out.startswith("{") or out.startswith("["):
        try:
            return {"ok": True, "data": json.loads(out)}
        except json.JSONDecodeError:
            pass
    return {"ok": True, "data": out}


def compact_eyes(snap: dict[str, Any]) -> dict[str, Any]:
    """Agent view: never hand transition/empty as a fake tree."""
    aq = snap.get("ax_quality") or "error"
    settled = bool(snap.get("settled"))
    scene = snap.get("scene") or {}
    unusable = bool(snap.get("eyes_unusable")) or aq in ("empty", "transition", "error") or not settled
    out: dict[str, Any] = {
        "ok": True,
        "ax_quality": aq,
        "settled": settled,
        "phase": snap.get("phase"),
        "surface": scene.get("surface"),
        "keyboard_visible": scene.get("keyboard_visible"),
        "screen_title": scene.get("screen_title"),
        "eyes_unusable": unusable,
        "events": [
            {"kind": e.get("kind"), "payload": e.get("payload")}
            for e in (snap.get("events") or [])[-8:]
        ],
        "actionable_topk": [],
    }
    if unusable:
        out["suggestion"] = "Call ligh_ready (ensure_ready). Do not invent UI."
        out["fault"] = "eyes_unusable"
        return out
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
    out["actionable_topk"] = top
    return out


def compact_cap(raw: dict[str, Any]) -> dict[str, Any]:
    """Agent view of a capability result — verified or explicit fault with evidence."""
    if not raw.get("ok"):
        return {
            "ok": False,
            "fault": "infra",
            "error": raw.get("error"),
            "suggestion": "ligh_ready then retry",
        }
    data = raw.get("data")
    if not isinstance(data, dict):
        return {"ok": False, "fault": "infra", "error": "cap returned non-object"}
    detail = data.get("detail")
    evidence: dict[str, Any] = {}
    if isinstance(detail, dict):
        inner = detail.get("detail") if isinstance(detail.get("detail"), dict) else detail
        if isinstance(inner, dict):
            for key in ("candidates", "actionable_topk", "scene", "wanted", "error"):
                if inner.get(key) is not None:
                    evidence[key] = inner[key]
    obs = data.get("observe")
    if isinstance(obs, dict) and not evidence.get("actionable_topk"):
        top = []
        for n in (obs.get("actionable_topk") or [])[:15]:
            top.append(
                {
                    "id": n.get("identifier") or n.get("id"),
                    "label": n.get("label") or n.get("text"),
                    "role": n.get("role"),
                    "focused": n.get("focused"),
                    "hittable": n.get("hittable"),
                }
            )
        if top:
            evidence["actionable_topk"] = top
    out: dict[str, Any] = {
        "ok": bool(data.get("ok")),
        "fault": data.get("fault") or ("ok" if data.get("ok") else "fail"),
        "capability": data.get("capability"),
        "phase": data.get("phase"),
        "overlay": data.get("overlay"),
        "surface": data.get("surface"),
        "detail": detail,
    }
    if evidence:
        out["evidence"] = evidence
    fault = out.get("fault")
    if fault == "motor_no_effect":
        out["suggestion"] = "UI unchanged after tap — try ligh_cap_reach or fix affordance; do not retry same tap"
    elif fault == "target_missing":
        out["suggestion"] = "read evidence.candidates; use ligh_cap_reach or fix accessibility id"
    return out


def compact_qa_cap(raw: dict[str, Any]) -> dict[str, Any]:
    """Flatten QA capability (perceive/attempt/find/dismiss) for agents."""
    base = compact_cap(raw)
    data = raw.get("data") if raw.get("ok") else None
    if isinstance(data, dict) and isinstance(data.get("detail"), dict):
        detail = data["detail"]
        if "perceive" in detail:
            base["perceive"] = detail["perceive"]
        if "feel" in detail:
            # Feel IR — compact host world model (prefer over raw affordance dumps).
            base["feel"] = detail["feel"]
        if "verdict" in detail:
            v = detail["verdict"]
            base["intent_met"] = v.get("intent_met")
            base["evidence"] = v.get("evidence")
            base["perceive_after"] = v.get("perceive_after")
            if v.get("fault"):
                base["fault"] = v["fault"]
    return base


def compact_autopilot(raw: dict[str, Any]) -> dict[str, Any]:
    """Autopilot verdict for a code-fixing agent: outcome + why, no AX dump.

    On failure the agent gets a semantic diagnosis and (when known) the source file
    correlated with the failing screen — evidence to fix code, not a bare timeout.
    """
    base = compact_cap(raw)
    detail = base.get("detail")
    if not isinstance(detail, dict):
        return base
    out: dict[str, Any] = {
        "ok": bool(base.get("ok")),
        "fault": base.get("fault"),
        "reached": bool(detail.get("reached")),
        "goal": detail.get("goal"),
        "steps": detail.get("steps"),
        "elapsed_ms": detail.get("elapsed_ms"),
        "llm_tokens": detail.get("llm_tokens", 0),
    }
    # The pilot can die before it ever plans (install/launch). Surface that as infra,
    # so the agent does not read an environment failure as a bug in its own patch.
    if "reached" not in detail:
        inner = detail.get("detail") if isinstance(detail.get("detail"), dict) else {}
        out["stage"] = detail.get("stage") or inner.get("step")
        err = inner.get("error") or inner.get("detail") or detail.get("error")
        if err:
            out["error"] = err if isinstance(err, str) else str(err)[:400]
        out["host_error"] = True
        return out
    if detail.get("diagnosis"):
        out["diagnosis"] = detail["diagnosis"]
    if detail.get("source_hint"):
        out["source_hint"] = detail["source_hint"]
    if detail.get("stop_code"):
        out["stop_code"] = detail["stop_code"]
    # Trace stays short: what was driven, not every observation.
    trace = detail.get("trace")
    if isinstance(trace, list):
        out["trace"] = [
            {
                "step": t.get("step"),
                "intent": (t.get("act") or {}).get("intent"),
                "target": (t.get("act") or {}).get("id") or (t.get("act") or {}).get("label"),
                "fired": t.get("fired"),
                "changed": t.get("changed"),
            }
            for t in trace
        ]
    return out


def ligh_result_path(raw: dict[str, Any]) -> str | None:
    """Extract a filesystem path from nested ligh() / call_tool responses."""
    if raw.get("path"):
        return str(raw["path"])
    data = raw.get("data")
    if isinstance(data, dict):
        if data.get("path"):
            return str(data["path"])
        inner = data.get("data")
        if isinstance(inner, dict) and inner.get("path"):
            return str(inner["path"])
    detail = raw.get("detail")
    if isinstance(detail, dict) and detail.get("path"):
        return str(detail["path"])
    return None


def _perceive_qa(settle_ms: int = 2500, workspace: str | None = None) -> dict[str, Any]:
    ms = str(settle_ms)
    cmd = ["--json", "cap", "perceive", "--settle-ms", ms]
    if workspace:
        cmd += ["--workspace", workspace]
    return compact_qa_cap(ligh(*cmd))


def _perceive_usable(perceive: dict[str, Any] | None) -> bool:
    if not perceive:
        return False
    if perceive.get("eyes_unusable"):
        return False
    if perceive.get("ready"):
        return True
    return bool(perceive.get("affordances"))


def route_perceive(
    settle_ms: int = 2500,
    *,
    vision_fallback: bool = True,
    recover_homes: int = 4,
    workspace: str | None = None,
) -> dict[str, Any]:
    """AX-first perceive with optional vision escalation.

    channel=ax     — plan on affordances (default path)
    channel=vision — only after ligh_ready retry still eyes_unusable
    channel=none   — hard fail, no screenshot spam
    """
    first = _perceive_qa(settle_ms, workspace)
    perceive = first.get("perceive") or {}
    if _perceive_usable(perceive):
        return {**first, "channel": "ax", "vision_escalated": False, "route": "ax_first"}

    ready = ligh(
        "--json",
        "ready",
        "--settle-ms",
        str(settle_ms),
        "--recover-homes",
        str(recover_homes),
    )
    second = _perceive_qa(settle_ms, workspace)
    perceive2 = second.get("perceive") or {}
    if _perceive_usable(perceive2):
        return {
            **second,
            "channel": "ax",
            "vision_escalated": False,
            "route": "ax_after_ready",
            "ready_result": ready,
        }

    if not vision_fallback:
        return {
            **second,
            "ok": False,
            "channel": "none",
            "vision_escalated": False,
            "route": "fail_closed",
            "fault": "eyes_unusable",
            "ready_result": ready,
            "suggestion": "AX still unusable after ligh_ready — fix session trust; do not spam screenshots",
        }

    shot_path = os.path.expanduser("~/.ligh/routed-screenshot.png")
    os.makedirs(os.path.dirname(shot_path), exist_ok=True)
    shot = ligh("--json", "screenshot", "-o", shot_path)
    path = ligh_result_path(shot) or shot_path
    return {
        **second,
        "ok": True,
        "channel": "vision",
        "vision_escalated": True,
        "route": "vision_fallback",
        "ready_result": ready,
        "screenshot_path": path,
        "suggestion": "Vision escalation — use coordinates once, then perceive again for AX",
    }


def agent_rules_text() -> str:
    if os.path.isfile(AGENT_MD):
        return open(AGENT_MD, encoding="utf-8").read()
    return (
        "LIGH: observe(settle) → tap --label → observe again. "
        "Never plan from screenshots. If eyes_unusable, home and re-observe."
    )


TOOLS = [
    {
        "name": "ligh_agent_rules",
        "description": "Return LIGH agent instructions (settle loop, honesty, IT/EN labels).",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "ligh_status",
        "description": "Daemon + session status (booted udid, footprint).",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "ligh_up",
        "description": "Boot/ensure a Simulator session (headless). Call once before the loop.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "device": {"type": "string", "default": "iphone-15-pro"},
            },
        },
    },
    {
        "name": "ligh_observe",
        "description": "Settled accessibility observe (no PNG). Prefer this over screenshots.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "settle_ms": {"type": "integer", "default": 2500},
            },
        },
    },
    {
        "name": "ligh_tap",
        "description": "Tap by accessibility label (preferred) or normalized x,y.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "id": {"type": "string"},
                "x": {"type": "number"},
                "y": {"type": "number"},
            },
        },
    },
    {
        "name": "ligh_type",
        "description": "Type ASCII text via HID. Success means host accepted keystrokes.",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        },
    },
    {
        "name": "ligh_home",
        "description": "Press Home. Use when eyes_unusable or to return to SpringBoard.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "ligh_wait",
        "description": "Wait until label or id appears.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "id": {"type": "string"},
                "timeout_ms": {"type": "integer", "default": 8000},
            },
        },
    },
    {
        "name": "ligh_key",
        "description": "Press a named key: return, delete, escape, tab, space, arrows.",
        "inputSchema": {
            "type": "object",
            "properties": {"name": {"type": "string", "default": "return"}},
        },
    },
    {
        "name": "ligh_swipe",
        "description": "Swipe gesture (normalized 0–1). Explore / scroll when reach is not enough.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from_x": {"type": "number", "default": 0.5},
                "from_y": {"type": "number", "default": 0.8},
                "to_x": {"type": "number", "default": 0.5},
                "to_y": {"type": "number", "default": 0.2},
            },
        },
    },
    {
        "name": "ligh_scroll_until",
        "description": "Swipe-scroll until label/id is on-screen.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "id": {"type": "string"},
                "max_swipes": {"type": "integer", "default": 8},
                "timeout_ms": {"type": "integer", "default": 12000},
            },
        },
    },
    {
        "name": "ligh_run",
        "description": "Install (if needed) and launch an .app under test. Prefer for coding-agent app flows.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "Path to .app bundle"},
                "bundle_id": {"type": "string"},
            },
            "required": ["app"],
        },
    },
    {
        "name": "ligh_launch",
        "description": "Launch an already-installed app by bundle id (simctl). Use for system apps or after install.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "bundle_id": {"type": "string"},
            },
            "required": ["bundle_id"],
        },
    },
    {
        "name": "ligh_relaunch",
        "description": "Relaunch the session app_bundle_id from the last ligh_run.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "ligh_sense",
        "description": "Recent sensation events only.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "ligh_screenshot",
        "description": "DEBUG only — write PNG. Do not use for planning.",
        "inputSchema": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
        },
    },
    {
        "name": "ligh_ready",
        "description": "Control-plane ensure_ready: recover AX until Ready or structured fault.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "settle_ms": {"type": "integer", "default": 2500},
                "recover_homes": {"type": "integer", "default": 6},
            },
        },
    },
    {
        "name": "ligh_cap_open_settings",
        "description": "Capability: open Settings with act-with-settle. Returns fault class on failure.",
        "inputSchema": {
            "type": "object",
            "properties": {"settle_ms": {"type": "integer", "default": 2500}},
        },
    },
    {
        "name": "ligh_cap_settings_search",
        "description": "Capability: Settings search + type query (e.g. Bluetooth).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
            },
            "required": ["query"],
        },
    },
    {
        "name": "ligh_cap_assert_surface",
        "description": "Capability: assert scene.surface after settle.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "surface": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
            },
            "required": ["surface"],
        },
    },
    {
        "name": "ligh_cap_tap",
        "description": "Capability: settle → tap label/id → settle.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "id": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
            },
        },
    },
    {
        "name": "ligh_cap_type",
        "description": "Capability: settle → type → settle.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
            },
            "required": ["text"],
        },
    },
    {
        "name": "ligh_cap_run_app",
        "description": "Product path: install Debug .app → launch → settle → optional wait_label. Use for apps under development.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "Absolute path to .app"},
                "bundle_id": {"type": "string"},
                "wait_label": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 3500},
                "timeout_ms": {"type": "integer", "default": 8000},
            },
            "required": ["app"],
        },
    },
    {
        "name": "ligh_cap_autopilot",
        "description": (
            "Host drives the UI to a goal and verifies it — zero LLM tokens, path discovered "
            "at runtime from Feel IR. Give the acceptance target and any data the flow needs; "
            "never a step list. On failure returns a semantic diagnosis plus source_hint."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "Absolute path to .app (installs + launches)"},
                "bundle_id": {"type": "string"},
                "goal_id": {"type": "string", "description": "Acceptance target: accessibility identifier"},
                "goal_label": {"type": "string", "description": "Acceptance target: visible label"},
                "params": {
                    "type": "array",
                    "description": "Data for text fields, in form order: [{value, secure}]",
                    "items": {
                        "type": "object",
                        "properties": {
                            "value": {"type": "string"},
                            "secure": {"type": "boolean", "default": False},
                        },
                        "required": ["value"],
                    },
                },
                "max_steps": {"type": "integer", "default": 24},
                "workspace": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 1500},
                "timeout_ms": {"type": "integer", "default": 8000},
            },
        },
    },
    {
        "name": "ligh_cap_wait_label",
        "description": "Capability: settle → wait until AX label exists (app chrome).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
                "timeout_ms": {"type": "integer", "default": 8000},
            },
            "required": ["label"],
        },
    },
    {
        "name": "ligh_cap_app_job",
        "description": "Product job: install/relaunch Debug .app then motor steps (wait/tap/type). Returns ok+fault — fail-closed for coding agents.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "Absolute path to .app"},
                "bundle_id": {"type": "string"},
                "steps": {
                    "type": "array",
                    "description": "[{op:wait|tap|type, id?, label?, text?}]",
                    "items": {"type": "object"},
                },
                "settle_ms": {"type": "integer", "default": 3500},
                "timeout_ms": {"type": "integer", "default": 12000},
                "no_install": {"type": "boolean", "default": False},
                "launch_args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Extra argv for simctl launch (e.g. --ui_test_login_failure)",
                },
            },
            "required": ["app", "steps"],
        },
    },
    {
        "name": "ligh_perceive",
        "description": "QA layer: settled world model + Feel IR (place/salience/block/delta). Prefer feel for planning.",
        "inputSchema": {
            "type": "object",
            "properties": {"settle_ms": {"type": "integer", "default": 2500}},
        },
    },
    {
        "name": "ligh_perceive_routed",
        "description": "AX-first perceive router: perceive → ligh_ready retry → vision screenshot only if still eyes_unusable. Returns channel=ax|vision|none.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "settle_ms": {"type": "integer", "default": 2500},
                "vision_fallback": {"type": "boolean", "default": True},
                "recover_homes": {"type": "integer", "default": 4},
            },
        },
    },
    {
        "name": "ligh_attempt",
        "description": "QA layer: tap|type|key with built-in verify. Returns intent_met + evidence.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "intent": {"type": "string", "enum": ["tap", "type", "key"]},
                "label": {"type": "string"},
                "id": {"type": "string"},
                "text": {"type": "string"},
                "key": {"type": "string"},
                "expect": {"type": "object"},
                "settle_ms": {"type": "integer", "default": 2500},
                "timeout_ms": {"type": "integer", "default": 8000},
            },
            "required": ["intent"],
        },
    },
    {
        "name": "ligh_find",
        "description": "QA layer: locate label/id on screen; scroll_until host-owned.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "id": {"type": "string"},
                "scroll": {"type": "boolean", "default": True},
                "settle_ms": {"type": "integer", "default": 2500},
                "timeout_ms": {"type": "integer", "default": 12000},
                "max_swipes": {"type": "integer", "default": 8},
            },
        },
    },
    {
        "name": "ligh_cap_reach",
        "description": "Motor: dismiss overlay, scroll, wait for id/label. Returns candidates on target_missing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "label": {"type": "string"},
                "max_swipes": {"type": "integer", "default": 12},
                "settle_ms": {"type": "integer", "default": 2500},
                "timeout_ms": {"type": "integer", "default": 12000},
            },
        },
    },
    {
        "name": "ligh_dismiss",
        "description": "QA layer: dismiss keyboard/alert/sheet blocking overlay.",
        "inputSchema": {
            "type": "object",
            "properties": {"settle_ms": {"type": "integer", "default": 2500}},
        },
    },
    {
        "name": "ligh_cap_dismiss_overlay",
        "description": "Motor: dismiss keyboard/sheet/alert if present.",
        "inputSchema": {
            "type": "object",
            "properties": {"settle_ms": {"type": "integer", "default": 2500}},
        },
    },
    {
        "name": "ligh_ux_status",
        "description": "UX graph: nodes, edges, baselines summary (.ligh/uxgraph.json).",
        "inputSchema": {
            "type": "object",
            "properties": {"workspace": {"type": "string"}},
        },
    },
    {
        "name": "ligh_ux_baseline",
        "description": "UX graph: snapshot current screens as named baseline for regress diff.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "workspace": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
            },
            "required": ["name"],
        },
    },
    {
        "name": "ligh_ux_regress",
        "description": "UX graph: diff current screen vs baseline (structural regress, not pixels).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "baseline": {"type": "string"},
                "workspace": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
            },
            "required": ["baseline"],
        },
    },
    {
        "name": "ligh_ux_explore",
        "description": "UX graph: safe BFS explore — records screens/transitions into uxgraph.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "max_steps": {"type": "integer", "default": 6},
                "max_depth": {"type": "integer", "default": 3},
                "workspace": {"type": "string"},
                "settle_ms": {"type": "integer", "default": 2500},
                "timeout_ms": {"type": "integer", "default": 8000},
            },
        },
    },
    {
        "name": "ligh_cap_explore",
        "description": "Motor explore: reach → swipe probes → reach. probes_tried in evidence on fault.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "label": {"type": "string"},
                "max_probes": {"type": "integer", "default": 4},
                "max_swipes": {"type": "integer", "default": 10},
                "timeout_ms": {"type": "integer", "default": 18000},
            },
        },
    },
    {
        "name": "ligh_ux_hint",
        "description": "UX graph: link screen fingerprint to Swift source file.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "fingerprint": {"type": "string"},
                "source_path": {"type": "string"},
                "workspace": {"type": "string"},
            },
            "required": ["fingerprint", "source_path"],
        },
    },
    {
        "name": "ligh_cap_app_goal",
        "description": "Declarative job: setup steps + postconditions (wait_id). Motor expands reach/scroll/dismiss.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "app": {"type": "string"},
                "bundle_id": {"type": "string"},
                "setup": {"type": "array", "items": {"type": "object"}, "default": []},
                "postconditions": {"type": "array", "items": {"type": "object"}},
                "settle_ms": {"type": "integer", "default": 3500},
                "timeout_ms": {"type": "integer", "default": 15000},
                "no_install": {"type": "boolean", "default": False},
            },
            "required": ["postconditions"],
        },
    },
]


def session_udid() -> str | None:
    st = ligh("--json", "status")
    if not st.get("ok"):
        return None
    data = st.get("data") or {}
    if isinstance(data, dict):
        sess = data.get("session") or {}
        return sess.get("udid") or (data.get("device") or {}).get("udid")
    return None


def call_tool(name: str, args: dict[str, Any]) -> dict[str, Any]:
    if name == "ligh_agent_rules":
        return {"ok": True, "rules": agent_rules_text()}
    if name == "ligh_status":
        return ligh("--json", "status")
    if name == "ligh_up":
        device = str(args.get("device") or "iphone-15-pro")
        # Ensure daemon
        daemon = ligh("daemon", "status")
        if not daemon.get("ok"):
            started = ligh("daemon", "start")
            if not started.get("ok"):
                return started
        return ligh("up", "--device", device, timeout=180)
    if name == "ligh_ready":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        homes = str(args.get("recover_homes") if args.get("recover_homes") is not None else 6)
        return ligh("--json", "ready", "--settle-ms", ms, "--recover-homes", homes)
    if name == "ligh_cap_open_settings":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        return ligh("--json", "cap", "open-settings", "--settle-ms", ms, timeout=180)
    if name == "ligh_cap_settings_search":
        q = str(args.get("query") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        return ligh("--json", "cap", "settings-search", q, "--settle-ms", ms, timeout=180)
    if name == "ligh_cap_assert_surface":
        surf = str(args.get("surface") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        return ligh("--json", "cap", "assert-surface", surf, "--settle-ms", ms)
    if name == "ligh_cap_tap":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        cmd = ["--json", "cap", "tap", "--settle-ms", ms]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        elif args.get("id"):
            cmd += ["--id", str(args["id"])]
        else:
            return {"ok": False, "fault": "target_missing", "error": "need label or id"}
        return ligh(*cmd)
    if name == "ligh_cap_type":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        return ligh(
            "--json", "cap", "type", "--text", str(args.get("text") or ""), "--settle-ms", ms
        )
    if name == "ligh_cap_run_app":
        app = str(args.get("app") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 3500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 8000)
        cmd = ["--json", "cap", "run-app", app, "--settle-ms", ms, "--timeout-ms", to]
        if args.get("bundle_id"):
            cmd += ["--bundle-id", str(args["bundle_id"])]
        if args.get("wait_label"):
            cmd += ["--wait-label", str(args["wait_label"])]
        return ligh(*cmd, timeout=180)
    if name == "ligh_cap_autopilot":
        goal_id = str(args.get("goal_id") or "")
        goal_label = str(args.get("goal_label") or "")
        if not goal_id and not goal_label:
            return {"ok": False, "fault": "infra", "error": "need goal_id or goal_label"}
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 1500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 8000)
        st = str(args.get("max_steps") if args.get("max_steps") is not None else 24)
        cmd = [
            "--json", "cap", "autopilot",
            "--max-steps", st, "--settle-ms", ms, "--timeout-ms", to,
        ]
        if goal_id:
            cmd += ["--goal-id", goal_id]
        if goal_label:
            cmd += ["--goal-label", goal_label]
        for p in args.get("params") or []:
            if isinstance(p, dict):
                val = str(p.get("value") or "")
                cmd += ["--param", f"secure:{val}" if p.get("secure") else val]
            else:
                cmd += ["--param", str(p)]
        if args.get("app"):
            cmd += ["--app", str(args["app"])]
        if args.get("bundle_id"):
            cmd += ["--bundle-id", str(args["bundle_id"])]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_autopilot(ligh(*cmd, timeout=420))
    if name == "ligh_cap_wait_label":
        lab = str(args.get("label") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 8000)
        return ligh(
            "--json", "cap", "wait-label", lab, "--settle-ms", ms, "--timeout-ms", to, timeout=120
        )
    if name == "ligh_cap_app_job":
        app = str(args.get("app") or "")
        steps = args.get("steps")
        if not isinstance(steps, list):
            return {"ok": False, "fault": "infra", "error": "steps must be a JSON array"}
        import json as _json
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 3500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 12000)
        cmd = [
            "--json", "cap", "app-job", app,
            "--steps", _json.dumps(steps),
            "--settle-ms", ms, "--timeout-ms", to,
        ]
        if args.get("bundle_id"):
            cmd += ["--bundle-id", str(args["bundle_id"])]
        if args.get("no_install"):
            cmd += ["--no-install"]
        for la in args.get("launch_args") or []:
            cmd += [f"--launch-arg={la}"]
        return compact_cap(ligh(*cmd, timeout=300))
    if name == "ligh_perceive":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        cmd = ["--json", "cap", "perceive", "--settle-ms", ms]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_qa_cap(ligh(*cmd))
    if name == "ligh_perceive_routed":
        ms = int(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        vf = args.get("vision_fallback")
        vision_fallback = True if vf is None else bool(vf)
        homes = int(args.get("recover_homes") if args.get("recover_homes") is not None else 4)
        ws = str(args["workspace"]) if args.get("workspace") else None
        return route_perceive(ms, vision_fallback=vision_fallback, recover_homes=homes, workspace=ws)
    if name == "ligh_attempt":
        import json as _json
        intent = str(args.get("intent") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 8000)
        cmd = ["--json", "cap", "attempt", intent, "--settle-ms", ms, "--timeout-ms", to]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        if args.get("id"):
            cmd += ["--id", str(args["id"])]
        if args.get("text"):
            cmd += ["--text", str(args["text"])]
        if args.get("key"):
            cmd += ["--key", str(args["key"])]
        if args.get("expect"):
            cmd += ["--expect", _json.dumps(args["expect"])]
        return compact_qa_cap(ligh(*cmd, timeout=120))
    if name == "ligh_find":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 12000)
        sw = str(args.get("max_swipes") if args.get("max_swipes") is not None else 8)
        scroll = args.get("scroll")
        scroll_flag = "true" if scroll is None or scroll else "false"
        cmd = ["--json", "cap", "find", "--settle-ms", ms, "--timeout-ms", to, "--max-swipes", sw]
        if scroll is not None:
            cmd += ["--scroll", scroll_flag]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        if args.get("id"):
            cmd += ["--id", str(args["id"])]
        if not args.get("label") and not args.get("id"):
            return {"ok": False, "fault": "target_missing", "error": "need label or id"}
        return compact_qa_cap(ligh(*cmd, timeout=120))
    if name == "ligh_dismiss":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        return compact_qa_cap(ligh("--json", "cap", "dismiss", "--settle-ms", ms))
    if name == "ligh_cap_reach":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 12000)
        sw = str(args.get("max_swipes") if args.get("max_swipes") is not None else 12)
        cmd = ["--json", "cap", "reach", "--settle-ms", ms, "--timeout-ms", to, "--max-swipes", sw]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        elif args.get("id"):
            cmd += ["--id", str(args["id"])]
        else:
            return {"ok": False, "fault": "target_missing", "error": "need id or label"}
        return compact_cap(ligh(*cmd, timeout=180))
    if name == "ligh_cap_dismiss_overlay":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        return compact_cap(ligh("--json", "cap", "dismiss-overlay", "--settle-ms", ms))
    if name == "ligh_ux_status":
        cmd = ["--json", "uxgraph", "status"]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_qa_cap(ligh(*cmd))
    if name == "ligh_ux_baseline":
        name_b = str(args.get("name") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        cmd = ["--json", "uxgraph", "baseline", name_b, "--settle-ms", ms]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_qa_cap(ligh(*cmd, timeout=120))
    if name == "ligh_ux_regress":
        base = str(args.get("baseline") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        cmd = ["--json", "uxgraph", "regress", base, "--settle-ms", ms]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_qa_cap(ligh(*cmd, timeout=120))
    if name == "ligh_ux_explore":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 8000)
        st = str(args.get("max_steps") if args.get("max_steps") is not None else 6)
        dp = str(args.get("max_depth") if args.get("max_depth") is not None else 3)
        cmd = ["--json", "uxgraph", "explore", "--settle-ms", ms, "--timeout-ms", to, "--max-steps", st, "--max-depth", dp]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_qa_cap(ligh(*cmd, timeout=180))
    if name == "ligh_cap_explore":
        sw = str(args.get("max_swipes") if args.get("max_swipes") is not None else 10)
        pr = str(args.get("max_probes") if args.get("max_probes") is not None else 4)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 18000)
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 3500)
        cmd = ["--json", "cap", "explore", "--max-swipes", sw, "--max-probes", pr, "--settle-ms", ms, "--timeout-ms", to]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        elif args.get("id"):
            cmd += ["--id", str(args["id"])]
        else:
            return {"ok": False, "fault": "target_missing", "error": "need id or label"}
        out = compact_cap(ligh(*cmd, timeout=240))
        det = out.get("detail") or {}
        if isinstance(det, dict) and det.get("probes_tried"):
            out.setdefault("evidence", {})["probes_tried"] = det["probes_tried"]
        return out
    if name == "ligh_ux_hint":
        fp = str(args.get("fingerprint") or "")
        sp = str(args.get("source_path") or "")
        cmd = ["--json", "uxgraph", "hint", fp, sp]
        if args.get("workspace"):
            cmd += ["--workspace", str(args["workspace"])]
        return compact_qa_cap(ligh(*cmd))
    if name == "ligh_cap_app_goal":
        import json as _json
        post = args.get("postconditions")
        if not isinstance(post, list):
            return {"ok": False, "fault": "infra", "error": "postconditions must be array"}
        setup = args.get("setup") if isinstance(args.get("setup"), list) else []
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 3500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 15000)
        cmd = [
            "--json", "cap", "app-goal",
            "--setup", _json.dumps(setup),
            "--postconditions", _json.dumps(post),
            "--settle-ms", ms, "--timeout-ms", to,
        ]
        if args.get("app"):
            cmd += ["--app", str(args["app"])]
        if args.get("bundle_id"):
            cmd += ["--bundle-id", str(args["bundle_id"])]
        if args.get("no_install"):
            cmd += ["--no-install"]
        return compact_cap(ligh(*cmd, timeout=300))
    if name == "ligh_observe":
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        raw = ligh("--json", "observe", "--settle-ms", ms)
        if not raw.get("ok"):
            return {
                "ok": False,
                "eyes_unusable": True,
                "error": raw.get("error"),
                "suggestion": "ligh_up / ligh_home, then observe again",
            }
        data = raw.get("data")
        if not isinstance(data, dict):
            return {"ok": False, "eyes_unusable": True, "error": "observe returned non-object"}
        return compact_eyes(data)
    if name == "ligh_tap":
        if args.get("label"):
            return ligh("--json", "tap", "--label", str(args["label"]), "--timeout-ms", "7000")
        if args.get("id"):
            return ligh("--json", "tap", "--id", str(args["id"]), "--timeout-ms", "5000")
        if "x" in args and "y" in args:
            return ligh("--json", "tap", "--x", str(args["x"]), "--y", str(args["y"]))
        return {"ok": False, "error": "need label, id, or x/y"}
    if name == "ligh_type":
        return ligh("--json", "type", "--text", str(args.get("text") or ""))
    if name == "ligh_home":
        return ligh("home")
    if name == "ligh_wait":
        cmd = ["--json", "wait", "--timeout-ms", str(args.get("timeout_ms") or 8000)]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        elif args.get("id"):
            cmd += ["--id", str(args["id"])]
        else:
            return {"ok": False, "error": "need label or id"}
        return ligh(*cmd)
    if name == "ligh_key":
        key = str(args.get("name") or "return")
        return ligh("--json", "key", "--name", key)
    if name == "ligh_swipe":
        fx = str(args.get("from_x", 0.5))
        fy = str(args.get("from_y", 0.8))
        tx = str(args.get("to_x", 0.5))
        ty = str(args.get("to_y", 0.2))
        return ligh("--json", "swipe", "--from-x", fx, "--from-y", fy, "--to-x", tx, "--to-y", ty)
    if name == "ligh_scroll_until":
        sw = str(args.get("max_swipes") if args.get("max_swipes") is not None else 8)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 12000)
        cmd = ["--json", "scroll-until", "--max-swipes", sw, "--timeout-ms", to]
        if args.get("label"):
            cmd += ["--label", str(args["label"])]
        elif args.get("id"):
            cmd += ["--id", str(args["id"])]
        else:
            return {"ok": False, "error": "need label or id"}
        return compact_cap(ligh(*cmd, timeout=180))
    if name == "ligh_run":
        app = str(args.get("app") or "")
        if not app:
            return {"ok": False, "error": "app path required"}
        cmd = ["--json", "run", app]
        if args.get("bundle_id"):
            cmd += ["--bundle-id", str(args["bundle_id"])]
        return ligh(*cmd, timeout=180)
    if name == "ligh_launch":
        bid = str(args.get("bundle_id") or "")
        if not bid:
            return {"ok": False, "error": "bundle_id required"}
        udid = session_udid()
        if not udid:
            return {"ok": False, "error": "no booted session — call ligh_up first"}
        p = subprocess.run(
            ["xcrun", "simctl", "launch", udid, bid],
            capture_output=True,
            text=True,
            timeout=60,
        )
        if p.returncode != 0:
            return {"ok": False, "error": (p.stderr or p.stdout or "launch failed").strip()}
        return {"ok": True, "bundle_id": bid, "stdout": (p.stdout or "").strip()}
    if name == "ligh_relaunch":
        return ligh("--json", "relaunch")
    if name == "ligh_sense":
        return ligh("--json", "sense")
    if name == "ligh_screenshot":
        path = args.get("path") or os.path.expanduser("~/.ligh/mcp-screenshot.png")
        return ligh("--json", "screenshot", "-o", str(path))
    return {"ok": False, "error": f"unknown tool {name}"}


def reply(msg_id: Any, result: Any) -> None:
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg_id, "result": result}) + "\n")
    sys.stdout.flush()


def reply_err(msg_id: Any, code: int, message: str) -> None:
    sys.stdout.write(
        json.dumps(
            {"jsonrpc": "2.0", "id": msg_id, "error": {"code": code, "message": message}}
        )
        + "\n"
    )
    sys.stdout.flush()


def main() -> int:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = req.get("method")
        msg_id = req.get("id")
        params = req.get("params") or {}

        if method == "initialize":
            reply(
                msg_id,
                {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "ligh", "version": "0.4.0"},
                },
            )
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            reply(msg_id, {"tools": TOOLS})
        elif method == "tools/call":
            name = params.get("name")
            args = params.get("arguments") or {}
            try:
                result = call_tool(name, args)
            except Exception as e:
                result = {"ok": False, "error": str(e)}
            # compact_eyes returns ok without nested ok from ligh — treat eyes_unusable as soft ok
            is_err = False
            if isinstance(result, dict):
                if result.get("ok") is False:
                    is_err = True
            reply(
                msg_id,
                {
                    "content": [
                        {"type": "text", "text": json.dumps(result, ensure_ascii=False)}
                    ],
                    "isError": is_err,
                },
            )
        elif method == "ping":
            reply(msg_id, {})
        else:
            if msg_id is not None:
                reply_err(msg_id, -32601, f"method not found: {method}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
