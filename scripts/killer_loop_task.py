#!/usr/bin/env python3
"""Load frozen killer-loop task definitions (agent sees task.json only, not ground-truth)."""

from __future__ import annotations

import json
import os
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def load_task(task_path: str | None = None) -> dict[str, Any]:
    path = task_path or os.environ.get(
        "LIGH_KILLER_TASK",
        os.path.join(ROOT, "fixtures/frozen/tasks/onboarding-home-broken/task.json"),
    )
    if not os.path.isabs(path):
        path = os.path.join(ROOT, path)
    task = json.load(open(path, encoding="utf-8"))
    task["_task_path"] = path
    task["_root"] = ROOT
    for key in ("source_root", "app_path", "build_script", "bug_patch"):
        if key in task and task[key] and not os.path.isabs(task[key]):
            task[key] = os.path.join(ROOT, task[key])
    return task


def list_swift_sources(source_root: str) -> list[str]:
    out: list[str] = []
    for dirpath, _, files in os.walk(source_root):
        for f in files:
            if f.endswith(".swift"):
                out.append(os.path.relpath(os.path.join(dirpath, f), ROOT))
    return sorted(out)


def safe_source_path(task: dict[str, Any], rel: str) -> str:
    rel = (rel or "").strip()
    root = task["source_root"]
    if rel.startswith("/"):
        p = os.path.normpath(rel)
    else:
        p = os.path.normpath(os.path.join(ROOT, rel))
    if not p.startswith(root):
        base = os.path.basename(rel)
        for r in list_swift_sources(root):
            if r.endswith("/" + base):
                return os.path.normpath(os.path.join(ROOT, r))
        raise ValueError(f"path must be under {root}: {rel}")
    return p
