#!/usr/bin/env python3
"""TRAIL repair orchestrator — broken-tree only, no score-tuning oracles.

Pipeline (single wall, hot path):
  prove (trace) → localize → constrained LLM fix (≤2 shots) → build → certify

Architecture rules (product path):
  - Index the *broken* source tree only (never a healthy BACKUP twin)
  - Mode from TraceFailure / prove phase — not task id strings
  - Localize missing AX ids via TabView structure + observed siblings
  - Certify = same exercise oracle (no autopilot soft-pass)
  - ≤2 LLM shots, no golden reverse

Usage:
  LIGH_TRAIL_TASK=fixtures/frozen/tasks/kix-notes-tab-missing/task.json \\
    python3 scripts/trail_holy.py
"""

from __future__ import annotations

import json
import os
import re
import subprocess
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
    fresh_install_app,
    perceive,
    run_steps,
    strict_verify,
    verification_markers,
)
from effect_classifier import enrich_trace_failure  # noqa: E402
from repair_job import (  # noqa: E402
    build_repair_bundle,
    ensure_ligh_session,
    fix_plan,
)
from trail_fixer import apply_candidate, build_messages, openai_full_file  # noqa: E402
from repair_engine import (  # noqa: E402
    RepairContext,
    causal_localize,
    classify,
    graph_neighborhood,
    try_structural_fixes,
)

def _exported_type_names(swift: str) -> set[str]:
    import re

    names = set()
    for m in re.finditer(r"\b(?:struct|class|enum)\s+(\w+)\b", swift):
        names.add(m.group(1))
    return names


def _symbol_retention_ok(original: str, candidate: str) -> bool:
    """Reject edits that delete types the App entry may still reference (OSS-safe)."""
    before = _exported_type_names(original)
    after = _exported_type_names(candidate)
    # Allow adding types; forbid dropping any previously exported type.
    return before.issubset(after)
OUT = os.environ.get(
    "LIGH_TRAIL_HOLY_OUT",
    os.path.join(ROOT, "docs/assets/trail-holy-latest.json"),
)
WALL_MS = int(os.environ.get("LIGH_TRAIL_WALL_MS", "120000"))
MAX_FIX_ATTEMPTS = int(os.environ.get("LIGH_TRAIL_FIX_ATTEMPTS", "2"))


def _now() -> int:
    return int(time.time() * 1000)


def infer_mode(tf: dict[str, Any], prove_phase: str | None) -> str:
    """Mode from TraceFailure — OSS-general Effect Classifier."""
    return str(classify(tf, prove_phase=prove_phase)["mode"])


def localize_from_trace(
    ctx: RepairContext,
    tf: dict[str, Any],
    mode: str,
) -> dict[str, Any]:
    """Delegate to repair_engine causal localizer."""
    return causal_localize(ctx, tf, mode).as_dict()


