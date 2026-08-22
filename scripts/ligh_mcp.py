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
    """Agent view of a capability result — verified or explicit fault."""
    if not raw.get("ok"):
        return {
            "ok": False,
            "fault": "infra",
            "error": raw.get("error"),
            "suggestion": "ligh_ready then retry app-job",
        }
    data = raw.get("data")
    if not isinstance(data, dict):
        return {"ok": False, "fault": "infra", "error": "cap returned non-object"}
    return {
        "ok": bool(data.get("ok")),
        "fault": data.get("fault") or ("ok" if data.get("ok") else "fail"),
        "capability": data.get("capability"),
        "phase": data.get("phase"),
        "overlay": data.get("overlay"),
        "surface": data.get("surface"),
        "detail": data.get("detail"),
    }


def compact_qa_cap(raw: dict[str, Any]) -> dict[str, Any]:
    """Flatten QA capability (perceive/attempt/find/dismiss) for agents."""
    base = compact_cap(raw)
    data = raw.get("data") if raw.get("ok") else None
    if isinstance(data, dict) and isinstance(data.get("detail"), dict):
        detail = data["detail"]
        if "perceive" in detail:
            base["perceive"] = detail["perceive"]
        if "verdict" in detail:
            v = detail["verdict"]
            base["intent_met"] = v.get("intent_met")
            base["evidence"] = v.get("evidence")
            base["perceive_after"] = v.get("perceive_after")
            if v.get("fault"):
                base["fault"] = v["fault"]
    return base


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
        "description": "QA layer (preferred): settled world model — fingerprint, typed affordances, blocking overlay. One call replaces raw observe parsing.",
        "inputSchema": {
            "type": "object",
            "properties": {"settle_ms": {"type": "integer", "default": 2500}},
        },
    },
    {
        "name": "ligh_attempt",
        "description": "QA layer (preferred): tap|type|key with built-in verify. Returns intent_met + evidence (fingerprints, delta, hypotheses).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "intent": {"type": "string", "enum": ["tap", "type", "key"]},
                "label": {"type": "string"},
                "id": {"type": "string"},
                "text": {"type": "string"},
                "key": {"type": "string"},
                "expect": {
                    "type": "object",
                    "description": "see_id, see_label, surface, fingerprint_changed",
                },
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
        "name": "ligh_dismiss",
        "description": "QA layer: dismiss keyboard/alert/sheet blocking overlay.",
        "inputSchema": {
            "type": "object",
            "properties": {"settle_ms": {"type": "integer", "default": 2500}},
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
        return compact_qa_cap(ligh("--json", "cap", "perceive", "--settle-ms", ms))
    if name == "ligh_attempt":
        import json as _json
        intent = str(args.get("intent") or "")
        ms = str(args.get("settle_ms") if args.get("settle_ms") is not None else 2500)
        to = str(args.get("timeout_ms") if args.get("timeout_ms") is not None else 8000)
        cmd = ["--json", "cap", "attempt", intent, "--settle-ms", ms, "--timeout-ms", to]
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
        cmd = [
            "--json", "cap", "find",
            "--settle-ms", ms, "--timeout-ms", to,
            "--max-swipes", sw,
        ]
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
