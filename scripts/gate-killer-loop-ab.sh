#!/usr/bin/env bash
# Killer loop A/B — same frozen task, same bug, LIGH vs vision baseline (+ optional hybrid).
#
# Publishes docs/assets/killer-loop-ab-latest.json
# Set LIGH_KILLER_AB_HYBRID=1 to include the AX-first hybrid arm (3rd run).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK="${LIGH_KILLER_TASK:-$ROOT/fixtures/frozen/tasks/onboarding-home-broken/task.json}"
OUT="${LIGH_KILLER_AB_OUT:-$ROOT/docs/assets/killer-loop-ab-latest.json}"
INCLUDE_HYBRID="${LIGH_KILLER_AB_HYBRID:-0}"

fail() { echo "✗ $*" >&2; exit 1; }

echo "══ Killer loop A/B ══"
echo "  task=$TASK"
echo "  Run A: LIGH (perceive/attempt)"
echo "  Run B: baseline (screenshot/vision)"
if [[ "$INCLUDE_HYBRID" == "1" ]]; then
  echo "  Run C: hybrid (routed perceive → vision on escalation)"
fi

LIGH_OUT="$ROOT/docs/assets/killer-loop-ligh-latest.json"
BASE_OUT="$ROOT/docs/assets/killer-loop-baseline-latest.json"
HYBRID_OUT="$ROOT/docs/assets/killer-loop-hybrid-latest.json"

LIGH_PASS=0
LIGH_KILLER_ARM=ligh LIGH_KILLER_OUT="$LIGH_OUT" "$ROOT/scripts/gate-killer-loop.sh" && LIGH_PASS=1 || true

BASE_PASS=0
LIGH_KILLER_ARM=baseline LIGH_KILLER_OUT="$BASE_OUT" "$ROOT/scripts/gate-killer-loop.sh" && BASE_PASS=1 || true

HYBRID_PASS=0
if [[ "$INCLUDE_HYBRID" == "1" ]]; then
  LIGH_KILLER_ARM=hybrid LIGH_KILLER_OUT="$HYBRID_OUT" "$ROOT/scripts/gate-killer-loop.sh" && HYBRID_PASS=1 || true
fi

python3 - "$OUT" "$LIGH_OUT" "$BASE_OUT" "$HYBRID_OUT" "$LIGH_PASS" "$BASE_PASS" "$HYBRID_PASS" "$INCLUDE_HYBRID" "$TASK" <<'PY'
import json, sys

out, ligh_path, base_path, hybrid_path, ligh_pass, base_pass, hybrid_pass, include_hybrid, task = sys.argv[1:10]
ligh_pass, base_pass, hybrid_pass = int(ligh_pass), int(base_pass), int(hybrid_pass)
include_hybrid = include_hybrid == "1"
ligh = json.load(open(ligh_path)) if __import__("os").path.isfile(ligh_path) else {}
base = json.load(open(base_path)) if __import__("os").path.isfile(base_path) else {}
hybrid = json.load(open(hybrid_path)) if include_hybrid and __import__("os").path.isfile(hybrid_path) else {}

def arm_summary(d):
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
        "faults": len(d.get("faults") or []),
        "llm_tokens": d.get("llm_tokens"),
        "wall_time_ms": d.get("wall_time_ms"),
        "human_interventions": d.get("human_interventions", 0),
    }

runs = {
    "ligh": {"pass": ligh_pass == 1, **arm_summary(ligh), "artifact": ligh_path},
    "baseline": {"pass": base_pass == 1, **arm_summary(base), "artifact": base_path},
}
if include_hybrid:
    runs["hybrid"] = {"pass": hybrid_pass == 1, **arm_summary(hybrid), "artifact": hybrid_path}

report = {
    "gate": "killer_loop_ab",
    "protocol_version": json.load(open(task)).get("protocol_version", 1),
    "task": json.load(open(task)).get("id"),
    "task_file": task,
    "runs": runs,
    "interpretation": "Publish even if all pass or all fail — comparison is the signal.",
}
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(0)
PY

echo "══ → $OUT"
