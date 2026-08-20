#!/usr/bin/env bash
# Time-to-first-loop: daemon start → first successful observe + wait(label).
# Measures cold-ish agent readiness on an already-booted sim (`ligh up`).
#
# Usage: ./scripts/time-to-first-loop.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
# shellcheck source=lib/agent-env.sh
source "$ROOT/scripts/lib/agent-env.sh"

echo "════════════════════════════════════════"
echo " LIGH — time to first loop"
echo "════════════════════════════════════════"

# Must already have a session; we only bounce the daemon.
if ! "$LIGH" --json status 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); raise SystemExit(0 if d.get("booted") else 1)'; then
  echo "error: simulator not booted — run: ligh up" >&2
  exit 1
fi

T0="$(python3 -c 'import time; print(time.time())')"

"$LIGH" daemon stop >/dev/null 2>&1 || true
"$LIGH" daemon start >/dev/null
"$LIGH" --json observe >/dev/null
LABEL="$(agent_wake_springboard 25000)"

T1="$(python3 -c 'import time; print(time.time())')"
MS="$(python3 -c "print(int(($T1-$T0)*1000))")"

echo "first_loop_ms=$MS  label=$LABEL"
echo "ok"
