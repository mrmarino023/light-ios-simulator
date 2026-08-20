#!/usr/bin/env bash
# Default: killer agent workflow bench (LIGHd vs simctl/MCP).
# Legacy GPU path: ./scripts/benchmark.sh boot [device]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build --release -q -p ligh-cli -p ligh-daemon

MODE="${1:-agent}"
case "$MODE" in
  agent)
    exec ./target/release/ligh bench agent --steps "${2:-40}"
    ;;
  boot)
    exec ./target/release/ligh bench boot --device "${2:-iphone-15-pro}"
    ;;
  *)
    echo "usage: $0 [agent [steps]|boot [device]]" >&2
    exit 1
    ;;
esac
