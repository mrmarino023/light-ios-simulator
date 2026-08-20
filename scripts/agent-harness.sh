#!/usr/bin/env bash
# Local agent harness — one command for the de-facto local gate.
#
# Requires: lighd + booted sim (`ligh daemon start && ligh up`).
#
# Runs: TTFL → springboard → settings×3 → messages×3 → optional REL_N both
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export LIGH_BIN="${LIGH_BIN:-$ROOT/target/release/ligh}"
REL_N="${LIGH_HARNESS_REL_N:-0}"

echo "════════════════════════════════════════"
echo " LIGH local agent harness"
echo "════════════════════════════════════════"

"$LIGH_BIN" daemon status >/dev/null
if ! "$LIGH_BIN" --json status 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); raise SystemExit(0 if d.get("booted") else 1)'; then
  echo "error: simulator not booted — run: ligh up" >&2
  exit 1
fi

./scripts/time-to-first-loop.sh
./scripts/workloads/springboard-icons.sh
./scripts/agent-reliability.sh 3 settings
./scripts/agent-reliability.sh 3 messages

if [[ "$REL_N" =~ ^[0-9]+$ ]] && [[ "$REL_N" -gt 0 ]]; then
  echo
  echo "==> extended reliability N=$REL_N both"
  ./scripts/agent-reliability.sh "$REL_N" both
fi

echo
echo "✓ local harness ok"
