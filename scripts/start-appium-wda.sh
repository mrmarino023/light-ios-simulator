#!/usr/bin/env bash
# Start Appium for LIGH physical arms (WDA / XCUITest).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export APPIUM_HOME="${APPIUM_HOME:-$ROOT/.appium}"
APPIUM_BIN="${APPIUM_BIN:-$ROOT/node_modules/.bin/appium}"
if [[ ! -x "$APPIUM_BIN" ]]; then
  APPIUM_BIN="$(command -v appium || true)"
fi
if [[ -z "$APPIUM_BIN" ]]; then
  echo "appium not found — npm i -D appium && APPIUM_HOME=.appium appium driver install xcuitest" >&2
  exit 1
fi
echo "APPIUM_HOME=$APPIUM_HOME"
echo "starting Appium on :4723 (xcuitest)…"
exec "$APPIUM_BIN" --address 127.0.0.1 --port 4723 --relaxed-security
