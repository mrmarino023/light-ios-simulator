#!/usr/bin/env bash
# Multi-task TRAIL gate — R1–R3 on frozen tasks (no golden diff).
#
# Pass: ≥2/3 tasks localize within prove budget.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASKS="${LIGH_TRAIL_TASKS:-kix-notes-tab-missing login-never-navigates onboarding-home-broken}"
OUT="${LIGH_TRAIL_MULTI_OUT:-$ROOT/docs/assets/trail-multi-latest.json}"
PROVE_BUDGET="${LIGH_TRAIL_PROVE_MS:-45000}"

fail() { echo "✗ $*" >&2; exit 1; }

echo "══ TRAIL multi (trace + localize) ══"
echo "  tasks=$TASKS prove_budget=${PROVE_BUDGET}ms"

export LIGH_TRAIL_HOT=1
rows=()
pass_n=0
run_n=0

for tid in $TASKS; do
  TASK="$ROOT/fixtures/frozen/tasks/$tid/task.json"
  [[ -f "$TASK" ]] || { echo "  ⚠ skip missing $tid"; continue; }
  run_n=$((run_n + 1))
  TASK_OUT="/tmp/trail-${tid}.json"
  echo "  ── $tid"
  export LIGH_TRAIL_TASK="$TASK"
  export LIGH_TRAIL_OUT="$TASK_OUT"
  if LIGH_TRAIL_PROVE_MS="$PROVE_BUDGET" "$ROOT/scripts/gate-trail.sh" >/tmp/trail-multi-"$tid".log 2>&1; then
    pass_n=$((pass_n + 1))
    rows+=("$tid")
  else
    echo "    ⚠ failed — /tmp/trail-multi-${tid}.log"
    [[ -f "$TASK_OUT" ]] && rows+=("$tid")
  fi
done

python3 - "$OUT" "$PROVE_BUDGET" "$pass_n" "$run_n" "${rows[@]+"${rows[@]}"}" <<'PY'
import json, os, sys

out, budget, pass_n, run_n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
ids = sys.argv[5:]
rows = []
for tid in ids:
    p = f"/tmp/trail-{tid}.json"
    if not os.path.isfile(p):
        continue
    d = json.load(open(p))
    rows.append({
        "task": tid,
        "localization_ok": d.get("localization_ok"),
        "trail_wall_ms": d.get("trail_wall_ms"),
        "within_prove_budget": d.get("within_prove_budget"),
        "mode": (d.get("repair_bundle") or {}).get("mode"),
        "primary_path": (d.get("repair_bundle") or {}).get("scope", {}).get("primary_path"),
        "failed_identity": d.get("failed_identity"),
        "verified": d.get("verified"),
    })

need = max(2, (run_n + 1) // 2) if run_n else 2
claim = pass_n >= need and run_n >= 2
doc = {
    "gate": "trail_multi",
    "architecture": "trace_repair_autopilot_identity_localization",
    "prove_budget_ms": budget,
    "tasks_run": run_n,
    "tasks_localized": pass_n,
    "claim_pass": claim,
    "tasks": rows,
}
json.dump(doc, open(out, "w"), indent=2)
print(json.dumps({"claim_pass": claim, "localized": pass_n, "run": run_n, "need": need}, indent=2))
PY

python3 -c "import json; d=json.load(open('$OUT')); exit(0 if d.get('claim_pass') else 1)" \
  && echo "✓ trail_multi → $OUT" \
  || fail "trail_multi claim failed — see $OUT"
