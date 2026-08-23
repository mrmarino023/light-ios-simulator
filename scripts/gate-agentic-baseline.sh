#!/usr/bin/env bash
# Agentic baseline gate — LIGH scripted vs vision (vision skipped without OPENAI_API_KEY).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_AGENTIC_BASELINE_OUT:-$ROOT/docs/assets/agentic-baseline-latest.json}"

fail() { echo "✗ $*" >&2; exit 1; }

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"

[[ -x "$LIGH" ]] || (cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon --locked) || fail "build failed"
"$LIGH" daemon stop >/dev/null 2>&1 || true
sleep 1
"$LIGH" daemon start
# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"
sim_clean_reboot "$LIGH" || fail "sim prep"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/agentic-baseline-first.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 4000 --recover-homes 6 >/dev/null \
  || fail "ligh_ready failed"
"$ROOT/scripts/build-xcuitestdemo.sh" >/tmp/agentic-baseline-build.log 2>&1 || fail "XCUITestDemo build"

ARGS=(--arm both --no-llm)
if [[ -n "${OPENAI_API_KEY:-}" ]]; then
  ARGS=(--arm both)
fi

python3 "$ROOT/scripts/agentic-baseline.py" "${ARGS[@]}"
echo "══ → $OUT"
