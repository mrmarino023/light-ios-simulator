#!/usr/bin/env bash
# L3 sealed blind TRAIL — held-out tasks, agent sees task.json only.
#
# Pass: ≥2/3 verified with wall ≤ budget (default 120s).
#
# Usage:
#   ./scripts/gate-trail-l3.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="${LIGH_L3_MANIFEST:-$ROOT/fixtures/frozen/l3-sealed/manifest.json}"
OUT="${LIGH_TRAIL_L3_OUT:-$ROOT/docs/assets/trail-l3-latest.json}"
BUDGET="${LIGH_TRAIL_WALL_MS:-120000}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -f "$MANIFEST" ]] || fail "missing manifest $MANIFEST"

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

echo "══ TRAIL L3 sealed ══"
python3 - "$MANIFEST" "$BUDGET" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
print(f"  pack={m['pack_id']} budget={sys.argv[2]}ms tasks={len(m['tasks'])}")
PY

export LIGH_TRAIL_REUSE_SESSION=0
rows=()
pass_n=0
holy_n=0
run_n=0

TASK_PATHS=$(python3 - "$MANIFEST" "$ROOT" <<'PY'
import json, os, sys
m = json.load(open(sys.argv[1]))
root = sys.argv[2]
for t in m["tasks"]:
    p = t["task_path"]
    if not os.path.isabs(p):
        p = os.path.join(root, p)
    print(t["id"], p)
PY
)

while read -r tid TASK; do
  [[ -n "$tid" ]] || continue
  run_n=$((run_n + 1))
  TASK_OUT="/tmp/trail-l3-${tid}.json"
  echo "  ── $tid"
  export LIGH_TRAIL_REUSE_SESSION=0
  # Fresh daemon per task — Kix/XCUITestDemo switch thrashes sim if reused.
  "$ROOT/target/release/ligh" daemon stop --json >/dev/null 2>&1 || true
  pkill -x lighd 2>/dev/null || true
  sleep 0.5
  rm -f "${HOME}/.ligh/lighd.sock"
  if [[ -x "$ROOT/target/release/lighd" ]]; then
    nohup env LIGH_UI=sim "$ROOT/target/release/lighd" >>/tmp/lighd-trail-l3.log 2>&1 &
    sleep 1.2
  fi
  if LIGH_TRAIL_TASK="$TASK" LIGH_TRAIL_HOLY_OUT="$TASK_OUT" LIGH_TRAIL_WALL_MS="$BUDGET" \
    "$ROOT/scripts/gate-trail-holy.sh" >/tmp/trail-l3-multi-"$tid".log 2>&1; then
    pass_n=$((pass_n + 1))
    holy_n=$((holy_n + 1))
    rows+=("$tid")
  else
    echo "    ⚠ fail — /tmp/trail-l3-multi-${tid}.log"
    [[ -f "$TASK_OUT" ]] && rows+=("$tid")
    if [[ -f "$TASK_OUT" ]] && python3 -c "import json; d=json.load(open('$TASK_OUT')); exit(0 if d.get('verified') else 1)"; then
      pass_n=$((pass_n + 1))
    fi
  fi
done <<< "$TASK_PATHS"

python3 - "$OUT" "$MANIFEST" "$BUDGET" "$pass_n" "$holy_n" "$run_n" "${rows[@]+"${rows[@]}"}" <<'PY'
import json, os, sys

out, manifest_path, budget = sys.argv[1], sys.argv[2], int(sys.argv[3])
pass_n, holy_n, run_n = int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6])
rows = sys.argv[7:]
manifest = json.load(open(manifest_path))
need = int(manifest.get("claim_need") or 2)

task_rows = []
for tid in rows:
    p = f"/tmp/trail-l3-{tid}.json"
    if not os.path.isfile(p):
        continue
    d = json.load(open(p))
    task_rows.append({
        "task": tid,
        "verified": d.get("verified"),
        "holy_shit": d.get("holy_shit"),
        "wall_ms": d.get("wall_ms"),
        "mode": d.get("mode"),
        "primary_path": d.get("primary_path"),
        "llm_tokens": d.get("llm_tokens"),
        "operator": (d.get("fix_attempts") or [{}])[0].get("method"),
        "within_budget": d.get("within_budget"),
        "reason": d.get("reason"),
    })

verified = sum(1 for r in task_rows if r.get("verified"))
holy = sum(1 for r in task_rows if r.get("holy_shit"))
claim = verified >= need and run_n >= 2

doc = {
    "gate": "trail_l3_sealed",
    "pack_id": manifest.get("pack_id"),
    "wall_budget_ms": budget,
    "tasks_run": run_n,
    "tasks_verified": verified,
    "tasks_holy_shit": holy,
    "claim_pass": claim,
    "l3_generalized": claim and holy >= need,
    "tasks": task_rows,
}
os.makedirs(os.path.dirname(out), exist_ok=True)
json.dump(doc, open(out, "w"), indent=2)
open(out, "a").write("\n")
print(json.dumps({
    "claim_pass": claim,
    "l3_generalized": doc["l3_generalized"],
    "verified": verified,
    "holy": holy,
    "run": run_n,
    "need": need,
}, indent=2))
sys.exit(0 if claim else 1)
PY

echo "✓ trail_l3 → $OUT"