def prove_with_post(task: dict[str, Any], *, install: bool | None = None) -> dict[str, Any]:
    """Exercise then postcondition — TraceFailure from first motor or oracle miss."""
    from goal_spec import compile_task_goal
    from killer_loop_verify import evaluate_goal_stable

    app = os.environ.get("LIGH_APP_PATH", task["app_path"])
    bundle_id = task["bundle_id"]
    ver = task.get("verification") or {}
    wait_label = task.get("bootstrap_wait_label")
    markers = verification_markers(task)
    fast = os.environ.get("LIGH_TRAIL_FAST", "0") == "1"
    post_ms = 800 if fast else 2500
    peek_ms = 400 if fast else 800

    if not ensure_ligh_session().get("ok"):
        return {"ok": False, "phase": "session"}

    boot = bootstrap_app(
        app,
        bundle_id,
        wait_label=wait_label,
        app_markers=markers,
        task=task,
        install=install,
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
                "observed_identities": sorted(perceive(peek_ms)["keys"])[:24],
                "fault": fail.get("fault") or "setup_failed",
                "label": fail.get("label"),
            },
            "trace": setup,
        }

    pre = eval_spec(ver.get("preconditions") or {}, perceive(min(900, post_ms + 100))["keys"])
    if not pre.get("ok"):
        return {"ok": False, "phase": "precondition", "evidence": pre}

    exercise = run_steps(ver.get("exercise") or [], "exercise")
    if exercise and not all(s.get("ok") for s in exercise):
        fail_idx = next(i for i, s in enumerate(exercise) if not s.get("ok"))
        fail = exercise[fail_idx]
        keys_before = fail.get("keys_before") or []
        keys = fail.get("keys_after") or sorted(perceive(peek_ms)["keys"])[:24]
        must = (ver.get("postconditions") or {}).get("must_see_labels") or []
        tf = enrich_trace_failure(
            {
                "step": fail_idx + 1,
                "action": fail.get("action") or "unknown",
                "expected_identity": fail.get("id") or fail.get("label") or "",
                "observed_identities": sorted(keys)[:24],
                "fault": fail.get("fault") or "exercise_failed",
                "label": fail.get("label"),
                "control": fail.get("id") or fail.get("label"),
            },
            keys_before=keys_before,
            keys_after=keys,
            sig_before=fail.get("sig_before"),
            sig_after=fail.get("sig_after"),
            acceptance_pending=[str(x) for x in must],
        )
        return {
            "ok": False,
            "phase": "exercise",
            "trace_failure": tf,
            "trace": exercise,
        }

    goal = compile_task_goal(task)
    post, obs = evaluate_goal_stable(goal, post_ms)
    if post.get("ok"):
        return {"ok": True, "phase": "postcondition", "trace": exercise}

    must = (ver.get("postconditions") or {}).get("must_see_labels") or []
    expected = str(must[0] if must else goal.get("target") or "goal")
    last = exercise[-1] if exercise else {}
    control = last.get("id") or last.get("label") or ""
    keys = sorted((obs.get("keys") or set()))[:24]
    fault = "control_fired_no_transition" if control else "acceptance_not_in_ax"
    tf = enrich_trace_failure(
        {
            "step": len(exercise) + 1,
            "action": "assert",
            "expected_identity": expected,
            "observed_identities": keys,
            "fault": fault,
            "label": expected,
            "control": control,
        },
        keys_after=keys,
        acceptance_pending=[str(x) for x in must if str(x) not in (obs.get("keys") or set())],
    )
    return {
        "ok": False,
        "phase": "postcondition",
        "trace_failure": tf,
        "trace": exercise,
        "post": post,
    }


def certify_trace(task: dict[str, Any]) -> dict[str, Any]:
    """Same exercise + postconditions — hard fail-closed (no autopilot recovery)."""
    app = os.environ.get("LIGH_APP_PATH", task["app_path"])
    if not os.path.isabs(app):
        app = os.path.join(ROOT, app)
    bundle_id = task["bundle_id"]
    ver = task.get("verification") or {}
    wait_label = task.get("bootstrap_wait_label")
    markers = verification_markers(task)
    fast = os.environ.get("LIGH_TRAIL_FAST", "0") == "1"
    post_ms = 700 if fast else 2000

    if not ensure_ligh_session().get("ok"):
        return {"verified": False, "reason": "certify_session_failed"}

    prev_no = os.environ.pop("LIGH_TRAIL_NO_INSTALL", None)
    boot: dict[str, Any] = {}
    try:
        for attempt in range(1, 3):
            # Clean app data so certify exercise starts from task preconditions (login, etc.).
            fresh_install_app(app, bundle_id)
            if not ensure_ligh_session().get("ok"):
                continue
            # simctl already installed; relaunch via ligh (full install thrashes to SpringBoard).
            boot = bootstrap_app(
                app,
                bundle_id,
                wait_label=wait_label,
                app_markers=markers,
                task=task,
                install=False,
            )
            if boot.get("foreground_ok"):
                break
            time.sleep(0.5 * attempt)
    finally:
        if prev_no is not None:
            os.environ["LIGH_TRAIL_NO_INSTALL"] = prev_no

    if not boot.get("foreground_ok"):
        return {"verified": False, "reason": "certify_bootstrap_failed", "bootstrap": boot}

    setup = run_steps(ver.get("initial_setup") or [], "certify_setup")
    if setup and not all(s.get("ok") for s in setup):
        fail = next(s for s in setup if not s.get("ok"))
        return {"verified": False, "reason": "certify_setup_failed", "fail": fail}

    exercise = run_steps(ver.get("exercise") or [], "certify_exercise")
    if exercise and not all(s.get("ok") for s in exercise):
        fail = next(s for s in exercise if not s.get("ok"))
        return {
            "verified": False,
            "reason": "certify_exercise_failed",
            "fail": fail,
            "trace": exercise,
        }

    post = eval_spec(ver.get("postconditions") or {}, perceive(post_ms)["keys"])
    if post.get("ok"):
        return {"verified": True, "reason": "verified", "post": post, "trace": exercise}
    return {
        "verified": False,
        "reason": "postcondition_not_satisfied",
        "post": post,
        "trace": exercise,
    }


