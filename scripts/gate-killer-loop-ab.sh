#!/usr/bin/env bash
# Killer loop A/B — same frozen task, same bug, LIGH vs vision baseline (+ optional hybrid).
#
# Honest mode (LIGH_KILLER_HONEST=1):
#   - Default task: login-never-navigates (XCUITestDemo)
#   - No host exercise_app; agent must drive UI
#   - Publishes docs/assets/killer-loop-ab-honest-latest.json
#
# Product mode (default):
#   - Default task: onboarding-home-broken
#   - Publishes docs/assets/killer-loop-ab-latest.json
#   - Set LIGH_KILLER_AB_HYBRID=1 for optional hybrid arm
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HONEST="${LIGH_KILLER_HONEST:-0}"
INCLUDE_HYBRID="${LIGH_KILLER_AB_HYBRID:-0}"

if [[ "$HONEST" == "1" ]]; then
  TASK="${LIGH_KILLER_TASK:-$ROOT/fixtures/frozen/tasks/login-never-navigates/task.json}"
  OUT="${LIGH_KILLER_AB_OUT:-$ROOT/docs/assets/killer-loop-ab-honest-latest.json}"
  INCLUDE_HYBRID=0
else
  TASK="${LIGH_KILLER_TASK:-$ROOT/fixtures/frozen/tasks/onboarding-home-broken/task.json}"
  OUT="${LIGH_KILLER_AB_OUT:-$ROOT/docs/assets/killer-loop-ab-latest.json}"
fi

fail() { echo "✗ $*" >&2; exit 1; }

echo "══ Killer loop A/B ══"
echo "  protocol=$([ "$HONEST" = 1 ] && echo honest || echo product)"
echo "  task=$TASK"
echo "  Run A: LIGH (perceive/attempt)"
echo "  Run B: baseline (screenshot/vision)"
if [[ "$INCLUDE_HYBRID" == "1" ]]; then
  echo "  Run C: hybrid (routed perceive → vision on escalation)"
fi

if [[ "$HONEST" == "1" ]]; then
  LIGH_OUT="$ROOT/docs/assets/killer-loop-honest-ligh-latest.json"
  BASE_OUT="$ROOT/docs/assets/killer-loop-honest-baseline-latest.json"
else
  LIGH_OUT="$ROOT/docs/assets/killer-loop-ligh-latest.json"
  BASE_OUT="$ROOT/docs/assets/killer-loop-baseline-latest.json"
fi
HYBRID_OUT="$ROOT/docs/assets/killer-loop-hybrid-latest.json"

export LIGH_KILLER_HONEST="$HONEST"

LIGH_PASS=0
LIGH_KILLER_ARM=ligh LIGH_KILLER_OUT="$LIGH_OUT" LIGH_KILLER_TASK="$TASK" \
  "$ROOT/scripts/gate-killer-loop.sh" && LIGH_PASS=1 || true

BASE_PASS=0
LIGH_KILLER_ARM=baseline LIGH_KILLER_OUT="$BASE_OUT" LIGH_KILLER_TASK="$TASK" \
  "$ROOT/scripts/gate-killer-loop.sh" && BASE_PASS=1 || true

HYBRID_PASS=0
if [[ "$INCLUDE_HYBRID" == "1" ]]; then
  LIGH_KILLER_ARM=hybrid LIGH_KILLER_OUT="$HYBRID_OUT" LIGH_KILLER_TASK="$TASK" \
    "$ROOT/scripts/gate-killer-loop.sh" && HYBRID_PASS=1 || true
fi

python3 - "$OUT" "$LIGH_OUT" "$BASE_OUT" "$HYBRID_OUT" "$LIGH_PASS" "$BASE_PASS" "$HYBRID_PASS" "$INCLUDE_HYBRID" "$TASK" "$HONEST" <<'PY'
import json, sys

out, ligh_path, base_path, hybrid_path, ligh_pass, base_pass, hybrid_pass, include_hybrid, task, honest = sys.argv[1:11]
ligh_pass, base_pass, hybrid_pass = int(ligh_pass), int(base_pass), int(hybrid_pass)
include_hybrid = include_hybrid == "1"
honest = honest == "1"
ligh = json.load(open(ligh_path)) if __import__("os").path.isfile(ligh_path) else {}
base = json.load(open(base_path)) if __import__("os").path.isfile(base_path) else {}
hybrid = json.load(open(hybrid_path)) if include_hybrid and __import__("os").path.isfile(hybrid_path) else {}

def arm_summary(d):
    actions = d.get("agent_actions") or []
    used_exercise = any((a.get("action") == "exercise_app") for a in actions)
    return {
        "verified": d.get("verified"),
        "claim_pass": d.get("claim_pass"),
        "false_success": d.get("false_success"),
        "legacy_weak_pass": d.get("legacy_weak_pass"),
        "verification_reason": d.get("verification_reason"),
        "final_state": d.get("final_state"),
        "build_attempts": d.get("build_attempts"),
        "verification_attempts": d.get("verification_attempts"),
        "perception_channels": d.get("perception_channels"),
        "used_exercise_app": used_exercise,
        "faults": len(d.get("faults") or []),
        "llm_tokens": d.get("llm_tokens"),
        "wall_time_ms": d.get("wall_time_ms"),
        "human_interventions": d.get("human_interventions", 0),
        "protocol": d.get("protocol"),
    }

runs = {
    "ligh": {"pass": ligh_pass == 1, **arm_summary(ligh), "artifact": ligh_path},
    "baseline": {"pass": base_pass == 1, **arm_summary(base), "artifact": base_path},
}
if include_hybrid:
    runs["hybrid"] = {"pass": hybrid_pass == 1, **arm_summary(hybrid), "artifact": hybrid_path}

report = {
    "gate": "killer_loop_ab_honest" if honest else "killer_loop_ab",
    "protocol": "honest" if honest else "product",
    "protocol_version": json.load(open(task)).get("protocol_version", 1),
    "task": json.load(open(task)).get("id"),
    "task_file": task,
    "honest_rules": {
        "exercise_app_disabled": honest,
        "shared_minimal_prompt": honest,
        "agent_must_drive_ui": honest,
    } if honest else None,
    "runs": runs,
    "interpretation": (
        "Honest A/B: same task/prompt; only AX vs vision modality differs. Publish even if both fail."
        if honest
        else "Publish even if all pass or all fail — comparison is the signal."
    ),
}
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(0)
PY

echo "══ → $OUT"
