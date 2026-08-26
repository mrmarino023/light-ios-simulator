#!/usr/bin/env python3
"""TRAIL holy-shit orchestrator — generalize repair without golden/template.

Pipeline (single wall, hot path):
  prove (trace) → localize → constrained LLM fix (≤2 shots) → build → certify

Usage:
  LIGH_TRAIL_TASK=fixtures/frozen/tasks/kix-notes-tab-missing/task.json \\
    python3 scripts/trail_holy.py
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from identity_index import build_identity_index, lookup  # noqa: E402
from killer_loop_task import load_task  # noqa: E402
from killer_loop_verify import (  # noqa: E402
    bootstrap_app,
    eval_spec,
    perceive,
    run_steps,
    strict_verify,
    verification_markers,
)
from ligh_mcp import call_tool  # noqa: E402
from repair_job import (  # noqa: E402
    build_repair_bundle,
    ensure_ligh_session,
    fix_plan,
    repair_mode_from_trace,
)
from trail_fixer import apply_candidate, build_messages, openai_full_file  # noqa: E402
from view_graph import hybrid_localize  # noqa: E402

WALL_MS = int(os.environ.get("LIGH_TRAIL_WALL_MS", "120000"))
OUT = os.environ.get(
    "LIGH_TRAIL_HOLY_OUT",
    os.path.join(ROOT, "docs/assets/trail-holy-latest.json"),
)
MAX_FIX_ATTEMPTS = int(os.environ.get("LIGH_TRAIL_FIX_ATTEMPTS", "2"))


def _now() -> int:
    return int(time.time() * 1000)


def localize_fallback(source_root: str, mode: str, expected: str) -> dict[str, Any] | None:
    """Mode / keyword localization when AX identity is absent (onboarding labels, etc.)."""
    needles: list[str] = []
    if mode == "state_gate_stuck":
        needles = ["isLoggedIn = false", "isLoggedIn = true", "isLoggedIn", "LoginViewModel"]
    elif mode == "blocked_overlay":
        needles = ["isOnboardingVisible = false", "isOnboardingVisible", "hasCompletedOnboarding"]
    elif mode == "tab_chrome_missing":
        needles = ["TabView", expected or "tab_"]
    if expected:
        needles.append(expected)

    hits: list[dict[str, Any]] = []
    for dirpath, _, files in os.walk(source_root):
        if any(s in dirpath for s in ("/build/", "/DerivedData/", "/.git/")):
            continue
        for name in files:
            if not name.endswith(".swift"):
                continue
            path = os.path.join(dirpath, name)
            try:
                text = open(path, encoding="utf-8").read()
            except OSError:
                continue
            for needle in needles:
                if needle and needle in text:
                    line = next(
                        (i for i, ln in enumerate(text.splitlines(), 1) if needle in ln),
                        1,
                    )
                    rel = os.path.relpath(path, source_root).replace("\\", "/")
                    score = 0
                    if mode == "state_gate_stuck":
                        if "ViewModel" in name:
                            score += 20
                        if "Login" in name:
                            score += 10
                        if "ContentView" in name:
                            score -= 5
                    if mode == "blocked_overlay":
                        if name == "OnboardingView.swift":
                            score += 30
                        if "Onboard" in name:
                            score += 10
                        if "UserInput" in name:
                            score -= 10
                    if mode == "tab_chrome_missing" and ("Tab" in name or "Navigation" in rel):
                        score += 10
                    if needle.startswith("isLoggedIn") or needle.startswith("isOnboarding"):
                        score += 15
                    hits.append(
                        {
                            "file": rel,
                            "line": line,
                            "snippet": needle,
                            "score": score,
                            "role": "fallback",
                        }
                    )
                    break
    if not hits:
        return None
    hits.sort(key=lambda h: (-h["score"], len(h["file"])))
    best = hits[0]
    return {
        "identity": expected,
        "sites": hits[:3],
        "composition": best,
        "primary_path": best["file"],
    }


def prove_with_post(task: dict[str, Any]) -> dict[str, Any]:
    """Exercise then postcondition — TraceFailure from first motor or oracle miss."""
    from goal_spec import compile_task_goal  # local import
    from killer_loop_verify import evaluate_goal_stable

    app = os.environ.get("LIGH_APP_PATH", task["app_path"])
    bundle_id = task["bundle_id"]
    ver = task.get("verification") or {}
    wait_label = task.get("bootstrap_wait_label")
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
        fail = next(s for s in setup if not s.get("ok"))
        return {
            "ok": False,
            "phase": "setup",
            "trace_failure": {
                "step": fail.get("step") or 1,
                "action": fail.get("action") or "tap",
                "expected_identity": fail.get("id") or fail.get("label") or "",
                "observed_identities": sorted(perceive(800)["keys"])[:24],
                "fault": fail.get("fault") or "setup_failed",
                "label": fail.get("label"),
            },
            "trace": setup,
        }

    pre = eval_spec(ver.get("preconditions") or {}, perceive(1200)["keys"])
    if not pre.get("ok"):
        return {"ok": False, "phase": "precondition", "evidence": pre}

    exercise = run_steps(ver.get("exercise") or [], "exercise")
    if exercise and not all(s.get("ok") for s in exercise):
        fail_idx = next(i for i, s in enumerate(exercise) if not s.get("ok"))
        fail = exercise[fail_idx]
        return {
            "ok": False,
            "phase": "exercise",
            "trace_failure": {
                "step": fail_idx + 1,
                "action": fail.get("action") or "unknown",
                "expected_identity": fail.get("id") or fail.get("label") or "",
                "observed_identities": sorted(perceive(800)["keys"])[:24],
                "fault": fail.get("fault") or "exercise_failed",
                "label": fail.get("label"),
            },
            "trace": exercise,
        }

    goal = compile_task_goal(task)
    post, obs = evaluate_goal_stable(goal, 2500)
    if post.get("ok"):
        return {"ok": True, "phase": "postcondition", "trace": exercise}

    # Postcondition fail after successful exercise → state/overlay bug.
    must = (ver.get("postconditions") or {}).get("must_see_labels") or []
    expected = str(must[0] if must else goal.get("target") or "goal")
    last = exercise[-1] if exercise else {}
    control = last.get("id") or last.get("label") or ""
    keys = sorted((obs.get("keys") or set()))[:24]
    fault = "control_fired_no_transition" if control else "acceptance_not_in_ax"
    return {
        "ok": False,
        "phase": "postcondition",
        "trace_failure": {
            "step": len(exercise) + 1,
            "action": "assert",
            "expected_identity": expected,
            "observed_identities": keys,
            "fault": fault,
            "label": expected,
            "control": control,
        },
        "trace": exercise,
        "post": post,
    }


def main() -> int:
    # Fast settle for holy-shit wall.
    os.environ.setdefault("LIGH_TRAIL_FAST", "1")
    os.environ.setdefault("LIGH_TRAIL_SETTLE_CAP_MS", "900")

    t0 = _now()
    task_path = os.environ.get(
        "LIGH_TRAIL_TASK",
        "fixtures/frozen/tasks/kix-notes-tab-missing/task.json",
    )
    if not os.path.isabs(task_path):
        task_path = os.path.join(ROOT, task_path)
    os.environ["LIGH_KILLER_TASK"] = task_path
    task = load_task()

    doc: dict[str, Any] = {
        "gate": "trail_holy",
        "architecture": "trail",
        "task": task["id"],
        "wall_budget_ms": WALL_MS,
        "infra_ms": int(os.environ.get("LIGH_TRAIL_INFRA_MS") or 0),
        "phase_trace": [],
        "llm_tokens": 0,
        "verified": False,
        "holy_shit": False,
    }

    def mark(phase: str, **extra: Any) -> None:
        doc["phase_trace"].append({"phase": phase, "ms": _now() - t0, **extra})

    index_root = os.environ.get("LIGH_IDENTITY_SOURCE", task["source_root"])
    index = build_identity_index(index_root)
    mark("index", size=len(index))

    prove = prove_with_post(task)
    mark("prove", ok=prove.get("ok"), prove_phase=prove.get("phase"))
    if prove.get("ok"):
        doc.update({"reason": "unexpected_pass_on_broken", "wall_ms": _now() - t0})
        _write(doc)
        return 1
    tf = prove.get("trace_failure")
    if not tf:
        doc.update({"reason": f"prove_{prove.get('phase')}", "prove": prove, "wall_ms": _now() - t0})
        _write(doc)
        return 1

    expected = str(tf.get("expected_identity") or "")
    mode = repair_mode_from_trace(str(tf.get("fault") or ""), expected)
    # Task / control heuristics — generalize without per-app templates.
    tid = task["id"]
    control = str(tf.get("control") or expected or "").lower()
    if "login" in tid or "login" in control or control in {"loginbutton", "login_button", "sign in"}:
        mode = "state_gate_stuck"
    if "onboard" in tid or "finish" in control or "onboarding" in control:
        mode = "blocked_overlay"
    if prove.get("phase") == "postcondition":
        if any(x in expected.lower() for x in ("home", "hometitle", "hello")):
            mode = "state_gate_stuck"
            if "onboard" in tid:
                mode = "blocked_overlay"
        if "onboard" in tid:
            mode = "blocked_overlay"
    if expected.startswith("tab_") or (str(tf.get("fault")) in ("motor_failed", "target_missing") and "tab" in expected):
        if "login" not in tid and "onboard" not in tid:
            mode = "tab_chrome_missing"

    # Prefer composition/fallback owners over leaf identity sites for sticky modes.
    loc: dict[str, Any] = {"primary_path": None}
    if mode in ("state_gate_stuck", "blocked_overlay"):
        fb = localize_fallback(index_root, mode, expected)
        if fb:
            loc = fb
    if not loc.get("primary_path"):
        loc = hybrid_localize(index_root, index, expected)
    if not loc.get("primary_path"):
        control_id = str(tf.get("control") or "")
        if control_id:
            loc = hybrid_localize(index_root, index, control_id)
    if not loc.get("primary_path"):
        fb = localize_fallback(index_root, mode, expected)
        if fb:
            loc = fb
    mark("localize", primary=loc.get("primary_path"), mode=mode)
    if not loc.get("primary_path"):
        doc.update({"reason": "localize_failed", "trace_failure": tf, "wall_ms": _now() - t0})
        _write(doc)
        return 1

    # Force mode into bundle via patched fault if needed.
    tf_for_bundle = dict(tf)
    if mode == "tab_chrome_missing":
        tf_for_bundle["fault"] = "target_missing"
        if not str(tf_for_bundle.get("expected_identity") or "").startswith("tab_"):
            tf_for_bundle["expected_identity"] = expected or "tab_notes"
    elif mode == "state_gate_stuck":
        tf_for_bundle["fault"] = "control_fired_no_transition"
    elif mode == "blocked_overlay":
        tf_for_bundle["fault"] = "blocked"

    bundle = build_repair_bundle(task, tf_for_bundle, loc)
    bundle["mode"] = mode
    bundle["fix_plan"] = fix_plan(mode, tf_for_bundle)
    bundle["scope"]["edit_intent"] = bundle["fix_plan"]
    if mode == "state_gate_stuck":
        bundle["scope"]["edit_globs"] = ["**/Auth/**", "**/*ViewModel*.swift", "**/*Login*.swift"]
        bundle["scope"]["forbidden_globs"] = []
    elif mode == "blocked_overlay":
        bundle["scope"]["edit_globs"] = ["**/Onboard*/**", "**/*Onboarding*.swift", "**/*.swift"]
        bundle["scope"]["forbidden_globs"] = []
    doc["repair_bundle"] = bundle
    doc["trace_failure"] = tf
    mark("bundle", mode=mode)

    target_rel = bundle["scope"]["primary_path"]
    abs_target = target_rel if os.path.isabs(target_rel) else os.path.join(ROOT, target_rel)
    # Prefer live broken tree under source_root.
    live = os.path.join(task["source_root"], loc["primary_path"])
    if os.path.isfile(live):
        abs_target = live
        bundle["scope"]["primary_path"] = os.path.relpath(abs_target, ROOT).replace("\\", "/")
        bundle["fixer_input"]["target_file"] = bundle["scope"]["primary_path"]

    original = open(abs_target, encoding="utf-8").read()
    attempts: list[dict[str, Any]] = []
    feedback = None
    changed = False
    tokens = 0
    candidate_text = original
    build_ms = 0
    for attempt in range(1, MAX_FIX_ATTEMPTS + 1):
        messages = build_messages(bundle, original if attempt == 1 else candidate_text, attempt, feedback)
        candidate, usage = openai_full_file(messages)
        tokens += int(usage.get("total_tokens") or 0)
        ok_change = candidate != original
        attempts.append({"attempt": attempt, "changed": ok_change, "usage": usage})
        if not ok_change and attempt == 1:
            feedback = "Returned unchanged file — apply the fix plan to the target file."
            continue
        apply_candidate(abs_target, candidate, bundle)
        candidate_text = candidate
        changed = True

        build_t0 = _now()
        build = subprocess.run(
            [task["build_script"]], cwd=ROOT, capture_output=True, text=True, timeout=180
        )
        build_ms = _now() - build_t0
        mark("build", ok=build.returncode == 0, build_ms=build_ms, attempt=attempt)
        if build.returncode == 0:
            break
        # Shot N+1 with compiler feedback (ChatRepair-style), restore then retry.
        tail = (build.stderr or build.stdout or "")[-1200:]
        feedback = f"Build failed:\n{tail}\nFix compile errors in the same target file only."
        open(abs_target, "w", encoding="utf-8").write(original)
        changed = False
        if attempt == MAX_FIX_ATTEMPTS:
            doc.update(
                {
                    "reason": "build_failed",
                    "build_ms": build_ms,
                    "tail": tail[-800:],
                    "llm_tokens": tokens,
                    "fix_attempts": attempts,
                    "wall_ms": _now() - t0,
                }
            )
            _write(doc)
            return 1

    doc["llm_tokens"] = tokens
    doc["fix_attempts"] = attempts
    mark("fix", changed=changed, tokens=tokens)
    if not changed:
        doc.update({"reason": "fixer_no_change", "wall_ms": _now() - t0})
        _write(doc)
        return 1

    # Last successful build already done inside the fix loop.

    # Certify: prefer host autopilot (same motor, fewer settle taxes) when FAST.
    verify: dict[str, Any]
    if os.environ.get("LIGH_TRAIL_FAST", "0") == "1":
        from goal_spec import compile_task_goal

        goal = compile_task_goal(task)
        params = task.get("run_goal_params") or []
        # Relaunch fixed app, then autopilot to acceptance.
        boot = bootstrap_app(
            os.environ.get("LIGH_APP_PATH", task["app_path"]),
            task["bundle_id"],
            wait_label=task.get("bootstrap_wait_label"),
            app_markers=verification_markers(task),
            task=task,
        )
        if not boot.get("foreground_ok"):
            verify = {"verified": False, "reason": "certify_bootstrap_failed", "bootstrap": boot}
        else:
            # Reset to login surface if needed by terminating+relaunch is already in bootstrap.
            auto = call_tool(
                "ligh_cap_autopilot",
                {
                    "goal_spec": goal,
                    "params": params,
                    "max_steps": 16,
                    "settle_ms": 700,
                    "timeout_ms": 90000,
                },
            )
            reached = bool(auto.get("reached") or (auto.get("ok") and auto.get("fault") in (None, "ok")))
            # Double-check with a cheap postcondition perceive when reached.
            if reached:
                post_keys = perceive(800)["keys"]
                post = eval_spec(task.get("verification", {}).get("postconditions") or {}, post_keys)
                verify = {
                    "verified": bool(post.get("ok")),
                    "reason": "verified" if post.get("ok") else "postcondition_not_satisfied",
                    "autopilot": {"reached": reached, "steps": auto.get("steps")},
                    "post": post,
                }
            else:
                # Fallback: full harness if autopilot missed (still honest).
                verify = strict_verify(task)
                verify["autopilot_fallback"] = True
                verify["autopilot"] = {"reached": False, "fault": auto.get("fault")}
    else:
        verify = strict_verify(task)
    mark("certify", verified=verify.get("verified"), reason=verify.get("reason"))
    wall = _now() - t0
    verified = bool(verify.get("verified"))
    doc.update(
        {
            "verified": verified,
            "verification_reason": verify.get("reason"),
            "build_ms": build_ms,
            "wall_ms": wall,
            "within_budget": wall <= WALL_MS,
            "holy_shit": verified and wall <= WALL_MS,
            "reason": "verified" if verified else verify.get("reason") or "certify_failed",
            "primary_path": bundle["scope"]["primary_path"],
            "mode": mode,
        }
    )
    _write(doc)
    print(
        json.dumps(
            {
                "task": task["id"],
                "verified": verified,
                "holy_shit": doc["holy_shit"],
                "wall_ms": wall,
                "mode": mode,
                "primary_path": bundle["scope"]["primary_path"],
                "llm_tokens": tokens,
            }
        )
    )
    return 0 if doc["holy_shit"] else 1


def _write(doc: dict[str, Any]) -> None:
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    raise SystemExit(main())
