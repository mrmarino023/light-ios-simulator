#!/usr/bin/env python3
"""Minimal Maestro MCP for developer A/B vs LIGH (stdio JSON-RPC).

Tools:
  maestro_run_flow — run a Maestro YAML flow, return ok + output tail
  maestro_status   — is maestro CLI available

Cursor mcp.json (see scripts/print-comparison-mcp.sh):
  command: python3
  args: [..., scripts/maestro_mcp.py]
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
MAESTRO = os.environ.get(
    "MAESTRO_BIN", os.path.expanduser("~/.maestro/bin/maestro")
)


def maestro_run(flow: str, timeout: float = 300) -> dict[str, Any]:
    if not os.path.isfile(flow):
        return {"ok": False, "error": f"flow not found: {flow}"}
    if not os.path.isfile(MAESTRO):
        return {"ok": False, "error": f"maestro not found: {MAESTRO}"}
    env = os.environ.copy()
    env.setdefault("MAESTRO_CLI_NO_ANALYTICS", "1")
    env.setdefault("MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED", "1")
    t0 = time.time()
    p = subprocess.run(
        [MAESTRO, "test", flow],
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
    )
    ms = int((time.time() - t0) * 1000)
    out = (p.stdout or "") + (p.stderr or "")
    return {
        "ok": p.returncode == 0,
        "exit_code": p.returncode,
        "ms": ms,
        "output_tail": out[-4000:],
        "flow": flow,
    }


TOOLS = [
    {
        "name": "maestro_status",
        "description": "Check if Maestro CLI is installed.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "maestro_run_flow",
        "description": "Run a Maestro YAML flow on the booted simulator.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "flow": {"type": "string", "description": "Absolute path to .yaml flow"},
            },
            "required": ["flow"],
        },
    },
]


def handle(msg: dict[str, Any]) -> dict[str, Any] | None:
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "maestro-mcp", "version": "0.1.0"},
            },
        }
    if method == "notifications/initialized":
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": mid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = msg.get("params") or {}
        name = params.get("name")
        args = params.get("arguments") or {}
        if name == "maestro_status":
            result = {
                "installed": os.path.isfile(MAESTRO),
                "path": MAESTRO,
            }
        elif name == "maestro_run_flow":
            result = maestro_run(str(args.get("flow") or ""))
        else:
            result = {"ok": False, "error": f"unknown tool: {name}"}
        return {
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "content": [{"type": "text", "text": json.dumps(result, indent=2)}],
            },
        }
    if method == "ping":
        return {"jsonrpc": "2.0", "id": mid, "result": {}}
    if mid is not None:
        return {
            "jsonrpc": "2.0",
            "id": mid,
            "error": {"code": -32601, "message": f"method not found: {method}"},
        }
    return None


def main() -> None:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        resp = handle(msg)
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