def main() -> int:
    os.environ.setdefault("LIGH_TRAIL_FAST", "1")
    os.environ.setdefault("LIGH_TRAIL_SETTLE_CAP_MS", "700")

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
        "protocol": "broken_tree_only",
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

    # Broken tree only — ignore healthy BACKUP even if an old gate exported it.
    index_root = task["source_root"]
    if not os.path.isabs(index_root):
        index_root = os.path.join(ROOT, index_root)
    index = build_identity_index(index_root)
    ctx = RepairContext.from_source_root(index_root)
    mark("index", size=len(index), root=os.path.relpath(index_root, ROOT), kb=ctx.kb.to_dict())

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

    mode = infer_mode(tf, prove.get("phase"))
    decision = classify(tf, prove_phase=prove.get("phase"))
    mark("classify", mode=mode, refuse=decision.get("refuse_edit"))
    if decision.get("refuse_edit"):
        doc.update(
            {
                "reason": "effect_unclassified",
                "trace_failure": tf,
                "mode": mode,
                "wall_ms": _now() - t0,
            }
        )
        _write(doc)
        return 1

    loc_result = causal_localize(ctx, tf, mode)
    loc = loc_result.as_dict()
    mark(
        "localize",
        primary=loc.get("primary_path"),
        mode=mode,
        ascent=loc.get("ascent"),
        targets=loc.get("edit_targets"),
    )
    doc["ascent"] = loc.get("ascent")
    if not loc.get("primary_path"):
        doc.update(
            {
                "reason": "localize_failed",
                "trace_failure": tf,
                "mode": mode,
                "wall_ms": _now() - t0,
            }
        )
        _write(doc)
        return 1

    tf_for_bundle = dict(tf)
    if mode == "tab_chrome_missing":
        tf_for_bundle["fault"] = "target_missing"
    elif mode == "state_gate_stuck":
        tf_for_bundle["fault"] = "control_fired_no_transition"
    elif mode == "blocked_overlay":
        tf_for_bundle["fault"] = "blocked"

    bundle = build_repair_bundle(task, tf_for_bundle, loc)
    bundle["mode"] = mode
    bundle["graph_neighborhood"] = graph_neighborhood(
        ctx, loc["primary_path"], str(tf.get("control") or "")
    )
    bundle["fix_plan"] = fix_plan(mode, tf_for_bundle)
    bundle["scope"]["edit_intent"] = bundle["fix_plan"]
    if mode == "state_gate_stuck":
        bundle["scope"]["edit_globs"] = [
            "**/*ViewModel*.swift",
            "**/*Login*.swift",
            "**/Auth/**",
            "**/*.swift",
        ]
        bundle["scope"]["forbidden_globs"] = []
    elif mode == "blocked_overlay":
        bundle["scope"]["edit_globs"] = ["**/*.swift"]
        bundle["scope"]["forbidden_globs"] = []
    elif mode == "tab_chrome_missing":
        bundle["scope"]["edit_globs"] = ["**/*Tab*.swift", "**/Navigation/**", "**/*.swift"]
        bundle["scope"]["forbidden_globs"] = ["**/Auth/**"]

    doc["repair_bundle"] = bundle
    doc["structural_kb"] = ctx.kb.to_dict()
    doc["graph_neighborhood"] = bundle["graph_neighborhood"]
    doc["trace_failure"] = tf
    mark("bundle", mode=mode)

    target_rel = bundle["scope"]["primary_path"]
    abs_target = target_rel if os.path.isabs(target_rel) else os.path.join(ROOT, target_rel)
    live = os.path.join(task["source_root"] if os.path.isabs(task["source_root"]) else os.path.join(ROOT, task["source_root"]), loc["primary_path"])
    # loc primary is relative to source_root
    live = os.path.join(index_root, loc["primary_path"])
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
    expected_id = str(tf.get("expected_identity") or "")
    prior_tab_ids = set(re.findall(r'\.accessibilityIdentifier\(\s*"(tab_[^"]+)"\s*\)', original))

    def _run_build() -> tuple[bool, int, str]:
        build_t0 = _now()
        build = subprocess.run(
            [
                task["build_script"]
                if os.path.isabs(task["build_script"])
                else os.path.join(ROOT, task["build_script"])
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=180,
        )
        ms = _now() - build_t0
        tail = (build.stderr or build.stdout or "")[-1200:]
        return build.returncode == 0, ms, tail

    # R4 structural operators (effect-class) before LLM.
    structural_hits = try_structural_fixes(ctx, mode, tf, loc_result)
    for hit in structural_hits:
        if changed:
            break
        apply_candidate(hit["abs_path"], hit["text"], bundle)
        ok, build_ms, tail = _run_build()
        mark(
            "build",
            ok=ok,
            build_ms=build_ms,
            attempt=0,
            method=hit.get("method"),
            file=hit.get("file"),
        )
        attempts.append(
            {
                "attempt": 0,
                "changed": True,
                "method": hit.get("method"),
                "build_ok": ok,
                "file": hit.get("file"),
            }
        )
        if ok:
            abs_target = hit["abs_path"]
            bundle["scope"]["primary_path"] = os.path.relpath(abs_target, ROOT).replace("\\", "/")
            bundle["fixer_input"]["target_file"] = bundle["scope"]["primary_path"]
            candidate_text = hit["text"]
            changed = True
            tokens = 0
        else:
            open(hit["abs_path"], "w", encoding="utf-8").write(hit["original"])
            feedback = (
                f"Structural operator ({hit.get('method')}) build failed:\n{tail}\n"
                "Apply a minimal fix in the target file only."
            )

    for attempt in range(1, MAX_FIX_ATTEMPTS + 1):
        if changed:
            break
        messages = build_messages(
            bundle, original if attempt == 1 else candidate_text, attempt, feedback
        )
        # Tighten tab restore prompts: surgical insert, keep sibling tabs.
        if mode == "tab_chrome_missing" and attempt == 1:
            messages = list(messages)
            messages[0] = {
                **messages[0],
                "content": (
                    messages[0]["content"]
                    + " For missing tabs: insert one TabView child modeled on siblings; "
                    "do not rewrite or delete existing tabs/identifiers."
                ),
            }
        candidate, usage = openai_full_file(messages)
        tokens += int(usage.get("total_tokens") or 0)
        ok_change = candidate != original
        attempts.append({"attempt": attempt, "changed": ok_change, "usage": usage})
        if not ok_change and attempt == 1:
            feedback = "Returned unchanged file — apply the fix plan to the target file."
            continue
        if mode == "tab_chrome_missing" and expected_id and expected_id not in candidate:
            feedback = (
                f"Missing accessibilityIdentifier '{expected_id}' in the TabView composition. "
                "Restore that tab item and identifier. Keep the rest of the file unchanged."
            )
            continue
        if prior_tab_ids:
            kept = set(
                re.findall(r'\.accessibilityIdentifier\(\s*"(tab_[^"]+)"\s*\)', candidate)
            )
            dropped = sorted(prior_tab_ids - kept)
            if dropped:
                feedback = (
                    f"Do not drop existing tab identifiers {dropped}. "
                    f"Only add '{expected_id}' (and keep siblings)."
                )
                continue
        if candidate.count("{") != candidate.count("}"):
            feedback = "Output looks truncated or unbalanced braces — return the complete Swift file only."
            continue
        if not _symbol_retention_ok(original, candidate):
            missing = sorted(_exported_type_names(original) - _exported_type_names(candidate))
            feedback = (
                f"Do not delete exported types {missing}. Keep existing struct/class/enum declarations; "
                "apply a minimal state/navigation fix only."
            )
            continue
        apply_candidate(abs_target, candidate, bundle)
        candidate_text = candidate
        changed = True

        ok, build_ms, tail = _run_build()
        mark("build", ok=ok, build_ms=build_ms, attempt=attempt)
        if ok:
            break
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
                    "mode": mode,
                    "primary_path": bundle["scope"]["primary_path"],
                }
            )
            _write(doc)
            return 1

    doc["llm_tokens"] = tokens
    doc["fix_attempts"] = attempts
    mark("fix", changed=changed, tokens=tokens)
    if not changed:
        doc.update({"reason": "fixer_no_change", "wall_ms": _now() - t0, "mode": mode})
        _write(doc)
        return 1

    if os.environ.get("LIGH_TRAIL_FAST", "0") == "1":
        verify = certify_trace(task)
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
                "protocol": "broken_tree_only",
            }
        )
    )
    return 0 if doc.get("holy_shit") else 1


def _write(doc: dict[str, Any]) -> None:
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    raise SystemExit(main())
