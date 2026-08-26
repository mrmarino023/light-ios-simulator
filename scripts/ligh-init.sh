#!/usr/bin/env bash
# 5-minute onboarding: build, doctor, daemon, MCP snippet.
# For full agent setup (audit + smoke + prompt): ./scripts/ligh-paradise.sh
#
# Usage:
#   ./scripts/ligh-init.sh
#   ./scripts/ligh-paradise.sh /path/to/YourApp.xcodeproj --build
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "══ LIGH init ══"
echo "  repo: $ROOT"
echo "  → agent paradise: ./scripts/ligh-paradise.sh [YourApp.xcodeproj --build]"
echo

[[ "$(uname -s)" == "Darwin" ]] || { echo "error: macOS required" >&2; exit 1; }
xcode-select -p &>/dev/null || { echo "error: install Xcode CLT" >&2; exit 1; }
command -v cargo &>/dev/null || { echo "error: install Rust (rustup.rs)" >&2; exit 1; }

echo "▶ build release…"
(cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release --locked -p ligh-cli -p ligh-daemon)

LIGH="$ROOT/target/release/ligh"
LIGHD="$ROOT/target/release/lighd"
export PATH="$ROOT/target/release:${PATH:-}"

mkdir -p "$HOME/.ligh"
if [[ ! -f "$HOME/.ligh/wda.env" ]]; then
  cp "$ROOT/scripts/wda.env.example" "$HOME/.ligh/wda.env"
  echo "▶ created ~/.ligh/wda.env — edit UDID + bundle for physical arms"
fi

echo "▶ doctor"
"$LIGH" doctor || true

echo "▶ daemon"
"$LIGH" daemon stop --json 2>/dev/null || true
pkill -x lighd 2>/dev/null || true
sleep 1
nohup "$LIGHD" >> /tmp/lighd-init.log 2>&1 &
sleep 1
"$LIGH" daemon status --json || true

echo
echo "▶ Cursor MCP (paste into Settings → MCP)"
"$ROOT/scripts/print-cursor-mcp.sh"

echo
echo "✓ init done"
echo "  paradise: ./scripts/ligh-paradise.sh"
echo "  sim:      $LIGH up && $LIGH observe --json"
echo "  test:     LIGH_WORKSPACE=/path/to/app ./scripts/ligh-test.sh"
echo "  prove:    ./scripts/gate-trail-holy-multi.sh"
