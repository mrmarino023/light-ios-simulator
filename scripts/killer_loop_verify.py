#!/usr/bin/env python3
"""Strict transition-based verifier for killer-loop tasks (protocol v2).

Harness only — not shown to the agent. Uses LIGH MCP for deterministic setup/exercise/post.
Requires app_ready trust gate (no SpringBoard-as-success).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from killer_loop_task import load_task  # noqa: E402
from goal_spec import compile_task_goal, evaluate_goal  # noqa: E402
from ligh_mcp import call_tool  # noqa: E402


def affordance_keys(perceive: dict[str, Any]) -> set[str]:
    keys: set[str] = set()
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        for k in ("id", "label", "identifier", "text"):
            if a.get(k):
                keys.add(str(a[k]))
    return keys


def perceive(settle_ms: int = 2500) -> dict[str, Any]:
    r = call_tool("ligh_perceive", {"settle_ms": settle_ms})
    perceive_doc = r.get("perceive") or {}
    keys = affordance_keys(perceive_doc)
    surface = (perceive_doc.get("scene") or {}).get("surface")
    return {
        "ok": bool(r.get("ok")),
        "keys": keys,
        "surface": surface,
        "perceive": perceive_doc,
        "raw": r,
    }


def is_springboard(
    keys: set[str], surface: str | None, *, app_markers: set[str] | None = None
) -> bool:
    if surface == "springboard":
        return True
    home_markers = {
        "Cerca",
        "Search",
        "Safari",
        "Messaggi",
        "Messages",
        "Fitness",
        "Watch",
        "Contatti",
        "Contacts",
        "File",
        "Files",
    }
    # Task-defined markers avoid coupling trust to one fixture's copy.
    if keys & (app_markers or set()):
        return False
    if len(keys & home_markers) >= 2:
        return True
    # Dense home grid without in-app chrome.
    icon_like = {k for k in keys if k and k[0].isupper() and " " not in k and len(k) < 24}
    return len(icon_like) >= 6


def bootstrap_app(
    app: str,
    bundle_id: str,
    *,
    wait_label: str | None = None,
    app_markers: set[str] | None = None,
) -> dict[str, Any]:
    call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
    payload: dict[str, Any] = {
        "app": app,
        "bundle_id": bundle_id,
        "settle_ms": 3500,
        "timeout_ms": 15000,
    }
    if wait_label:
        payload["wait_label"] = wait_label
    boot = call_tool("ligh_cap_run_app", payload)
    fault = boot.get("fault") or (boot.get("detail") or {}).get("fault")
    if fault and fault not in ("ok", None):
        return {**boot, "foreground_ok": False, "trust_fault": fault}

    for attempt in range(1, 8):
        p = perceive(2500)
        if is_springboard(p["keys"], p.get("surface"), app_markers=app_markers):
            call_tool("ligh_launch", {"bundle_id": bundle_id})
            time.sleep(1.5)
            p = perceive(3000)
        if not is_springboard(p["keys"], p.get("surface"), app_markers=app_markers):
            return {**boot, "foreground_ok": True, "attempt": attempt, "keys": sorted(p["keys"])[:16]}
        app_label = os.path.basename(app).replace(".app", "")
        if app_label in p["keys"]:
            call_tool(
                "ligh_attempt",
                {"intent": "tap", "label": app_label, "settle_ms": 2500, "timeout_ms": 10000},
            )
            time.sleep(1.2)
            continue
    return {**boot, "foreground_ok": False, "trust_fault": "app_not_foreground"}


def run_tap(
    label: str | None = None,
    settle_ms: int = 2500,
    *,
    id: str | None = None,
) -> dict[str, Any]:
    pre = perceive(settle_ms)
    payload: dict[str, Any] = {
        "intent": "tap",
        "settle_ms": settle_ms,
        "timeout_ms": 12000,
    }
    if label:
        payload["label"] = label
    if id:
        payload["id"] = id
    result = call_tool("ligh_attempt", payload)
    fault = result.get("fault") or ""
    target = label or id or ""
    target_seen = target in pre["keys"] if target else False
    # A seen target is not a successful step. The motor must report a verified effect.
    ok = bool(result.get("ok")) and fault in ("", "ok")
    return {
        **result,
        "ok": ok,
        "fault": fault or None,
        "target_seen": target_seen,
    }


def run_type(
    text: str,
    settle_ms: int = 2500,
    *,
    id: str | None = None,
    label: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "intent": "type",
        "text": text,
        "settle_ms": settle_ms,
        "timeout_ms": 12000,
    }
    if id:
        payload["id"] = id
    if label:
        payload["label"] = label
    result = call_tool("ligh_attempt", payload)
    fault = result.get("fault") or ""
    ok = (
        bool(result.get("ok"))
        and fault in ("ok", None, "")
        and result.get("intent_met") is not False
    )
    return {**result, "ok": ok, "fault": fault or None}


def eval_spec(spec: dict[str, Any], keys: set[str]) -> dict[str, Any]:
    must_see = spec.get("must_see_labels") or []
    must_not = spec.get("must_not_see_labels") or []
    seen = {l: (l in keys) for l in must_see}
    absent = {l: (l not in keys) for l in must_not}
    ok = all(seen.values()) and all(absent.values())
    return {
        "ok": ok,
        "must_see": seen,
        "must_not_see": absent,
        "keys_sample": sorted(keys)[:40],
    }


def evaluate_goal_stable(goal: dict[str, Any], settle_ms: int = 3500) -> tuple[dict[str, Any], dict[str, Any]]:
    """Independent temporal verifier for the same GoalSpec used by Autopilot."""
    required = max(2, int(goal.get("stable_observations") or 2))
    deadline = time.monotonic() + max(settle_ms, int(goal.get("stability_window_ms") or 0)) / 1000
    streak = 0
    latest_perceive: dict[str, Any] = {}
    latest_result: dict[str, Any] = {"ok": False}
    while time.monotonic() < deadline:
        observation = perceive(min(settle_ms, 1200))
        latest_perceive = observation
        latest_result = evaluate_goal(goal, observation.get("perceive") or {})
        streak = streak + 1 if latest_result["ok"] else 0
        if streak >= required:
            return {**latest_result, "stable_observations": streak}, observation
        time.sleep(0.05)
    return {**latest_result, "stable_observations": streak}, latest_perceive


def run_steps(steps: list[dict[str, Any]], phase: str) -> list[dict[str, Any]]:
    trace: list[dict[str, Any]] = []
    for i, step in enumerate(steps, start=1):
        action = (step.get("action") or "tap").lower()
        settle = int(step.get("settle_ms") or 2500)
        label = str(step.get("label") or "") or None
        sid = str(step.get("id") or "") or None
        if action == "tap":
            result = run_tap(label, settle, id=sid)
            trace.append(
                {
                    "phase": phase,
                    "step": i,
                    "action": "tap",
                    "label": label,
                    "id": sid,
                    "ok": bool(result.get("ok")),
                    "fault": result.get("fault"),
                }
            )
            if not result.get("ok"):
                break
            continue
        if action == "type":
            text = str(step.get("text") or "")
            result = run_type(text, settle, id=sid, label=label)
            trace.append(
                {
                    "phase": phase,
                    "step": i,
                    "action": "type",
                    "id": sid,
                    "label": label,
                    "text_len": len(text),
                    "ok": bool(result.get("ok")),
                    "fault": result.get("fault"),
                }
            )
            if not result.get("ok"):
                break
            continue
        trace.append({"phase": phase, "step": i, "ok": False, "error": f"unsupported action {action}"})
        break
    return trace


def legacy_weak_pass(task: dict[str, Any], keys: set[str]) -> bool:
    labels = task.get("legacy_weak_success_labels") or task.get("harness_success_labels") or []
    return any(l in keys for l in labels)


def overlay_visible(task: dict[str, Any], keys: set[str]) -> bool:
    overlay = (task.get("verification") or {}).get("onboarding_overlay_labels") or []
    return any(l in keys for l in overlay)


def verification_markers(task: dict[str, Any]) -> set[str]:
    ver = task.get("verification") or {}
    markers: set[str] = set()
    for phase in ("preconditions", "postconditions"):
        spec = ver.get(phase) or {}
        markers.update(str(v) for v in spec.get("must_see_labels") or [])
    if task.get("bootstrap_wait_label"):
        markers.add(str(task["bootstrap_wait_label"]))
    return markers


def goal_visible(task: dict[str, Any], keys: set[str]) -> bool:
    post = ((task.get("verification") or {}).get("postconditions") or {})
    return any(str(label) in keys for label in post.get("must_see_labels") or [])


def strict_verify(task: dict[str, Any] | None = None, *, app: str | None = None, bundle_id: str | None = None) -> dict[str, Any]:
    task = task or load_task()
    app = app or os.environ.get("LIGH_APP_PATH") or task["app_path"]
    bundle_id = bundle_id or task["bundle_id"]
    ver = task.get("verification") or {}
    wait_label = task.get("bootstrap_wait_label") or "Show Onboarding"

    markers = verification_markers(task)
    boot = bootstrap_app(app, bundle_id, wait_label=wait_label, app_markers=markers)
    if not boot.get("foreground_ok"):
        return {
            "verified": False,
            "reason": boot.get("trust_fault") or "app_not_foreground",
            "phase": "bootstrap",
            "evidence": boot,
            "bootstrap": boot,
            "setup_trace": [],
            "exercise_trace": [],
            "legacy_weak_pass": False,
            "false_success": False,
        }

    setup_trace = run_steps(ver.get("initial_setup") or [], "setup")
    if setup_trace and not all(s.get("ok") for s in setup_trace):
        return {
            "verified": False,
            "reason": "setup_failed",
            "phase": "setup",
            "evidence": setup_trace[-1],
            "bootstrap": boot,
            "setup_trace": setup_trace,
            "exercise_trace": [],
            "legacy_weak_pass": False,
            "false_success": False,
        }
    pre_keys = perceive(2500)["keys"]
    pre = eval_spec(ver.get("preconditions") or {}, pre_keys)
    # If a task-declared overlay persisted, allow one clean relaunch recovery.
    overlay_markers = set(ver.get("onboarding_overlay_labels") or [])
    if not pre["ok"] and overlay_markers & pre_keys:
        call_tool("ligh_launch", {"bundle_id": bundle_id})
        time.sleep(1.5)
        boot2 = bootstrap_app(
            app, bundle_id, wait_label=wait_label, app_markers=markers
        )
        if boot2.get("foreground_ok"):
            boot = boot2
            setup_trace = run_steps(ver.get("initial_setup") or [], "setup")
            pre = eval_spec(ver.get("preconditions") or {}, perceive(3000)["keys"])
    if not pre["ok"]:
        return {
            "verified": False,
            "reason": "precondition_not_satisfied",
            "phase": "precondition",
            "evidence": pre,
            "bootstrap": boot,
            "setup_trace": setup_trace,
            "exercise_trace": [],
            "legacy_weak_pass": False,
            "false_success": False,
        }

    exercise_trace = run_steps(ver.get("exercise") or [], "exercise")
    exercise_ok = bool(exercise_trace) and all(s.get("ok") for s in exercise_trace)
    goal_spec = compile_task_goal(task)
    post, post_observation = evaluate_goal_stable(goal_spec, 3500)
    post_keys = post_observation.get("keys") or set()
    weak = legacy_weak_pass(task, post_keys)
    overlay = overlay_visible(task, post_keys)
    home = goal_visible(task, post_keys)

    false_success = weak and (overlay or not post["ok"])
    verified = exercise_ok and post["ok"] and not overlay

    reason = "verified" if verified else "postcondition_not_satisfied"
    phase = "postcondition"
    if not exercise_ok:
        reason = "exercise_failed"
        phase = "exercise"

    return {
        "verified": verified,
        "reason": reason,
        "phase": phase,
        "evidence": {
            "homeTitle": home,
            "onboardingOverlay": overlay,
            "post": post,
            "goal_spec": goal_spec,
            "legacy_weak_pass": weak,
        },
        "bootstrap": boot,
        "setup_trace": setup_trace,
        "exercise_trace": exercise_trace,
        "preconditions": pre,
        "legacy_weak_pass": weak,
        "false_success": false_success,
    }


def establish_initial_state(task: dict[str, Any] | None = None, *, app: str | None = None, bundle_id: str | None = None) -> dict[str, Any]:
    task = task or load_task()
    app = app or os.environ.get("LIGH_APP_PATH") or task["app_path"]
    bundle_id = bundle_id or task["bundle_id"]
    ver = task.get("verification") or {}
    wait_label = task.get("bootstrap_wait_label") or "Show Onboarding"

    markers = verification_markers(task)
    boot = bootstrap_app(app, bundle_id, wait_label=wait_label, app_markers=markers)
    if not boot.get("foreground_ok"):
        return {
            "ok": False,
            "reason": boot.get("trust_fault") or "app_not_foreground",
            "bootstrap": boot,
            "setup_trace": [],
            "preconditions": {},
        }

    setup_trace = run_steps(ver.get("initial_setup") or [], "setup")
    setup_ok = not setup_trace or all(s.get("ok") for s in setup_trace)
    pre = eval_spec(ver.get("preconditions") or {}, perceive(2500)["keys"])
    return {
        "ok": setup_ok and pre["ok"],
        "reason": "initial_state_ready" if setup_ok and pre["ok"] else (
            "setup_failed" if not setup_ok else "initial_state_failed"
        ),
        "phase": None if setup_ok and pre["ok"] else ("setup" if not setup_ok else "precondition"),
        "bootstrap": boot,
        "setup_trace": setup_trace,
        "preconditions": pre,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--task", default=os.environ.get("LIGH_KILLER_TASK"))
    ap.add_argument("--phase", choices=("setup", "strict"), default="strict")
    args = ap.parse_args()
    task = load_task(args.task)
    if args.phase == "setup":
        result = establish_initial_state(task)
    else:
        result = strict_verify(task)
    print(json.dumps(result, indent=2))
    ok = result.get("ok") if args.phase == "setup" else result.get("verified")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
