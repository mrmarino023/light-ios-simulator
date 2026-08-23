#!/usr/bin/env bash
# Full agent stack validation — run before Cursor trials or publishing claims.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "══════════════════════════════════════"
echo " LIGH full agent gate"
echo "══════════════════════════════════════"

export LIGH_BIN="${LIGH_BIN:-$ROOT/target/release/ligh}"
(cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon --locked) \
  || { echo "✗ build failed"; exit 1; }

"$ROOT/scripts/gate-agent-environment.sh" || exit 1
"$ROOT/scripts/gate-agent-unified-loop.sh" || exit 1
"$ROOT/scripts/gate-agentic-baseline.sh" || exit 1
"$ROOT/scripts/gate-external-apps.sh" || exit 1

echo
echo "✓ full agent gate complete"
echo "  docs/assets/agent-environment-latest.json"
echo "  docs/assets/agent-unified-loop-gate-latest.json"
echo "  docs/assets/agentic-baseline-latest.json"
echo "  docs/assets/external-apps-latest.json"
