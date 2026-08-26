#!/usr/bin/env python3
"""TRAIL repair job — trace prove + hybrid localize + repair bundle.

No golden diff. No frozen_fast. No per-app templates.
Fixer (R4) + trace certify (R5) run in-daemon via `cap_repair_job` (next).

Usage:
  LIGH_REPAIR_JOB_TASK=fixtures/frozen/tasks/kix-notes-tab-missing/task.json \\
    ./scripts/gate-trail.sh
"""

from __future__ import annotations

import json
import os
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from identity_index import build_identity_index  # noqa: E402
from killer_loop_task import load_task  # noqa: E402
from killer_loop_verify import (  # noqa: E402
    bootstrap_app,
    eval_spec,
    perceive,
    run_steps,
    verification_markers,
)
from ligh_mcp import call_tool  # noqa: E402
from view_graph import hybrid_localize  # noqa: E402

WALL_MS = int(os.environ.get("LIGH_TRAIL_WALL_MS", "120000"))
PROVE_BUDGET_MS = int(os.environ.get("LIGH_TRAIL_PROVE_MS", "45000"))
OUT = os.environ.get(
    "LIGH_TRAIL_OUT",
    os.environ.get(
        "LIGH_REPAIR_JOB_OUT",
        os.path.join(ROOT, "docs/assets/trail-latest.json"),
    ),
)
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target/release/ligh"))


def _now_ms() -> int:
    return int(time.time() * 1000)


def _elapsed_ms(t0_ms: int) -> int:
    return _now_ms() - t0_ms


def ensure_ligh_session(*, retries: int = 3) -> dict[str, Any]:
    import subprocess

    last: dict[str, Any] = {"ok": False}
    for _ in range(retries):
        last = call_tool("ligh_ready", {"settle_ms": 1200, "recover_homes": 2})
        if last.get("ok") or last.get("ax_quality") == "ready":
            return {"ok": True, "ready": last}
        subprocess.run([LIGH, "daemon", "stop", "--json"], capture_output=True, timeout=15)
        time.sleep(0.8)
        subprocess.run([LIGH, "daemon", "start"], capture_output=True, timeout=15)
        subprocess.run(
            [LIGH, "up", "--device", os.environ.get("LIGH_DEVICE", "iphone-15-pro")],
            capture_output=True,
            timeout=30,
        )
        time.sleep(0.5)
    return {"ok": False, "last": last}


def trace_prove(task: dict[str, Any]) -> dict[str, Any]:
    """Exercise harness until motor fail; emit TraceFailure."""
    app = os.environ.get("LIGH_APP_PATH", task["app_path"])
    bundle_id = task["bundle_id"]
    ver = task.get("verification") or {}
    wait_label = task.get("bootstrap_wait_label") or "Welcome Back"
    markers = verification_markers(task)

    if not ensure_ligh_session().get("ok"):
        return {"ok": False, "phase": "session"}

    boot = bootstrap_app(
        app, bundle_id, wait_label=wait_label, app_markers=markers, task=task
    )
    if not boot.get("foreground_ok"):
        return {"ok": False, "phase": "bootstrap", "evidence": boot}

    setup = run_steps(ver.get("initial_setup") or [], "setup")
    if setup and not all(s.get("ok") for s in setup):
        return {"ok": False, "phase": "setup", "trace": setup}

    pre_keys = perceive(2500)["keys"]
    pre = eval_spec(ver.get("preconditions") or {}, pre_keys)
    if not pre.get("ok"):
        return {"ok": False, "phase": "precondition", "evidence": pre}

    scene_before = _scene_digest()
    exercise = run_steps(ver.get("exercise") or [], "exercise")
    if exercise and all(s.get("ok") for s in exercise):
        return {"ok": True, "phase": "exercise", "trace": exercise, "unexpected_pass": True}

    fail_idx = next((i for i, s in enumerate(exercise) if not s.get("ok")), None)
    fail_step = exercise[fail_idx] if fail_idx is not None else {}
    keys = perceive(1500)["keys"]
    observed = sorted(k for k in keys if k)
    expected = str(fail_step.get("id") or fail_step.get("label") or "")
    fault = fail_step.get("fault") or fail_step.get("error") or "exercise_failed"
    trace_failure = {
        "step": (fail_idx or 0) + 1,
        "action": fail_step.get("action") or "unknown",
        "expected_identity": expected,
        "observed_identities": observed[:24],
        "fault": str(fault),
        "scene_before": scene_before,
        "scene_after": _scene_digest(),
        "label": fail_step.get("label"),
    }
    return {
        "ok": False,
        "phase": "exercise",
        "trace_failure": trace_failure,
        "trace": exercise,
        "fail_step": fail_step,
    }


def _scene_digest() -> str | None:
    obs = call_tool("ligh_observe", {"settle_ms": 800})
    scene = obs.get("scene") or {}
    regions = scene.get("regions") or []
    if not regions:
        return scene.get("surface")
    kinds = "|".join(sorted({r.get("kind") or "flat" for r in regions[:4]}))
    return f"{scene.get('surface') or 'app'}:{kinds}"


