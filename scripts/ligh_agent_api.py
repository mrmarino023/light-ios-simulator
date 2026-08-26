#!/usr/bin/env python3
"""Agent-first LIGH API — load .ligh bundles, test, init (shared by MCP + CLI)."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))


def workspace_root(workspace: str | None = None) -> str:
    ws = workspace or os.environ.get("LIGH_WORKSPACE") or os.getcwd()
    return os.path.abspath(ws)


def ligh_dir(workspace: str | None = None) -> str:
    return os.path.join(workspace_root(workspace), ".ligh")


def load_project(workspace: str | None = None) -> dict[str, Any]:
    path = os.path.join(ligh_dir(workspace), "project.json")
    if not os.path.isfile(path):
        raise FileNotFoundError(f"missing {path} — run ligh_init or ./scripts/ligh-paradise.sh")
    return json.load(open(path, encoding="utf-8"))


def load_app_job(workspace: str | None = None) -> list[dict[str, Any]]:
    path = os.path.join(ligh_dir(workspace), "app-job.json")
    if not os.path.isfile(path):
        raise FileNotFoundError(f"missing {path}")
    data = json.load(open(path, encoding="utf-8"))
    if not isinstance(data, list):
        raise ValueError("app-job.json must be a JSON array")
    return data


def load_app_goal(workspace: str | None = None) -> dict[str, Any]:
    path = os.path.join(ligh_dir(workspace), "app-goal.json")
    if os.path.isfile(path):
        data = json.load(open(path, encoding="utf-8"))
        if isinstance(data, dict) and data.get("postconditions"):
            return data
    proj = load_project(workspace)
    ver = proj.get("suggested_verification") or {}
    post = []
    for label in (ver.get("postconditions") or {}).get("must_see_labels") or []:
        post.append({"wait_label": label, "timeout_ms": 12000})
    for ident in _post_ids_from_exercise(ver.get("exercise") or []):
        post.append({"wait_id": ident, "timeout_ms": 12000})
    if not post and proj.get("suggested_app_job"):
        last = proj["suggested_app_job"][-1]
        if last.get("id"):
            post = [{"wait_id": last["id"], "timeout_ms": 12000}]
    setup = []
    bootstrap = ver.get("bootstrap_wait_label")
    if bootstrap:
        setup = [{"op": "wait", "label": bootstrap, "timeout_ms": 12000}]
    return {"setup": setup, "postconditions": post or [{"wait_label": "REPLACE_ME", "timeout_ms": 8000}]}


def _post_ids_from_exercise(exercise: list[dict[str, Any]]) -> list[str]:
    out: list[str] = []
    for step in exercise:
        ident = step.get("id") or step.get("expected_identity")
        if ident and step.get("action") in ("tap", "assert", None):
            out.append(str(ident))
    return out


def _ligh_json(*args: str, timeout: float = 300) -> dict[str, Any]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    blob = (p.stdout or p.stderr or "").strip()
    if blob.startswith("{") or blob.startswith("["):
        try:
            data = json.loads(blob)
            if isinstance(data, dict) and "ok" not in data:
                return {"ok": p.returncode == 0, "data": data}
            return data if isinstance(data, dict) else {"ok": True, "data": data}
        except json.JSONDecodeError:
            pass
    return {"ok": False, "error": blob or f"exit {p.returncode}"}


def run_test(
    workspace: str | None = None,
    *,
    mode: str = "goal",
    settle_ms: int = 3000,
    timeout_ms: int = 20000,
) -> dict[str, Any]:
    """Run verify from .ligh bundle — goal-first by default."""
    proj = load_project(workspace)
    app = proj.get("app_path")
    bid = proj.get("bundle_id")
    if not app or not os.path.isdir(app):
        return {
            "ok": False,
            "fault": "infra",
            "error": f"app not built: {app!r} — run ligh_init with --build",
        }
    if not bid:
        return {"ok": False, "fault": "infra", "error": "bundle_id missing in project.json"}

    if mode == "job":
        steps = load_app_job(workspace)
        cmd = [
            "--json",
            "cap",
            "app-job",
            app,
            "--bundle-id",
            bid,
            "--steps",
            json.dumps(steps),
            "--settle-ms",
            str(settle_ms),
            "--timeout-ms",
            str(timeout_ms),
        ]
        raw = _ligh_json(*cmd)
    else:
        goal = load_app_goal(workspace)
        cmd = [
            "--json",
            "cap",
            "app-goal",
            "--app",
            app,
            "--bundle-id",
            bid,
            "--setup",
            json.dumps(goal.get("setup") or []),
            "--postconditions",
            json.dumps(goal.get("postconditions") or []),
            "--settle-ms",
            str(settle_ms),
            "--timeout-ms",
            str(max(timeout_ms, 15000)),
        ]
        raw = _ligh_json(*cmd, timeout=360)

    ok = bool(raw.get("ok"))
    if not ok and isinstance(raw.get("data"), dict):
        ok = bool(raw["data"].get("ok"))
    out = raw.get("data") if isinstance(raw.get("data"), dict) else raw
    return {
        "ok": ok,
        "capability": "ligh_test",
        "mode": mode,
        "workspace": workspace_root(workspace),
        "app": app,
        "bundle_id": bid,
        **{k: out.get(k) for k in ("fault", "detail", "evidence", "trace") if out.get(k) is not None},
    }


def run_init(
    target: str,
    *,
    build: bool = False,
    workspace: str | None = None,
) -> dict[str, Any]:
    """Detect project, audit AX, write .ligh bundle."""
    script = os.path.join(ROOT, "scripts", "ligh_project.py")
    cmd = [sys.executable, script, target]
    if build:
        cmd.append("--build")
    ws = workspace_root(workspace) if workspace else None
    if ws:
        cmd.extend(["--write", os.path.join(ws, ".ligh")])
    cp = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=900)
    doc: dict[str, Any] = {"ok": cp.returncode == 0}
    try:
        doc = json.loads(cp.stdout)
    except Exception:
        doc["stdout"] = (cp.stdout or "")[-1200:]
        doc["stderr"] = (cp.stderr or "")[-800:]
    doc["capability"] = "ligh_init"
    return doc


def agent_rules_paradise() -> str:
    path = os.path.join(ROOT, "AGENTS.md")
    if os.path.isfile(path):
        return open(path, encoding="utf-8").read()
    return open(os.path.join(ROOT, "docs", "AGENT.md"), encoding="utf-8").read()
