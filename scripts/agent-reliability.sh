#!/usr/bin/env bash
# Reliability harness: run a named workload N times; print fail rate.
#
# Usage:
#   ./scripts/agent-reliability.sh [N] [workload]
#   N default 10; workload: settings | messages | both
#
# Requires: lighd + `ligh up` session. Does not boot the sim.
#
# Example (local, before claiming reliability):
#   ligh daemon start && ligh up
#   ./scripts/agent-reliability.sh 50 both
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${1:-10}"
WORKLOAD="${2:-settings}"
OUT_DIR="${LIGH_RELIABILITY_OUT:-/tmp/ligh-reliability}"
mkdir -p "$OUT_DIR"

if ! [[ "$N" =~ ^[0-9]+$ ]] || [[ "$N" -lt 1 ]]; then
  echo "usage: $0 <N> [settings|messages|both]" >&2
  exit 2
fi

if ! "$LIGH" daemon status &>/dev/null; then
  echo "error: lighd not running — ligh daemon start" >&2
  exit 1
fi

# Fail fast if guest is down (common cause of Settings/Safari timeouts).
if ! "$LIGH" --json status 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); raise SystemExit(0 if d.get("booted") else 1)'; then
  echo "error: simulator not booted — run: ligh up" >&2
  exit 1
fi

run_one() {
  local name="$1"
  local script="$ROOT/scripts/workloads/${name}.sh"
  local t0 t1 rc
  t0="$(python3 -c 'import time; print(time.time())')"
  set +e
  "$script" >"$OUT_DIR/${name}.$$.out" 2>"$OUT_DIR/${name}.$$.err"
  rc=$?
  set -e
  t1="$(python3 -c 'import time; print(time.time())')"
  python3 -c "print(int(($t1-$t0)*1000))" >"$OUT_DIR/${name}.$$.ms"
  return "$rc"
}

NAMES=()
case "$WORKLOAD" in
  settings)    NAMES=(settings-search) ;;
  messages)    NAMES=(messages-compose) ;;
  springboard) NAMES=(springboard-icons) ;;
  both)        NAMES=(settings-search messages-compose) ;;
  all)         NAMES=(springboard-icons settings-search messages-compose) ;;
  *)
    echo "unknown workload: $WORKLOAD (settings|messages|springboard|both|all)" >&2
    exit 2
    ;;
esac

echo "════════════════════════════════════════"
echo " LIGH reliability · N=$N · $WORKLOAD"
echo "════════════════════════════════════════"

TOTAL=0
PASS=0
FAIL=0
WALLS=()

for name in "${NAMES[@]}"; do
  echo
  echo "── workload: $name ──"
  for ((i=1; i<=N; i++)); do
    TOTAL=$((TOTAL + 1))
    if run_one "$name"; then
      PASS=$((PASS + 1))
      ms="$(cat "$OUT_DIR/${name}.$$.ms")"
      WALLS+=("$ms")
      echo "  [$i/$N] PASS  ${ms}ms"
    else
      FAIL=$((FAIL + 1))
      ms="$(cat "$OUT_DIR/${name}.$$.ms" 2>/dev/null || echo 0)"
      WALLS+=("$ms")
      echo "  [$i/$N] FAIL  ${ms}ms"
      tail -3 "$OUT_DIR/${name}.$$.err" 2>/dev/null | sed 's/^/         /' || true
    fi
    # Leave compose / settings so next run starts clean
    "$LIGH" home >/dev/null 2>&1 || true
    sleep 0.4
  done
done

RATE="$(python3 -c "print(0 if $TOTAL==0 else round(100.0*$FAIL/$TOTAL, 2))")"
STATS="$(python3 - <<PY
walls = list(map(int, """${WALLS[*]}""".split())) if """${WALLS[*]}""".strip() else []
if not walls:
  print("p50=0 p95=0")
else:
  s=sorted(walls)
  def pct(p):
    i=min(len(s)-1, max(0, int(round((p/100.0)*(len(s)-1)))))
    return s[i]
  print(f"p50={pct(50)} p95={pct(95)}")
PY
)"

REPORT="$OUT_DIR/report-$(date +%Y%m%d-%H%M%S).json"
python3 - <<PY
import json
report = {
  "n": $TOTAL,
  "passed": $PASS,
  "failed": $FAIL,
  "fail_rate_pct": $RATE,
  "workload": "$WORKLOAD",
  "wall_ms": list(map(int, """${WALLS[*]}""".split())) if """${WALLS[*]}""".strip() else [],
  "stats": "$STATS",
}
open("$REPORT","w").write(json.dumps(report, indent=2)+"\n")
print(json.dumps(report, indent=2))
PY

echo
echo "fail_rate=${RATE}%  $STATS"
echo "report → $REPORT"

# Soft gate: non-zero fail rate exits 1 (CI / local gate)
if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
exit 0
