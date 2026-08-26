#!/usr/bin/env bash
# Speculate / Scene ablation — Meta-style A/B on the SAME frozen goal.
#
# H0: LIGH_SPECULATE=0 (classic settle) matches on=1 for claim metrics.
# H1: speculate on reduces elapsed_ms and/or steps without hurting reached.
#
# Does NOT invent wins. Publishes raw cells + summary JSON.
#
# Usage:
#   ./scripts/gate-speculate-ablation.sh
#   LIGH_ABLATE_N=3 LIGH_ABLATE_APP=... LIGH_ABLATE_BUNDLE=... ./scripts/gate-speculate-ablation.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_ABLATE_N:-2}"
OUT="${LIGH_ABLATE_OUT:-$ROOT/docs/assets/speculate-ablation-latest.json}"
TASK="${LIGH_ABLATE_TASK:-$ROOT/fixtures/frozen/tasks/login-never-navigates/task.json}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "missing $LIGH — cargo build --release -p ligh-cli -p ligh-daemon"
[[ -f "$TASK" ]] || fail "missing task $TASK"

APP=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['app_path']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
BID=$(python3 -c "import json; print(json.load(open('$TASK'))['bundle_id'])")
GOAL=$(python3 - <<PY
import json, sys
sys.path.insert(0, "$ROOT/scripts")
from goal_spec import compile_task_goal
print(json.dumps(compile_task_goal(json.load(open("$TASK")))))
PY
)

export LIGH_UI="${LIGH_UI:-sim}"
if [[ "$LIGH_UI" == "sim" || "$LIGH_UI" == "simulator" ]]; then
  unset LIGH_WDA_UDID LIGH_WDA_BUNDLE LIGH_WDA_URL LIGH_WDA_SESSION || true
fi

"$LIGH" daemon stop --json >/dev/null 2>&1 || true
pkill -x lighd 2>/dev/null || true
sleep 1
nohup env -u LIGH_WDA_UDID -u LIGH_WDA_BUNDLE LIGH_UI=sim \
  "$ROOT/target/release/lighd" >>/tmp/lighd-ablate.log 2>&1 &
sleep 2
"$LIGH" up --device iphone-15-pro >/dev/null 2>&1 || "$LIGH" up --device iphone-15-pro

CELLS=()
for mode in off on; do
  if [[ "$mode" == "off" ]]; then
    export LIGH_SPECULATE=0
  else
    export LIGH_SPECULATE=1
  fi
  # Restart daemon so env is inherited by the process that runs autopilot.
  "$LIGH" daemon stop --json >/dev/null 2>&1 || true
  pkill -x lighd 2>/dev/null || true
  sleep 1
  nohup env -u LIGH_WDA_UDID -u LIGH_WDA_BUNDLE LIGH_UI=sim LIGH_SPECULATE="$LIGH_SPECULATE" \
    "$ROOT/target/release/lighd" >>/tmp/lighd-ablate.log 2>&1 &
  sleep 2
  # Wait until RPC accepts — otherwise off/on cells measure daemon death, not speculation.
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if "$LIGH" --json ready --settle-ms 400 >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done

  for i in $(seq 1 "$N"); do
    echo "── mode=$mode run=$i/$N"
    CELL=$(mktemp)
    if "$LIGH" --json cap autopilot \
      --app "$APP" \
      --bundle-id "$BID" \
      --goal-spec "$GOAL" \
      --max-steps 24 \
      --settle-ms 1500 \
      --timeout-ms 8000 \
      >"$CELL" 2>/tmp/ablate-"$mode"-"$i".err; then
      :
    fi
    python3 - "$CELL" "$mode" "$i" <<'PY'
import json, sys
path, mode, i = sys.argv[1:4]
try:
    d = json.load(open(path))
except Exception as e:
    print(json.dumps({"mode": mode, "i": int(i), "ok": False, "error": str(e)}))
    raise SystemExit(0)
