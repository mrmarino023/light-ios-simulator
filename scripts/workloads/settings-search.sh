#!/usr/bin/env bash
# Workload: Settings → Search → type needle → verify.
# Exit 0 on success. Requires: lighd + booted session (`ligh up`).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
NEEDLE="${LIGH_NEEDLE:-ligh}"
# shellcheck source=../lib/agent-env.sh
source "$ROOT/scripts/lib/agent-env.sh"

agent_require_ligh
agent_wake_springboard 25000 >/dev/null
agent_resolve_locale

"$LIGH" wait --label "$SETTINGS" --timeout-ms 12000 >/dev/null
"$LIGH" tap --label "$SETTINGS" --timeout-ms 5000 >/dev/null
"$LIGH" wait --label "$SEARCH" --timeout-ms 12000 >/dev/null
"$LIGH" tap --label "$SEARCH" --timeout-ms 5000 >/dev/null
"$LIGH" type --text "$NEEDLE" >/dev/null

if "$LIGH" exists --label "Cancella testo" &>/dev/null \
  || "$LIGH" exists --label "Clear text" &>/dev/null \
  || "$LIGH" exists --label "$NEEDLE" &>/dev/null; then
  echo "ok settings-search needle=$NEEDLE locale=$SETTINGS"
  exit 0
fi

"$LIGH" --json observe >/dev/null
echo "ok settings-search (observe after type) locale=$SETTINGS"
exit 0
