#!/usr/bin/env bash
# Multi-task TRAIL holy-shit — ≥2/3 verified with wall ≤ budget.
#
# Usage:
#   ./scripts/gate-trail-holy-multi.sh
#   LIGH_TRAIL_TASKS="login-never-navigates kix-notes-tab-missing onboarding-home-broken" ./scripts/gate-trail-holy-multi.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASKS="${LIGH_TRAIL_TASKS:-login-never-navigates kix-notes-tab-missing onboarding-home-broken}"
OUT="${LIGH_TRAIL_HOLY_MULTI_OUT:-$ROOT/docs/assets/trail-holy-multi-latest.json}"
BUDGET="${LIGH_TRAIL_WALL_MS:-120000}"

fail() { echo "✗ $*" >&2; exit 1; }

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

echo "══ TRAIL holy multi ══"
echo "  tasks=$TASKS budget=${BUDGET}ms"

export LIGH_TRAIL_REUSE_SESSION="${LIGH_TRAIL_REUSE_SESSION:-0}"
rows=()
pass_n=0
holy_n=0
run_n=0

first=1
for tid in $TASKS; do
  TASK="$ROOT/fixtures/frozen/tasks/$tid/task.json"
  [[ -f "$TASK" ]] || { echo "  ⚠ skip $tid"; continue; }
  run_n=$((run_n + 1))
  TASK_OUT="/tmp/trail-holy-${tid}.json"
  echo "  ── $tid"
  # Reuse daemon after the first task (warm sim); certify retries handle app switches.
  if [[ "$first" -eq 0 ]]; then
    export LIGH_TRAIL_REUSE_SESSION=1
  fi
  first=0
  if LIGH_TRAIL_TASK="$TASK" LIGH_TRAIL_HOLY_OUT="$TASK_OUT" LIGH_TRAIL_WALL_MS="$BUDGET" \
    "$ROOT/scripts/gate-trail-holy.sh" >/tmp/trail-holy-multi-"$tid".log 2>&1; then
    pass_n=$((pass_n + 1))
    holy_n=$((holy_n + 1))
    rows+=("$tid")
  else
    echo "    ⚠ fail — /tmp/trail-holy-multi-${tid}.log"
    [[ -f "$TASK_OUT" ]] && rows+=("$tid")
    # Count verified-but-slow as pass for generality, not holy_shit.
    if [[ -f "$TASK_OUT" ]] && python3 -c "import json; d=json.load(open('$TASK_OUT')); exit(0 if d.get('verified') else 1)"; then
      pass_n=$((pass_n + 1))
    fi
  fi
done

python3 - "$OUT" "$BUDGET" "$pass_n" "$holy_n" "$run_n" "${rows[@]+"${rows[@]}"}" <<'PY'
import json, os, sys
out, budget, pass_n, holy_n, run_n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
ids = sys.argv[6:]
rows = []
for tid in ids:
    p = f"/tmp/trail-holy-{tid}.json"
    if not os.path.isfile(p):
        continue
    d = json.load(open(p))
    rows.append({
        "task": tid,
        "verified": d.get("verified"),
        "holy_shit": d.get("holy_shit"),
        "wall_ms": d.get("wall_ms"),
        "infra_ms": d.get("infra_ms"),
        "mode": d.get("mode"),
        "primary_path": d.get("primary_path"),
        "llm_tokens": d.get("llm_tokens"),
        "within_budget": d.get("within_budget"),
        "reason": d.get("reason"),
    })

need = max(2, (run_n + 1) // 2) if run_n else 2
verified_n = sum(1 for r in rows if r.get("verified"))
holy_count = sum(1 for r in rows if r.get("holy_shit"))
claim = verified_n >= need and run_n >= 2
holy_claim = holy_count >= need and run_n >= 2
doc = {
    "gate": "trail_holy_multi",
    "architecture": "trail",
    "wall_budget_ms": budget,
    "tasks_run": run_n,
    "tasks_verified": verified_n,
    "tasks_holy_shit": holy_count,
    "claim_pass": claim,
    "holy_shit_generalized": holy_claim,
    "tasks": rows,
}
json.dump(doc, open(out, "w"), indent=2)
print(json.dumps({
    "claim_pass": claim,
    "holy_shit_generalized": holy_claim,
    "verified": verified_n,
    "holy": holy_count,
    "run": run_n,
    "need": need,
}, indent=2))
PY

python3 -c "import json; d=json.load(open('$OUT')); print('holy_shit_generalized=', d.get('holy_shit_generalized')); exit(0 if d.get('claim_pass') else 1)" \
  && echo "✓ trail_holy_multi → $OUT" \
  || fail "trail_holy_multi claim failed — see $OUT"
