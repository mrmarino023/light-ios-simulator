#!/usr/bin/env bash
# Workload: SpringBoard icons visible (locale-aware smoke).
# Exit 0 if at least two known home icons exist after wake.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
# shellcheck source=../lib/agent-env.sh
source "$ROOT/scripts/lib/agent-env.sh"

agent_require_ligh
icon="$(agent_wake_springboard 25000)"
agent_resolve_locale

n=0
for L in "$SETTINGS" "$MESSAGES" Safari; do
  if "$LIGH" exists --label "$L" &>/dev/null; then
    n=$((n + 1))
  fi
done

if [[ "$n" -lt 2 ]]; then
  echo "error: expected ≥2 home icons, found $n (wake saw $icon)" >&2
  exit 1
fi

echo "ok springboard-icons n=$n wake=$icon settings=$SETTINGS messages=$MESSAGES"
exit 0
