#!/usr/bin/env bash
# Agent workload bench — 30–50 step observe→act→verify vs simctl/MCP (WDA when wired).
# Headline metric is workload wall-clock + pass rate, not screenshot vanity.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
LIGHD="${LIGHD_BIN:-$ROOT/target/release/lighd}"
STEPS="${1:-40}"
ITERS="${2:-6}"

if [[ ! -x "$LIGH" || ! -x "$LIGHD" ]]; then
  echo "build first: cargo build --release -p ligh-cli -p ligh-daemon"
  exit 1
fi

echo "ensuring session + lighd…"
"$LIGH" daemon start

if [[ "${LIGH_BENCH_JSON:-}" == "1" ]]; then
  "$LIGH" --json bench agent --steps "$STEPS" --iterations "$ITERS"
else
  "$LIGH" bench agent --steps "$STEPS" --iterations "$ITERS"
fi
