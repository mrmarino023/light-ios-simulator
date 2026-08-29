#!/usr/bin/env python3
"""LIGH Agent Scorepack — external eval contract (not developer paradise).

Buyers: agent platforms, eval labs, CI for agent-authored PRs.
Job: frozen tasks → inject → repair/agent → ok:true only → scored board.

Emits docs/assets/scorepack-latest.json with schema ligh.scorepack.result.v1.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DEFAULT_MANIFEST = os.path.join(ROOT, "scorepack", "v1", "manifest.json")
DEFAULT_OUT = os.path.join(ROOT, "docs", "assets", "scorepack-latest.json")


def load_manifest(path: str) -> dict[str, Any]:
    doc = json.load(open(path, encoding="utf-8"))
    if doc.get("schema") != "ligh.scorepack.v1":
        raise ValueError(f"unsupported scorepack schema: {doc.get('schema')}")
    return doc


def _task_abs(rel: str) -> str:
    return rel if os.path.isabs(rel) else os.path.join(ROOT, rel)


def run_task(task_path: str, *, out_path: str, wall_ms: int, reuse: bool) -> dict[str, Any]:
    env = os.environ.copy()
    env["LIGH_TRAIL_TASK"] = task_path
    env["LIGH_TRAIL_HOLY_OUT"] = out_path
    env["LIGH_TRAIL_WALL_MS"] = str(wall_ms)
    env["LIGH_TRAIL_REUSE_SESSION"] = "1" if reuse else "0"
    gate = os.path.join(ROOT, "scripts", "gate-trail-holy.sh")
    p = subprocess.run(
        [gate],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=max(300, wall_ms // 1000 + 240),
    )
    row: dict[str, Any] = {
        "task_path": task_path,
        "gate_exit": p.returncode,
        "verified": False,
        "holy_shit": False,
    }
    if os.path.isfile(out_path):
        try:
            d = json.load(open(out_path, encoding="utf-8"))
            row.update(
                {
                    "task": d.get("task") or os.path.basename(os.path.dirname(task_path)),
                    "verified": bool(d.get("verified")),
                    "holy_shit": bool(d.get("holy_shit")),
                    "wall_ms": d.get("wall_ms"),
                    "infra_ms": d.get("infra_ms"),
                    "llm_tokens": d.get("llm_tokens"),
                    "mode": d.get("mode"),
                    "primary_path": d.get("primary_path"),
                    "fault": d.get("fault") or d.get("reason"),
                    "reason": d.get("reason"),
                }
            )
        except (OSError, json.JSONDecodeError) as e:
            row["fault"] = f"artifact_unreadable:{e}"
    else:
        # Build/governor OOM often leaves no holy JSON — surface gate log fault.
        tail = ((p.stderr or "") + "\n" + (p.stdout or ""))[-1500:]
        fault = "gate_failed"
        if "infra_oom" in tail:
            fault = "infra_oom"
        elif "Killed: 9" in tail or "killed" in tail.lower():
            fault = "infra_oom"
        row["fault"] = fault
        row["log_tail"] = tail[-800:]
    return row


def score_board(manifest: dict[str, Any], rows: list[dict[str, Any]]) -> dict[str, Any]:
    scoring = manifest.get("scoring") or {}
    core_n = len(manifest.get("core_tasks") or [])
    need = max(1, (core_n * 2 + 2) // 3)  # ceil(2/3)
    verified = sum(1 for r in rows if r.get("verified"))
    holy = sum(1 for r in rows if r.get("holy_shit"))
    return {
        "schema": "ligh.scorepack.result.v1",
        "pack_id": manifest.get("pack_id"),
        "pack_version": manifest.get("version"),
        "buyer": manifest.get("buyer"),
        "wall_budget_ms": scoring.get("wall_budget_ms", 120000),
        "tasks_run": len(rows),
        "tasks_verified": verified,
        "tasks_holy_shit": holy,
        "need_verified": need,
        "claim_pass": verified >= need and core_n > 0,
        "holy_shit_generalized": holy == core_n and core_n > 0,
        "compose_with": manifest.get("compose_with"),
        "tasks": rows,
        "ts": int(time.time()),
    }


def run_scorepack(
    *,
    manifest_path: str = DEFAULT_MANIFEST,
    out_path: str = DEFAULT_OUT,
    dry_run: bool = False,
) -> dict[str, Any]:
    manifest = load_manifest(manifest_path)
    wall = int((manifest.get("scoring") or {}).get("wall_budget_ms") or 120000)
    rows: list[dict[str, Any]] = []
    if dry_run:
        for t in manifest.get("core_tasks") or []:
            rows.append(
                {
                    "task": t.get("id"),
                    "task_path": _task_abs(t["task"]),
                    "verified": False,
                    "holy_shit": False,
                    "fault": "dry_run",
                    "effect_class": t.get("effect_class"),
                    "shape": t.get("shape"),
                }
            )
        board = score_board(manifest, rows)
        board["dry_run"] = True
    else:
        reuse = False
        for i, t in enumerate(manifest.get("core_tasks") or []):
            tid = t["id"]
            task_path = _task_abs(t["task"])
            if not os.path.isfile(task_path):
                rows.append(
                    {
                        "task": tid,
                        "verified": False,
                        "holy_shit": False,
                        "fault": "missing_task",
                    }
                )
                continue
            out_t = f"/tmp/ligh-scorepack-{tid}.json"
            print(f"── scorepack task {tid}", flush=True)
            row = run_task(task_path, out_path=out_t, wall_ms=wall, reuse=reuse)
            row["task"] = row.get("task") or tid
            row["effect_class"] = t.get("effect_class")
            row["shape"] = t.get("shape")
            rows.append(row)
            reuse = True  # warm sim after first
        board = score_board(manifest, rows)

    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(board, f, indent=2)
        f.write("\n")
    return board


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="LIGH agent scorepack (eval/CI truth machine)")
    ap.add_argument("--manifest", default=DEFAULT_MANIFEST)
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate manifest + emit board skeleton without Simulator",
    )
    args = ap.parse_args(argv)
    board = run_scorepack(
        manifest_path=args.manifest, out_path=args.out, dry_run=args.dry_run
    )
    print(
        json.dumps(
            {
                "ok": True if args.dry_run else board.get("claim_pass"),
                "dry_run": bool(args.dry_run),
                "claim_pass": board.get("claim_pass"),
                "holy_shit_generalized": board.get("holy_shit_generalized"),
                "verified": f"{board.get('tasks_verified')}/{board.get('tasks_run')}",
                "out": args.out,
            },
            indent=2,
        )
    )
    if args.dry_run:
        return 0
    return 0 if board.get("claim_pass") else 1


if __name__ == "__main__":
    raise SystemExit(main())
