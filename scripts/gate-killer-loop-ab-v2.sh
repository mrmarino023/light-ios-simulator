#!/usr/bin/env bash
# Killer loop A/B v2 — host-driven UI (autopilot) vs vision, same task and verifier.
#
# The claim under test: moving UI execution out of the LLM and into the host makes the
# whole autonomous loop ≥3× faster than vision, and passes.
#
# What is held identical between arms:
#   - task, injected bug, strict harness, model, step budget
#   - failure nudges (modality-neutral under the honest protocol)
#   - the acceptance target (success criterion), given to both
# What differs: arm A gets `run_goal` (host discovers the path, zero tokens); arm B gets
# screenshot + vision taps and must drive every interaction through the LLM.
#
# Neither arm receives a step list. That was the cheat in the earlier benchmark.
#
# Usage:
#   ./scripts/gate-killer-loop-ab-v2.sh
#   LIGH_AB2_REPEAT=3 ./scripts/gate-killer-loop-ab-v2.sh    # measure top-1 patch acceptance
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK="${LIGH_KILLER_TASK:-$ROOT/fixtures/frozen/tasks/login-never-navigates/task.json}"
OUT="${LIGH_AB2_OUT:-$ROOT/docs/assets/killer-loop-ab-v2-latest.json}"
RUN_DIR="${LIGH_AB2_RUNS:-$ROOT/docs/assets/killer-loop-ab-v2-runs}"
REPEAT="${LIGH_AB2_REPEAT:-1}"
SPEEDUP_TARGET="${LIGH_AB2_SPEEDUP:-3.0}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -f "$TASK" ]] || fail "missing task $TASK"
mkdir -p "$RUN_DIR" "$(dirname "$OUT")"

echo "══ Killer loop A/B v2 ══"
echo "  task=$(basename "$(dirname "$TASK")")   repeats=$REPEAT   target=${SPEEDUP_TARGET}× vs vision"
echo "  Arm A: autopilot — LLM owns code, host owns UI (run_goal)"
echo "  Arm B: baseline  — LLM owns code and every tap (screenshot + vision)"
echo ""

# Honest protocol for both arms: no host exercise shortcut, neutral nudges.
export LIGH_KILLER_HONEST=1

for i in $(seq 1 "$REPEAT"); do
  for arm in autopilot baseline; do
    echo "── run $i/$REPEAT · arm=$arm"
    ARM_OUT="$RUN_DIR/${arm}-run${i}.json"
    LIGH_KILLER_ARM="$arm" LIGH_KILLER_OUT="$ARM_OUT" LIGH_KILLER_TASK="$TASK" \
      "$ROOT/scripts/gate-killer-loop.sh" >"/tmp/ab2-${arm}-${i}.log" 2>&1 || true
    if [[ -f "$ARM_OUT" ]]; then
      python3 - "$ARM_OUT" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
pe = d.get("patch_economics") or {}
mark = "✓" if d.get("claim_pass") else "✗"
print(f"   {mark} {d.get('arm')}: {d.get('wall_time_ms')}ms  {d.get('llm_tokens')} tok  "
      f"builds={d.get('build_attempts')}  patches={pe.get('patch_attempts')}  "
      f"accepted_at={pe.get('accepted_at_patch')}  reason={d.get('verification_reason')}")
PY
    else
      echo "   ✗ $arm: no artifact (see /tmp/ab2-${arm}-${i}.log)"
    fi
  done
done

python3 - "$OUT" "$RUN_DIR" "$TASK" "$REPEAT" "$SPEEDUP_TARGET" <<'PY'
import json, os, statistics, sys

out, run_dir, task, repeat, target = sys.argv[1:6]
repeat, target = int(repeat), float(target)
task_doc = json.load(open(task))


def load(arm):
    docs = []
    for i in range(1, repeat + 1):
        p = os.path.join(run_dir, f"{arm}-run{i}.json")
        if os.path.isfile(p):
            docs.append(json.load(open(p)))
    return docs


def median(vals):
    vals = [v for v in vals if isinstance(v, (int, float))]
    return statistics.median(vals) if vals else None


def arm_report(arm, docs):
    if not docs:
        return {"runs": 0}
    pe = [d.get("patch_economics") or {} for d in docs]
    top1 = [p.get("top1_accepted") for p in pe if p.get("accepted_at_patch") is not None]
    used_ui = any(
        a.get("action") in ("perceive", "attempt", "screenshot", "vision_tap", "vision_type")
        for d in docs
        for a in (d.get("agent_actions") or [])
    )
    return {
        "runs": len(docs),
        "passes": sum(1 for d in docs if d.get("claim_pass")),
        "median_wall_ms": median([d.get("wall_time_ms") for d in docs]),
        "median_tokens": median([d.get("llm_tokens") for d in docs]),
        "median_builds": median([d.get("build_attempts") for d in docs]),
        "median_patches": median([p.get("patch_attempts") for p in pe]),
        # Speculation decision variable: low top-1 acceptance means proposing k patches
        # per LLM call and building them in parallel is strictly cheaper.
        "top1_acceptance": (sum(1 for t in top1 if t) / len(top1)) if top1 else None,
        "accepted_at_patch": [p.get("accepted_at_patch") for p in pe],
        "llm_drove_ui": used_ui,
        "false_success": any(d.get("false_success") for d in docs),
        "reasons": [d.get("verification_reason") for d in docs],
    }


auto = arm_report("autopilot", load("autopilot"))
base = arm_report("baseline", load("baseline"))

speedup = None
token_ratio = None
if auto.get("median_wall_ms") and base.get("median_wall_ms"):
    speedup = round(base["median_wall_ms"] / auto["median_wall_ms"], 2)
if auto.get("median_tokens") and base.get("median_tokens"):
    token_ratio = round(base["median_tokens"] / auto["median_tokens"], 2)

# The full claim needs both halves: the host must be faster AND the loop must pass.
# A faster arm that fails is not a win, and we publish it as a loss.
autopilot_passes = auto.get("runs", 0) > 0 and auto.get("passes", 0) == auto.get("runs")
claim_pass = bool(autopilot_passes and speedup is not None and speedup >= target)

report = {
    "gate": "killer_loop_ab_v2",
    "claim": f"Host-driven UI makes the autonomous loop ≥{target}× faster than vision and passes",
    "protocol": "honest",
    "protocol_version": task_doc.get("protocol_version", 2),
    "task": task_doc.get("id"),
    "task_file": task,
    "held_identical": [
        "task + injected bug",
        "strict harness verifier",
        "model + step budget",
        "failure nudges (modality-neutral)",
        "acceptance target given to both arms",
    ],
    "only_difference": "arm A: host discovers and drives the path (0 tokens). arm B: LLM drives every tap via vision.",
    "arms": {"autopilot": auto, "baseline": base},
    "speedup_vs_vision": speedup,
    "token_ratio_vs_vision": token_ratio,
    "speedup_target": target,
    "verdict": {
        "autopilot_passed_all": autopilot_passes,
        "speedup_met": speedup is not None and speedup >= target,
        "autopilot_never_drove_ui": not auto.get("llm_drove_ui", True),
        "no_false_success": not (auto.get("false_success") or base.get("false_success")),
    },
    "claim_pass": claim_pass,
    "interpretation": (
        "Published regardless of outcome. Speedup without a pass is not a win; "
        "top1_acceptance decides whether speculative multi-patch turns are worth building."
    ),
}
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print("")
print(json.dumps(report, indent=2))
raise SystemExit(0 if claim_pass else 1)
PY

echo "══ → $OUT"
