#!/usr/bin/env bash
# 5-minute onboarding: build, doctor, daemon, optional Expo vendoring, MCP snippet.
#
# Usage:
#   ./scripts/ligh-init.sh
#   ./scripts/ligh-init.sh /path/to/YourExpoApp
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXPO_APP="${1:-}"

echo "══ LIGH init ══"
echo "  repo: $ROOT"
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

if [[ -n "$EXPO_APP" && -d "$EXPO_APP" ]]; then
  echo "▶ sync @mm-labs/ligh-expo → $EXPO_APP"
  "$ROOT/scripts/sync-ligh-expo.sh" "$EXPO_APP"
  echo "  → add \"@mm-labs/ligh-expo\" to app.json plugins[], rebuild dev client"
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
echo "  sim:     $LIGH up && $LIGH observe --json"
echo "  phone:   edit ~/.ligh/wda.env → ./scripts/start-appium-wda.sh → $LIGH device wait"
echo "  prove:   ./scripts/gate-trail-holy-multi.sh"
