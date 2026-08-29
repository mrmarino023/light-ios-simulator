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


def _goal_needs_discover(goal: dict[str, Any]) -> bool:
    for p in goal.get("postconditions") or []:
        for key in ("wait_id", "wait_label"):
            val = p.get(key)
            if isinstance(val, str) and ("REPLACE" in val or not val.strip()):
                return True
    for s in goal.get("setup") or []:
        for key in ("id", "label"):
            val = s.get(key)
            if isinstance(val, str) and "REPLACE" in val:
                return True
    return False


def ensure_live_goal(workspace: str | None = None) -> dict[str, Any]:
    """If static audit left placeholders, discover live AX labels (Maestro parity)."""
    goal = load_app_goal(workspace)
    if not _goal_needs_discover(goal):
        return goal
    proj = load_project(workspace)
    app = proj.get("app_path")
    bid = proj.get("bundle_id")
    if not app or not bid or not os.path.isdir(app):
        return goal
    from ligh_discover import discover_live, write_discovered_bundle

    disc = discover_live(app, bid, source_root=proj.get("source_root"))
    if disc.get("ok") and disc.get("agent_ready") and disc.get("goal"):
        write_discovered_bundle(ligh_dir(workspace), proj, disc)
        return disc["goal"]
    return goal


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
    """Run verify from .ligh bundle — goal-first by default.

    Always writes `.ligh/last-certify.json`. Refuses TRAIL-facing work when
    process_health says crashed/not running.
    """
    from ligh_session_gate import (
        enrich_certify_from_observe,
        gate_session,
        observe_snapshot,
        trail_allowed,
        write_certify_artifact,
    )

    ws = workspace_root(workspace)
    proj = load_project(workspace)
    app = proj.get("app_path")
    bid = proj.get("bundle_id")
    if not app or not os.path.isdir(app):
        out = {
            "ok": False,
            "fault": "infra",
            "fault_owner": "host",
            "capability": "ligh_test",
            "error": f"app not built: {app!r} — run ligh_init with --build",
            "trail_allowed": False,
            "repair_allowed": False,
        }
        out["certify_path"] = write_certify_artifact(ws, out)
        return out
    if not bid:
        out = {
            "ok": False,
            "fault": "infra",
            "fault_owner": "host",
            "capability": "ligh_test",
            "error": "bundle_id missing in project.json",
            "trail_allowed": False,
            "repair_allowed": False,
        }
        out["certify_path"] = write_certify_artifact(ws, out)
        return out

    # Seed daemon expected_bundle so observe stamps authoritative process_health.
    _ligh_json(
        "cap",
        "run-app",
        str(app),
        "--bundle-id",
        str(bid),
        "--settle-ms",
        str(min(settle_ms, 2500)),
    )

    # Session hard-gate before motor goal (crash ≠ missing chrome).
    blocked = gate_session(bundle_id=str(bid), settle_ms=min(settle_ms, 2000))
    if blocked:
        blocked.update(
            {
                "capability": "ligh_test",
                "mode": mode,
                "workspace": ws,
                "app": app,
                "bundle_id": bid,
            }
        )
        blocked["certify_path"] = write_certify_artifact(ws, blocked)
        return blocked

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
        goal = ensure_live_goal(workspace)
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
    data = raw.get("data") if isinstance(raw.get("data"), dict) else raw
    fault = data.get("fault") if isinstance(data, dict) else None
    if ok:
        fault = "ok"

    # Post observe for certify stamp (system_surface / screen_sig / health).
    snap = observe_snapshot(settle_ms=800)
    ph = snap.get("process_health") if isinstance(snap, dict) else None
    out: dict[str, Any] = {
        "ok": ok,
        "capability": "ligh_test",
        "mode": mode,
        "workspace": ws,
        "app": app,
        "bundle_id": bid,
        "fault": fault,
        "fault_owner": "none" if ok else ("app" if fault not in ("infra", "eyes_unusable", "timeout") else "host"),
        "trail_allowed": bool(ok is False and trail_allowed(str(fault) if fault else None, process_health=ph)),
        "repair_allowed": bool(ok is False and trail_allowed(str(fault) if fault else None, process_health=ph)),
    }
    if isinstance(data, dict):
        for k in ("detail", "evidence", "trace", "overlay"):
            if data.get(k) is not None:
                out[k] = data[k]
    enrich_certify_from_observe(out, snap)
    out["certify_path"] = write_certify_artifact(ws, out)
    return out



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
