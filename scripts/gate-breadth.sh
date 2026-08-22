#!/usr/bin/env bash
# Breadth gate — harder goals, LLM policy only (no host assumed answers).
# Expect lower pass rates than the narrow 40/40 Settings/Messages compose gate.
# Usage:
#   OPENAI_API_KEY=… ./scripts/gate-breadth.sh
#   LIGH_BREADTH_N=5 OPENAI_MODEL=gpt-5-mini ./scripts/gate-breadth.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_BREADTH_N:-5}"
MODEL="${OPENAI_MODEL:-gpt-5-mini}"
OUT="$ROOT/docs/assets/breadth-gate-latest.json"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
"$LIGH" daemon status >/dev/null 2>&1 || fail "lighd not running"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

echo "══ Breadth gate N=$N model=$MODEL policy=llm ══"

GOALS=(
  "Open Settings (Impostazioni or Settings), use search Cerca/Search, find Bluetooth, leave the Bluetooth row or screen visible, then done"
  "Open Safari, make sure the browser chrome or address field is visible, then done"
  "Open Settings, open Generali or General, then navigate back so Impostazioni/Settings root list is visible again, then done"
)

PASS=0
TOTAL=0
RESULTS=()

for goal in "${GOALS[@]}"; do
  for ((i=1; i<=N; i++)); do
    TOTAL=$((TOTAL + 1))
    "$LIGH" home >/dev/null 2>&1 || true
    sleep 0.35
    "$LIGH" home >/dev/null 2>&1 || true
    sleep 0.5
    if python3 "$ROOT/scripts/agent-llm-loop.py" --policy llm --model "$MODEL" --steps 18 --goal "$goal" >/tmp/ligh-breadth.log 2>&1; then
      PASS=$((PASS + 1))
      echo "  ok  $i/$N — ${goal:0:48}…"
      RESULTS+=("pass")
    else
      echo "  FAIL $i/$N — ${goal:0:48}… (see /tmp/ligh-breadth.log)"
      RESULTS+=("fail")
    fi
  done
done

python3 - <<PY
import json, time
report = {
  "ts": time.time(),
  "model": "$MODEL",
  "policy": "llm",
  "n_per_goal": int("$N"),
  "pass": int("$PASS"),
  "total": int("$TOTAL"),
  "pass_rate": (int("$PASS") / max(int("$TOTAL"), 1)),
  "goals": [
    "settings_search_bluetooth",
    "safari_chrome",
    "settings_general_back",
  ],
  "claim": "breadth_probe_not_narrow_40_40",
}
open("$OUT", "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print(f"breadth {report['pass']}/{report['total']} ({100*report['pass_rate']:.0f}%)")
PY

echo "wrote $OUT"
# Do not fail the script on low pass — breadth is diagnostic.
exit 0