def repair_mode_from_trace(fault: str, expected_identity: str) -> str:
    ident = (expected_identity or "").lower()
    tabish = ident.startswith("tab_") or ident in ("notes", "favorites", "home", "cart")
    if fault in (
        "target_missing",
        "target_never_visible",
        "exercise_failed",
        "motor_failed",
        "motor_no_effect",
    ) and tabish:
        return "tab_chrome_missing"
    if fault in ("motor_no_effect", "control_fired_no_transition"):
        return "state_gate_stuck"
    if fault == "blocked":
        return "blocked_overlay"
    if fault == "type_never_committed":
        return "type_never_committed"
    if fault == "motor_rejected":
        return "motor_rejected"
    return "unknown"


def fix_plan(mode: str, trace_failure: dict[str, Any]) -> str:
    ident = trace_failure.get("expected_identity") or "target"
    plans = {
        "tab_chrome_missing": (
            f"Restore the missing tab/navigation item for '{ident}' in the TabView "
            "composition file (not Auth/Login). Preserve existing tabs and identifiers."
        ),
        "state_gate_stuck": (
            f"Fix the state gate so '{ident}' transitions to the next screen — align "
            "handler state with router/navigation."
        ),
        "blocked_overlay": (
            f"Fix onboarding/overlay finish so '{ident}' dismisses overlay and reveals home."
        ),
    }
    return plans.get(
        mode,
        f"Minimal fix so exercise step for '{ident}' succeeds without breaking other flows.",
    )


