#!/usr/bin/env python3
"""Validation-week reporter — coverage, ingest, summary, failure taxonomy.

Does not change Autopilot. It only records whether the current claim survives
a growing matrix of tasks, apps, and repeated paired runs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import sys
from datetime import datetime, timezone
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
MATRIX_PATH = os.path.join(ROOT, "fixtures/validation-week/matrix.json")
RUNS_DIR = os.path.join(ROOT, "docs/assets/validation-week-runs")
SUMMARY_PATH = os.path.join(ROOT, "docs/assets/validation-week-summary.json")
RUNS_INDEX_PATH = os.path.join(ROOT, "docs/assets/validation-week-runs.json")
RESULTS_MD = os.path.join(ROOT, "docs/VALIDATION_WEEK_RESULTS.md")
RUN_SCHEMA_PATH = os.path.join(ROOT, "docs/assets/validation-week-run.schema.json")

TAXONOMY = (
    "ok",
    "planner_wrong_path",
    "perception_mislabeled",
    "async_state",
    "harness_wrong",
    "source_fix_wrong",
    "infra_flake",
    "unclassified",
)

FAILURE_PHASES = {
    "bootstrap",
    "setup",
    "precondition",
    "exercise",
    "postcondition",
    "agent",
    "protocol",
    "infra",
    "unknown",
}


def load_matrix() -> dict[str, Any]:
    with open(MATRIX_PATH, encoding="utf-8") as f:
        return json.load(f)


def sha256_file(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def prompt_hash(doc: dict[str, Any]) -> str:
    existing = doc.get("prompt_hash") or doc.get("task_prompt_hash")
    if isinstance(existing, str) and len(existing) == 64:
        return existing
    prompt = str(doc.get("task_prompt") or doc.get("agent_prompt") or "")
    return hashlib.sha256(prompt.encode("utf-8")).hexdigest()


def validate_run(row: dict[str, Any], *, source: str) -> None:
    try:
        import jsonschema
    except ImportError as exc:
        raise RuntimeError("run validation requires the 'jsonschema' Python package") from exc
    with open(RUN_SCHEMA_PATH, encoding="utf-8") as f:
        schema = json.load(f)
    validator = jsonschema.Draft202012Validator(schema)
    errors = sorted(validator.iter_errors(row), key=lambda e: list(e.absolute_path))
    if errors:
        err = errors[0]
        location = ".".join(str(part) for part in err.absolute_path) or "<root>"
        raise ValueError(f"{source}: run schema v2 violation at {location}: {err.message}")


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
    except Exception:
        return "unknown"


def percentile(vals: list[float], p: float) -> float | None:
    vals = sorted(v for v in vals if isinstance(v, (int, float)))
    if not vals:
        return None
    if len(vals) == 1:
        return float(vals[0])
    k = (len(vals) - 1) * p
    f = int(k)
    c = min(f + 1, len(vals) - 1)
    if f == c:
        return float(vals[f])
    return float(vals[f] + (vals[c] - vals[f]) * (k - f))


def median(vals: list[Any]) -> float | None:
    nums = [v for v in vals if isinstance(v, (int, float))]
    return float(statistics.median(nums)) if nums else None


def classify_diagnosis(doc: dict[str, Any]) -> str:
    if doc.get("pass") or doc.get("claim_pass") or doc.get("verified"):
        if doc.get("false_success"):
            return "harness_wrong"
        return "ok"
    reason = str(doc.get("verification_reason") or doc.get("reason") or "").lower()
    diagnosis = doc.get("diagnosis") or {}
    if isinstance(diagnosis, dict):
        code = str(diagnosis.get("code") or "").lower()
    else:
        code = str(diagnosis or "").lower()
    blob = " ".join([reason, code, str(doc.get("stop_code") or "").lower()])
    if "initial_state" in blob or "sim" in blob or "timeout" in blob and "infra" in blob:
        return "infra_flake"
    if "false_success" in blob or doc.get("false_success"):
        return "harness_wrong"
    if any(k in blob for k in ("secure", "misclass", "springboard", "eyes_unusable", "perceive")):
        return "perception_mislabeled"
    if any(k in blob for k in ("async", "delay", "settle", "race")):
        return "async_state"
    if any(k in blob for k in ("no_transition", "wrong_path", "exhausted", "dismiss", "motor_rejected")):
        return "planner_wrong_path"
    pe = doc.get("patch_economics") or {}
    writes = pe.get("writes") or len(doc.get("code_changes") or [])
    if writes and not (doc.get("pass") or doc.get("verified")):
        return "source_fix_wrong"
    if not writes and doc.get("kind") != "navigation":
        return "planner_wrong_path"
    return "unclassified"


def infer_failure_phase(doc: dict[str, Any]) -> str | None:
    if doc.get("pass") or doc.get("claim_pass") or doc.get("verified"):
        return None
    phase = str(
        doc.get("failure_phase")
        or (doc.get("strict_verify") or {}).get("phase")
        or ""
    ).lower()
    aliases = {
        "pre": "precondition",
        "post": "postcondition",
        "initial_state": "bootstrap",
        "host_accept": "postcondition",
    }
    phase = aliases.get(phase, phase)
    if phase in FAILURE_PHASES:
        return phase
    reason = str(doc.get("verification_reason") or doc.get("reason") or "").lower()
    if "initial_state" in reason or "foreground" in reason:
        return "bootstrap"
    if "exercise" in reason:
        return "exercise"
    if "precondition" in reason:
        return "precondition"
    if "postcondition" in reason:
        return "postcondition"
    if classify_diagnosis(doc) == "infra_flake":
        return "infra"
    return "agent"


def protocol_violations(doc: dict[str, Any]) -> list[str]:
    violations = list(doc.get("protocol_violations") or [])
    actions = doc.get("agent_actions") or []
    if any(isinstance(a, dict) and a.get("action") == "exercise_app" for a in actions):
        violations.append("exercise_app_used")
    return sorted(set(str(v) for v in violations))


def coverage(matrix: dict[str, Any], runs: list[dict[str, Any]]) -> dict[str, Any]:
    bar = matrix["minimum_bar"]
    apps = [a for a in matrix["apps"] if a.get("status") != "planned"]
    tasks = matrix["tasks"]
    runnable = [t for t in tasks if t.get("status") == "runnable"]
    planned = [t for t in tasks if t.get("status") == "planned"]
    by_app: dict[str, dict[str, int]] = {}
    for t in tasks:
        app = t["app_id"]
        row = by_app.setdefault(app, {"runnable": 0, "planned": 0, "bugfix": 0, "recovery": 0, "navigation": 0})
        row[t.get("status", "planned")] = row.get(t.get("status", "planned"), 0) + 1
        kind = t.get("kind") or "bugfix"
        if kind in row:
            row[kind] += 1
    third_party = [a for a in apps if a.get("third_party")]
    priority = [t["id"] for t in tasks if t.get("repeat_priority") and t.get("status") == "runnable"]
    repeats_by_task: dict[str, int] = {}
    complete_pairs, _ = join_scored_pairs(runs)
    for auto, _ in complete_pairs:
        tid = auto.get("task")
        repeats_by_task[tid] = repeats_by_task.get(tid, 0) + 1
    priority_with_enough = [
        tid for tid in priority if repeats_by_task.get(tid, 0) >= bar["paired_repeats_on_priority_tasks"]
    ]
    # Strict runnable bar: each core app must have 5 runnable tasks.
    # Core apps for the week floor: one fixture + two distinct third-party shapes.
    core_apps = ("lighonboard", "xcuitestdemo", "kix")
    tasks_per_app_ok = all(
        (by_app.get(a, {}).get("runnable", 0) + by_app.get(a, {}).get("planned", 0))
        >= bar["tasks_per_app"]
        or by_app.get(a, {}).get("runnable", 0) >= bar["tasks_per_app"]
        for a in core_apps
    )
    runnable_per_core = {
        app: by_app.get(app, {}).get("runnable", 0) for app in core_apps
    }
    runnable_bar_met = all(v >= bar["tasks_per_app"] for v in runnable_per_core.values())
    third_party_ready = [
        a for a in matrix["apps"] if a.get("third_party") and a.get("status") != "planned"
    ]
    week_complete = bool(
        len(apps) >= bar["apps_total"]
        and len(third_party_ready) >= bar["third_party_apps"]
        and runnable_bar_met
        and len(priority_with_enough) >= min(len(priority), bar["priority_tasks_with_repeats"])
        and os.path.isfile(SUMMARY_PATH)
    )
    gaps = []
    if not runnable_bar_met:
        gaps.append(f"need {bar['tasks_per_app']} runnable tasks per core app; have {runnable_per_core}")
    if len(priority_with_enough) < min(len(priority), bar["priority_tasks_with_repeats"]):
        gaps.append(
            f"need {bar['paired_repeats_on_priority_tasks']} autopilot repeats on "
            f"{bar['priority_tasks_with_repeats']} priority tasks; enough on {priority_with_enough or []}"
        )
    if "kix" not in {a["id"] for a in third_party_ready}:
        gaps.append("need third-party app Kix vendored and runnable")
    return {
        "apps_defined": len(apps),
        "third_party_apps": len(third_party),
        "runnable_tasks": len(runnable),
        "planned_tasks": len(planned),
        "runnable_per_core_app": runnable_per_core,
        "tasks_per_app_ok": tasks_per_app_ok,
        "runnable_bar_met": runnable_bar_met,
        "priority_tasks": priority,
        "autopilot_repeats_by_task": repeats_by_task,
        "priority_tasks_with_enough_repeats": priority_with_enough,
        "week_complete": week_complete,
        "gaps": gaps,
        "by_app": by_app,
    }


def load_runs(path: str = RUNS_DIR) -> list[dict[str, Any]]:
    if not os.path.isdir(path):
        return []
    rows = []
    for name in sorted(os.listdir(path)):
        if (
            not name.endswith(".json")
            or name.endswith("-row.json")
            or name.endswith("-raw.json")
        ):
            continue
        fp = os.path.join(path, name)
        try:
            with open(fp, encoding="utf-8") as f:
                doc = json.load(f)
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError(f"invalid ledger artifact {fp}: {exc}") from exc
        # A ledger entry is exactly one run. Aggregate documents are never expanded
        # because that would fabricate trial-level observations from summary data.
        if isinstance(doc, dict) and "arm" in doc:
            if doc.get("schema_version") == 2:
                validate_run(doc, source=fp)
            doc["_ledger_path"] = fp
            rows.append(doc)
    return rows


def normalize_run(doc: dict[str, Any], *, source: str) -> dict[str, Any]:
    pe = doc.get("patch_economics") or {}
    detail = doc.get("detail") if isinstance(doc.get("detail"), dict) else {}
    passed = doc.get(
        "pass",
        doc.get("claim_pass")
        or doc.get("verified")
        or doc.get("reached")
        or detail.get("reached"),
    )
    row = {
        "schema_version": 2,
        "app": doc.get("app") or doc.get("app_id"),
        "task": doc.get("task") or doc.get("task_id") or doc.get("id"),
        "arm": doc.get("arm") or "autopilot",
        "model": doc.get("model") or os.environ.get("OPENAI_MODEL", "gpt-5-mini"),
        "protocol": doc.get("protocol") or "legacy",
        "protocol_version": int(doc.get("protocol_version") or 1),
        "prompt_hash": prompt_hash(doc),
        "git_sha": doc.get("git_sha") or git_sha(),
        "wall_time_ms": doc.get("wall_time_ms") or doc.get("wall_ms") or detail.get("elapsed_ms"),
        "llm_tokens": doc.get("llm_tokens") if doc.get("llm_tokens") is not None else detail.get("llm_tokens"),
        "pass": bool(passed),
        "diagnosis_class": doc.get("diagnosis_class") or doc.get("failure_class") or classify_diagnosis(doc),
        "patches": pe.get("patch_attempts") or doc.get("patches"),
        "builds": doc.get("build_attempts") or doc.get("builds") or pe.get("builds"),
        "run_id": doc.get("run_id") or os.path.splitext(os.path.basename(source))[0],
        "repeat_index": doc.get("repeat_index"),
        "kind": doc.get("kind"),
        "false_success": bool(doc.get("false_success")),
        "verification_reason": doc.get("verification_reason") or doc.get("reason"),
        "failure_phase": doc.get("failure_phase") or infer_failure_phase(doc),
        "artifact_origin": doc.get("artifact_origin") or "legacy_unknown",
        "raw_artifact": doc.get("raw_artifact") or source,
        "raw_artifact_sha256": doc.get("raw_artifact_sha256"),
        "scored_eligible": bool(doc.get("scored_eligible", False)),
        "protocol_violations": list(doc.get("protocol_violations") or []),
        "source": source,
    }
    return row


def record_raw_run(
    src: str,
    *,
    app: str | None = None,
    task: str | None = None,
    arm: str | None = None,
    repeat: int | None = None,
    kind: str | None = None,
    dest: str,
    historical: bool = False,
) -> tuple[dict[str, Any], bool]:
    """Append one immutable ledger row derived from one raw trial artifact."""
    with open(src, encoding="utf-8") as f:
        doc = json.load(f)
    if not isinstance(doc, dict) or "runs" in doc and "arm" not in doc:
        raise ValueError(f"{src}: expected one raw per-run object, not an aggregate")
    row = normalize_run(doc, source=os.path.relpath(src, ROOT))
    row["app"] = app or row["app"]
    row["task"] = task or row["task"]
    row["arm"] = arm or row["arm"]
    row["repeat_index"] = repeat if repeat is not None else row.get("repeat_index")
    row["kind"] = kind or row.get("kind") or "bugfix"
    row["git_sha"] = doc.get("git_sha") or git_sha()
    row["artifact_origin"] = "raw_per_run"
    row["raw_artifact"] = os.path.relpath(src, ROOT)
    row["raw_artifact_sha256"] = sha256_file(src)
    row["source"] = row["raw_artifact"]
    row["historical"] = bool(historical)
    row["protocol_violations"] = protocol_violations(doc)
    row["scored_eligible"] = bool(
        row["kind"] == "bugfix"
        and row["protocol"] in ("honest", "scored")
        and not row["protocol_violations"]
    )
    if row["protocol_violations"]:
        row["pass"] = False
        row["diagnosis_class"] = "harness_wrong"
        row["failure_phase"] = "protocol"
        row["verification_reason"] = ",".join(row["protocol_violations"])
    validate_run(row, source=src)
    payload = json.dumps(row, indent=2) + "\n"
    if os.path.exists(dest):
        with open(dest, encoding="utf-8") as f:
            existing = f.read()
        if existing != payload:
            raise FileExistsError(f"append-only ledger entry already exists: {dest}")
        return row, False
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with open(dest, "x", encoding="utf-8") as f:
        f.write(payload)
    return row, True


def ingest_historical() -> list[str]:
    """Import only preserved raw trial artifacts; aggregate summaries are ignored."""
    candidates = [
        (
            "docs/assets/killer-loop-ab-v2-runs/autopilot-run1.json",
            "xcuitestdemo",
            "login-never-navigates",
            "autopilot",
            "bugfix",
        ),
        (
            "docs/assets/killer-loop-ab-v2-runs/baseline-run1.json",
            "xcuitestdemo",
            "login-never-navigates",
            "baseline",
            "bugfix",
        ),
    ]
    trace_dir = os.path.join(ROOT, "docs/assets/autopilot-generality-traces")
    if os.path.isdir(trace_dir):
        for name in sorted(os.listdir(trace_dir)):
            if name.endswith(".json") and not name.endswith("-row.json"):
                app_id = name[:-5]
                task_id = f"{app_id}-happy-path"
                if app_id == "xcuitestdemo":
                    task_id = "xcuitestdemo-home-visible"
                elif app_id == "kix":
                    task_id = "kix-home-visible"
                candidates.append(
                    (
                        f"docs/assets/autopilot-generality-traces/{name}",
                        app_id,
                        task_id,
                        "autopilot",
                        "navigation",
                    )
                )
    written = []
    for rel, app_id, task_id, arm, kind in candidates:
        src = os.path.join(ROOT, rel)
        if not os.path.isfile(src):
            continue
        dest = os.path.join(RUNS_DIR, f"raw-{arm}-{task_id}-r1.json")
        _, created = record_raw_run(
            src,
            app=app_id,
            task=task_id,
            arm=arm,
            repeat=1,
            kind=kind,
            dest=dest,
            historical=True,
        )
        if created:
            written.append(dest)
    return written


def arm_stats(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return {"runs": 0, "passes": 0, "pass_rate": None}
    walls = [r.get("wall_time_ms") for r in rows]
    toks = [r.get("llm_tokens") for r in rows]
    passes = sum(1 for r in rows if r.get("pass"))
    top1 = []
    for r in rows:
        patches = r.get("patches")
        if r.get("pass") and patches in (1, 1.0):
            top1.append(True)
        elif r.get("pass"):
            top1.append(False)
    classes: dict[str, int] = {}
    for r in rows:
        c = r.get("diagnosis_class") or "unclassified"
        classes[c] = classes.get(c, 0) + 1
    return {
        "runs": len(rows),
        "passes": passes,
        "pass_rate": round(passes / len(rows), 3) if rows else None,
        "median_wall_ms": median(walls),
        "p90_wall_ms": percentile([float(v) for v in walls if isinstance(v, (int, float))], 0.90),
        "median_tokens": median(toks),
        "p90_tokens": percentile([float(v) for v in toks if isinstance(v, (int, float))], 0.90),
        "median_patches": median([r.get("patches") for r in rows]),
        "median_builds": median([r.get("builds") for r in rows]),
        "top1_acceptance": (sum(1 for t in top1 if t) / len(top1)) if top1 else None,
        "diagnosis_classes": classes,
    }


def join_scored_pairs(
    rows: list[dict[str, Any]],
) -> tuple[list[tuple[dict[str, Any], dict[str, Any]]], list[dict[str, Any]]]:
    """Join complete scored trials by task+repeat; reject ambiguous or asymmetric pairs."""
    grouped: dict[tuple[str, int], dict[str, list[dict[str, Any]]]] = {}
    for row in rows:
        if (row.get("kind") or "bugfix") != "bugfix" or not row.get("scored_eligible"):
            continue
        task = row.get("task")
        repeat = row.get("repeat_index")
        if not task or not isinstance(repeat, int):
            continue
        grouped.setdefault((str(task), repeat), {}).setdefault(str(row.get("arm")), []).append(row)

    pairs = []
    rejected = []
    for (task, repeat), arms in sorted(grouped.items()):
        auto = arms.get("autopilot") or []
        base = arms.get("baseline") or []
        reason = None
        if len(auto) != 1 or len(base) != 1:
            reason = "missing_or_duplicate_arm"
        elif auto[0].get("model") != base[0].get("model"):
            reason = "model_mismatch"
        elif auto[0].get("protocol_version") != base[0].get("protocol_version"):
            reason = "protocol_version_mismatch"
        elif auto[0].get("prompt_hash") != base[0].get("prompt_hash"):
            reason = "prompt_hash_mismatch"
        if reason:
            rejected.append({"task": task, "repeat_index": repeat, "reason": reason})
        else:
            pairs.append((auto[0], base[0]))
    return pairs, rejected


def summarize() -> dict[str, Any]:
    matrix = load_matrix()
    raw = load_runs()
    rows = [normalize_run(r, source=str(r.get("source") or r.get("run_id") or "run")) for r in raw]
    cov = coverage(matrix, rows)
    # Headline metrics contain complete task+repeat pairs only. Unpaired arm pools
    # can differ in task difficulty and are not a valid treatment comparison.
    pairs, rejected_pairs = join_scored_pairs(rows)
    auto = [pair[0] for pair in pairs]
    base = [pair[1] for pair in pairs]
    auto_s = arm_stats(auto)
    base_s = arm_stats(base)
    min_n = int(matrix["minimum_bar"].get("paired_min_n", 5))
    claim_sample_sufficient = len(pairs) >= min_n
    nav = [r for r in rows if r.get("kind") == "navigation"]
    nav_s = arm_stats([r for r in nav if r.get("arm") == "autopilot"])
    speedup = None
    token_ratio = None
    if auto_s.get("median_wall_ms") and base_s.get("median_wall_ms"):
        speedup = round(base_s["median_wall_ms"] / auto_s["median_wall_ms"], 2)
    if auto_s.get("median_tokens") and base_s.get("median_tokens"):
        token_ratio = round(base_s["median_tokens"] / auto_s["median_tokens"], 2)
    claim_stronger = bool(
        cov["week_complete"]
        and claim_sample_sufficient
        and speedup is not None
        and speedup >= 2.0
        and token_ratio is not None
        and token_ratio > 1.0
        and (auto_s.get("pass_rate") or 0) >= (base_s.get("pass_rate") or 0)
    )
    stop = []
    if claim_sample_sufficient and speedup is not None and speedup < 1.5:
        stop.append("median_speedup_below_1.5")
    if claim_sample_sufficient and token_ratio is not None and token_ratio <= 1.0:
        stop.append("token_win_disappeared")
    if (
        claim_sample_sufficient
        and auto_s.get("pass_rate") is not None
        and base_s.get("pass_rate") is not None
        and auto_s["pass_rate"] < base_s["pass_rate"]
    ):
        stop.append("pass_rate_below_baseline")
    report = {
        "gate": "validation_week",
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git_sha": git_sha(),
        "claim": matrix["claim"],
        "week_complete": cov["week_complete"],
        "claim_stronger": claim_stronger,
        "claim_sample_sufficient": claim_sample_sufficient,
        "claim_refusal_reason": (
            None
            if claim_sample_sufficient
            else f"paired_n_below_minimum:{len(pairs)}<{min_n}"
        ),
        "paired_min_n": min_n,
        "stop_conditions_hit": stop,
        "coverage": cov,
        "arms": {"autopilot": auto_s, "baseline": base_s},
        "navigation_smoke": nav_s,
        "speedup_vs_vision": speedup,
        "token_ratio_vs_vision": token_ratio,
        "run_count": len(rows),
        "paired_bugfix_pairs": len(pairs),
        "paired_bugfix_runs": len(pairs) * 2,
        "rejected_pairs": rejected_pairs,
        "interpretation": (
            "Week is complete only when the minimum bar is met. "
            "Speedup/token ratios use complete task+repeat bugfix pairs only; "
            f"claims require at least N={min_n} pairs; "
            "navigation smoke is reported separately and never mixed in."
        ),
    }
    os.makedirs(os.path.dirname(SUMMARY_PATH), exist_ok=True)
    open(SUMMARY_PATH, "w", encoding="utf-8").write(json.dumps(report, indent=2) + "\n")
    open(RUNS_INDEX_PATH, "w", encoding="utf-8").write(
        json.dumps({"runs": rows, "count": len(rows)}, indent=2) + "\n"
    )
    write_results_md(report, rows)
    return report


def write_results_md(report: dict[str, Any], rows: list[dict[str, Any]]) -> None:
    cov = report["coverage"]
    auto = report["arms"]["autopilot"]
    base = report["arms"]["baseline"]
    lines = [
        "# Validation week results",
        "",
        f"Generated: `{report['generated_at']}` · git `{report['git_sha'][:12]}`",
        "",
        f"**Week complete:** {'yes' if report['week_complete'] else 'no'}  ",
        f"**Claim stronger:** {'yes' if report['claim_stronger'] else 'not yet'}",
        f"**Scored pairs:** {report['paired_bugfix_pairs']} (minimum N={report['paired_min_n']})  ",
        f"**Claim refusal:** {report.get('claim_refusal_reason') or 'none'}",
        "",
        "## Coverage vs minimum bar",
        "",
        f"- runnable tasks: {cov['runnable_tasks']} (planned: {cov['planned_tasks']})",
        f"- runnable per core app: `{json.dumps(cov['runnable_per_core_app'])}`",
        f"- autopilot repeats by task: `{json.dumps(cov['autopilot_repeats_by_task'])}`",
        "",
    ]
    if cov["gaps"]:
        lines.append("Gaps:")
        lines.append("")
        for g in cov["gaps"]:
            lines.append(f"- {g}")
        lines.append("")
    nav = report.get("navigation_smoke") or {}
    lines += [
        "## Paired bugfix loop only",
        "",
        "Only complete scored task+repeat pairs enter these medians. Navigation/generality smoke is excluded.",
        "",
        f"| Arm | Runs | Pass rate | Median wall | p90 wall | Median tokens |",
        f"|-----|------|-----------|-------------|----------|---------------|",
        f"| Autopilot | {auto.get('runs')} | {auto.get('pass_rate')} | {auto.get('median_wall_ms')} | {auto.get('p90_wall_ms')} | {auto.get('median_tokens')} |",
        f"| Vision | {base.get('runs')} | {base.get('pass_rate')} | {base.get('median_wall_ms')} | {base.get('p90_wall_ms')} | {base.get('median_tokens')} |",
        "",
        f"- speedup vs vision (median wall): **{report.get('speedup_vs_vision')}**",
        f"- token ratio vs vision: **{report.get('token_ratio_vs_vision')}**",
        f"- navigation smoke (autopilot only): runs={nav.get('runs')} pass_rate={nav.get('pass_rate')} median_wall={nav.get('median_wall_ms')}",
        "",
        "## Failure taxonomy",
        "",
    ]
    classes = {}
    for r in rows:
        if r.get("pass"):
            continue
        c = r.get("diagnosis_class") or "unclassified"
        classes[c] = classes.get(c, 0) + 1
    if not classes:
        lines.append("No failed runs in the current artifact set.")
    else:
        for k, v in sorted(classes.items(), key=lambda kv: (-kv[1], kv[0])):
            lines.append(f"- `{k}`: {v}")
    lines += [
        "",
        "## Stop conditions hit",
        "",
    ]
    if report["stop_conditions_hit"]:
        for s in report["stop_conditions_hit"]:
            lines.append(f"- {s}")
    else:
        lines.append("None yet (dataset may still be too small to trigger them).")
    lines.append("")
    RESULTS_MD_DIR = os.path.dirname(RESULTS_MD)
    os.makedirs(RESULTS_MD_DIR, exist_ok=True)
    open(RESULTS_MD, "w", encoding="utf-8").write("\n".join(lines) + "\n")


def cmd_status() -> int:
    matrix = load_matrix()
    rows = [normalize_run(r, source=str(r.get("run_id") or "run")) for r in load_runs()]
    cov = coverage(matrix, rows)
    print("══ Validation week status ══")
    print(f"  runnable tasks: {cov['runnable_tasks']}   planned: {cov['planned_tasks']}")
    print(f"  per core app:   {json.dumps(cov['runnable_per_core_app'])}")
    print(f"  repeats:        {json.dumps(cov['autopilot_repeats_by_task'])}")
    print(f"  week complete:  {cov['week_complete']}")
    for g in cov["gaps"]:
        print(f"  gap: {g}")
    if not cov["gaps"]:
        print("  no coverage gaps")
    return 0 if cov["week_complete"] else 2


def cmd_ingest() -> int:
    written = ingest_historical()
    print("ingested:")
    for p in written:
        print(f"  {os.path.relpath(p, ROOT)}")
    summarize()
    print(f"→ {os.path.relpath(SUMMARY_PATH, ROOT)}")
    print(f"→ {os.path.relpath(RESULTS_MD, ROOT)}")
    return 0


def cmd_summarize() -> int:
    report = summarize()
    print(json.dumps({k: v for k, v in report.items() if k != "coverage"}, indent=2))
    print(f"week_complete={report['week_complete']} claim_stronger={report['claim_stronger']}")
    print(f"→ {os.path.relpath(SUMMARY_PATH, ROOT)}")
    return 0 if report["week_complete"] else 2


def cmd_validate() -> int:
    from killer_loop_task import validate_task

    task_count = 0
    tasks_root = os.path.join(ROOT, "fixtures/frozen/tasks")
    for dirpath, _, files in os.walk(tasks_root):
        if "task.json" not in files:
            continue
        path = os.path.join(dirpath, "task.json")
        with open(path, encoding="utf-8") as f:
            task = json.load(f)
        validate_task(task, path=path)
        task_count += 1
    rows = load_runs()
    v2_count = sum(1 for row in rows if row.get("schema_version") == 2)
    legacy_count = len(rows) - v2_count
    print(
        f"validated tasks={task_count} run_v2={v2_count} "
        f"legacy_compatible_unscored={legacy_count}"
    )
    return 0


def cmd_record(args: argparse.Namespace) -> int:
    os.makedirs(RUNS_DIR, exist_ok=True)
    with open(args.src, encoding="utf-8") as f:
        doc = json.load(f)
    task = args.task or doc.get("task") or doc.get("task_id")
    arm = args.arm or doc.get("arm") or "autopilot"
    repeat = args.repeat if args.repeat is not None else doc.get("repeat_index")
    dest = args.dest or os.path.join(
        RUNS_DIR, f"{arm}-{task}-r{repeat or 0}.json"
    )
    _, created = record_raw_run(
        args.src,
        app=args.app,
        task=args.task,
        arm=args.arm,
        repeat=args.repeat,
        kind=args.kind,
        dest=dest,
    )
    print(f"{dest} ({'appended' if created else 'already present'})")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description="Validation week reporter")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status")
    sub.add_parser("ingest")
    sub.add_parser("summarize")
    sub.add_parser("validate")
    rec = sub.add_parser("record")
    rec.add_argument("src")
    rec.add_argument("--app")
    rec.add_argument("--task")
    rec.add_argument("--arm")
    rec.add_argument("--repeat", type=int)
    rec.add_argument("--kind")
    rec.add_argument("--dest")
    args = p.parse_args()
    if args.cmd == "status":
        return cmd_status()
    if args.cmd == "ingest":
        return cmd_ingest()
    if args.cmd == "summarize":
        return cmd_summarize()
    if args.cmd == "validate":
        return cmd_validate()
    if args.cmd == "record":
        return cmd_record(args)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