detail = d.get("detail") or d
stats = detail.get("speculate_stats") or {}
print(json.dumps({
    "mode": mode,
    "i": int(i),
    "ok": bool(d.get("ok") or detail.get("reached")),
    "reached": bool(detail.get("reached")),
    "elapsed_ms": detail.get("elapsed_ms"),
    "steps": detail.get("steps"),
    "speculate_enabled": detail.get("speculate_enabled"),
    "speculate_stats": stats,
    "fault": d.get("fault") or detail.get("fault"),
}))
PY
    CELLS+=("$(python3 - "$CELL" "$mode" "$i" <<'PY'
import json, sys
path, mode, i = sys.argv[1:4]
try:
    d = json.load(open(path))
except Exception as e:
    print(json.dumps({"mode": mode, "i": int(i), "ok": False, "error": str(e)}))
    raise SystemExit(0)
detail = d.get("detail") or d
stats = detail.get("speculate_stats") or {}
print(json.dumps({
    "mode": mode,
    "i": int(i),
    "ok": bool(d.get("ok") or detail.get("reached")),
    "reached": bool(detail.get("reached")),
    "elapsed_ms": detail.get("elapsed_ms"),
    "steps": detail.get("steps"),
    "speculate_enabled": detail.get("speculate_enabled"),
    "speculate_stats": stats,
    "fault": d.get("fault") or detail.get("fault"),
}))
PY
)")
    rm -f "$CELL"
  done
done

python3 - "$OUT" "$TASK" "$N" "${CELLS[@]}" <<'PY'
import json, statistics, sys
out, task, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
cells = [json.loads(c) for c in sys.argv[4:]]

def arm(mode):
    xs = [c for c in cells if c.get("mode") == mode]
    walls = [c["elapsed_ms"] for c in xs if isinstance(c.get("elapsed_ms"), (int, float))]
    steps = [c["steps"] for c in xs if isinstance(c.get("steps"), (int, float))]
    reached = sum(1 for c in xs if c.get("reached"))
    begins = sum((c.get("speculate_stats") or {}).get("begins") or 0 for c in xs)
    certified = sum((c.get("speculate_stats") or {}).get("certified") or 0 for c in xs)
    rejected = sum((c.get("speculate_stats") or {}).get("rejected") or 0 for c in xs)
    fires = sum((c.get("speculate_stats") or {}).get("fire_preplanned") or 0 for c in xs)
    return {
        "n": len(xs),
        "reached": reached,
        "reach_rate": reached / len(xs) if xs else 0.0,
        "median_elapsed_ms": statistics.median(walls) if walls else None,
        "median_steps": statistics.median(steps) if steps else None,
        "speculate_begins": begins,
        "speculate_certified": certified,
        "speculate_rejected": rejected,
        "fire_preplanned": fires,
        "certify_rate": (certified / (certified + rejected)) if (certified + rejected) else None,
        "cells": xs,
    }

off, on = arm("off"), arm("on")
speedup = None
if off.get("median_elapsed_ms") and on.get("median_elapsed_ms") and on["median_elapsed_ms"] > 0:
    speedup = off["median_elapsed_ms"] / on["median_elapsed_ms"]

inconclusive = (
    off.get("median_elapsed_ms") is None
    or on.get("median_elapsed_ms") is None
    or off["n"] == 0
    or on["n"] == 0
)

# Meta claim: only assert help if both arms measured AND reach not worse AND
# (faster OR certified preplan overlap).
helps = (
    not inconclusive
    and on["reach_rate"] >= off["reach_rate"]
    and (
        (speedup is not None and speedup >= 1.15)
        or (on.get("fire_preplanned", 0) > 0 and (on.get("certify_rate") or 0) >= 0.5 and on["reach_rate"] >= 1.0)
    )
)

doc = {
    "gate": "speculate_ablation",
    "protocol": "H0=LIGH_SPECULATE=0 vs H1=on; same task/goal/app; quarantine LIGH_UI=sim",
    "task": task,
    "n_per_arm": n,
    "off": off,
    "on": on,
    "speedup_on_vs_off": speedup,
    "verdict": {
        "speculate_helps": helps,
        "inconclusive": inconclusive,
        "null_not_rejected": (not helps) and (not inconclusive),
        "note": "Do not claim product holy-shit from this gate alone — only whether optimism earns latency/overlap on an already-reachable goal. Inconclusive = daemon/infra spoiled a cell.",
    },
}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({
    "out": out,
    "speculate_helps": helps,
    "inconclusive": inconclusive,
    "speedup_on_vs_off": speedup,
    "off_reach": off["reach_rate"],
    "on_reach": on["reach_rate"],
    "off_ms": off["median_elapsed_ms"],
    "on_ms": on["median_elapsed_ms"],
    "on_certify_rate": on.get("certify_rate"),
    "on_fire_preplanned": on.get("fire_preplanned"),
}, indent=2))
if inconclusive:
    raise SystemExit(3)
raise SystemExit(0 if helps else 2)
PY