def read_snippet(path: str, line: int | None, *, radius: int = 40) -> dict[str, Any]:
    """Return a bounded file slice for the constrained fixer."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.readlines()
    except OSError as e:
        return {"ok": False, "error": str(e), "path": path}

    total = len(lines)
    if total == 0:
        return {"ok": True, "path": path, "start_line": 1, "end_line": 1, "content": ""}

    anchor = max(1, min(line or 1, total))
    start = max(1, anchor - radius)
    end = min(total, anchor + radius)
    content = "".join(f"{i}|{lines[i-1]}" for i in range(start, end + 1))
    return {
        "ok": True,
        "path": path,
        "start_line": start,
        "end_line": end,
        "anchor_line": anchor,
        "content": content,
    }


def build_fixer_prompt(bundle: dict[str, Any], snippet: dict[str, Any]) -> str:
    """Single-shot R4 prompt: one file, one scoped edit, no repo walk."""
    tf = bundle.get("trace_failure") or {}
    scope = bundle.get("scope") or {}
    loc = bundle.get("localization") or {}
    comp = loc.get("composition") or {}
    ascent = comp.get("ascent") or "none"
    return "\n".join(
        [
            "You are the constrained TRAIL fixer.",
            f"Mode: {bundle.get('mode')}",
            f"Fix plan: {bundle.get('fix_plan')}",
            f"Oracle trace: {bundle.get('oracle_trace')}",
            f"Expected identity: {tf.get('expected_identity')}",
            f"Observed identities after failure: {', '.join(tf.get('observed_identities') or [])}",
            f"Target file only: {scope.get('primary_path')}",
            f"Allowed globs: {scope.get('edit_globs')}",
            f"Forbidden globs: {scope.get('forbidden_globs')}",
            f"Localization ascent: {ascent}",
            "Requirements:",
            "- Edit only the target file.",
            "- Do not touch Auth/Login for tab_chrome_missing.",
            "- Preserve existing tabs and identifiers.",
            "- Return the full updated file contents only.",
            "- If the file already satisfies the plan, return the original file unchanged.",
            "",
            f"Target slice `{snippet.get('path')}:{snippet.get('start_line')}-{snippet.get('end_line')}`:",
            snippet.get("content") or "",
        ]
    )


def build_repair_bundle(
    task: dict[str, Any],
    trace_failure: dict[str, Any],
    localization: dict[str, Any],
) -> dict[str, Any]:
    mode = repair_mode_from_trace(
        str(trace_failure.get("fault") or ""),
        str(trace_failure.get("expected_identity") or ""),
    )
    primary = localization.get("primary_path")
    source_root = task["source_root"]
    repo_primary = (
        os.path.join(os.path.relpath(source_root, ROOT), primary).replace("\\", "/")
        if primary
        else None
    )
    forbidden = ["**/Auth/**"] if mode == "tab_chrome_missing" else []
    edit_globs = (
        ["**/Navigation/**", "**/*TabView*.swift"]
        if mode == "tab_chrome_missing"
        else ["**/*.swift"]
    )
    plan = fix_plan(mode, trace_failure)
    primary_line = (
        ((localization.get("composition") or {}).get("line"))
        or ((localization.get("sites") or [{}])[0].get("line"))
        or 1
    )
    primary_abs = os.path.join(source_root, primary) if primary else None
    snippet = (
        read_snippet(primary_abs, int(primary_line), radius=40)
        if primary_abs
        else {"ok": False, "error": "missing primary path"}
    )
    scope = {
        "edit_globs": edit_globs,
        "forbidden_globs": forbidden,
        "primary_path": repo_primary,
        "edit_intent": plan,
    }
    bundle = {
        "schema": 1,
        "architecture": "trail",
        "mode": mode,
        "trace_failure": trace_failure,
        "oracle_trace": (
            f"step {trace_failure.get('step')} {trace_failure.get('action')} "
            f"expected '{trace_failure.get('expected_identity')}'"
        ),
        "fix_plan": plan,
        "scope": scope,
        "localization": localization,
        "max_patch_candidates": 2,
        "next_phase": "constrained_fixer_shot_1",
        "fixer_input": {
            "target_file": repo_primary,
            "target_line": primary_line,
            "snippet": snippet,
        },
    }
    bundle["fixer_input"]["prompt"] = build_fixer_prompt(bundle, snippet)
    return bundle


def main() -> int:
    t0_ms = int(os.environ.get("LIGH_TRAIL_T0_MS") or _now_ms())
    task_path = os.environ.get(
        "LIGH_REPAIR_JOB_TASK",
        "fixtures/frozen/tasks/kix-notes-tab-missing/task.json",
    )
    if not os.path.isabs(task_path):
        task_path = os.path.join(ROOT, task_path)
    os.environ["LIGH_KILLER_TASK"] = task_path
    task = load_task()

    doc: dict[str, Any] = {
        "gate": "trail",
        "architecture": "trace_repair_autopilot_identity_localization",
        "wall_budget_ms": WALL_MS,
        "prove_budget_ms": PROVE_BUDGET_MS,
        "infra_ms": int(os.environ.get("LIGH_TRAIL_INFRA_MS") or 0),
        "task": task["id"],
        "llm_tokens": 0,
        "phase_trace": [],
        "verified": False,
        "holy_shit": False,
    }

    def mark(phase: str, **extra: Any) -> None:
        doc["phase_trace"].append(
            {"phase": phase, "ms": _elapsed_ms(t0_ms), **extra}
        )

    # Broken tree only — never a healthy BACKUP twin via env oracle.
    index_root = task["source_root"]
    if not os.path.isabs(index_root):
        index_root = os.path.join(ROOT, index_root)
    index = build_identity_index(index_root)
    doc["identity_index_size"] = len(index)
    mark("index_built", size=len(index))

    prove = trace_prove(task)
    mark(
        "trace_exercise",
        ok=prove.get("ok"),
        prove_phase=prove.get("phase"),
    )
    if prove.get("ok"):
        doc.update(
            {
                "reason": "unexpected_pass_on_broken_app",
                "prove": prove,
                "trail_wall_ms": _elapsed_ms(t0_ms),
            }
        )
        _write(doc)
        return 1

    if prove.get("phase") != "exercise" or not prove.get("trace_failure"):
        doc.update(
            {
                "reason": f"prove_failed_{prove.get('phase')}",
                "prove": prove,
                "trail_wall_ms": _elapsed_ms(t0_ms),
            }
        )
        _write(doc)
        return 1

    # R1–R3: always localize on TraceFailure. Prove budget is a metric, not a hard abort —
    # exercise settle times alone can exceed 45s on cold Kix.
    tf = prove["trace_failure"]
    expected = str(tf.get("expected_identity") or "")
    loc = hybrid_localize(index_root, index, expected)
    mark(
        "hybrid_localize",
        primary=loc.get("primary_path"),
        ascent=(loc.get("composition") or {}).get("ascent"),
    )
    if not loc.get("primary_path"):
        doc.update(
            {
                "reason": "identity_not_in_index",
                "trace_failure": tf,
                "localization": loc,
                "trail_wall_ms": _elapsed_ms(t0_ms),
            }
        )
        _write(doc)
        return 1

    bundle = build_repair_bundle(task, tf, loc)
    doc["repair_bundle"] = bundle
    doc["trace_failure"] = tf
    doc["failed_identity"] = expected
    doc["primary_site"] = loc.get("composition") or loc.get("sites", [{}])[0]
    mark("repair_bundle", mode=bundle.get("mode"))

    trail_ms = _elapsed_ms(t0_ms)
    localized_ok = bool(loc.get("primary_path"))
    within_prove = trail_ms <= PROVE_BUDGET_MS
    doc.update(
        {
            "trail_wall_ms": trail_ms,
            "within_prove_budget": within_prove,
            "within_budget": trail_ms <= WALL_MS,
            "localization_ok": localized_ok,
            "reason": "awaiting_fixer_and_certify",
            "verified": False,
            "holy_shit": False,
        }
    )
    _write(doc)
    print(
        json.dumps(
            {
                "gate": "trail",
                "localization_ok": localized_ok,
                "trail_wall_ms": trail_ms,
                "within_prove_budget": within_prove,
                "mode": bundle.get("mode"),
                "primary_path": bundle.get("scope", {}).get("primary_path"),
                "expected_identity": expected,
                "verified": False,
                "next": "cap_repair_job fixer+certify",
            }
        )
    )
    # R1–R3 pass = TraceFailure + composition localized. Prove budget soft until R5.
    return 0 if localized_ok else 1


def _write(doc: dict[str, Any]) -> None:
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    raise SystemExit(main())
