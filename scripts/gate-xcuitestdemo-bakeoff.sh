#!/usr/bin/env bash
# Third-party bakeoff: XCUITestDemo (OSS, himalidev) — login → homeTitle.
# Same semantic job on LIGH app-job vs Maestro.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

chmod +x "$ROOT/scripts/build-xcuitestdemo.sh" 2>/dev/null || true

export LIGH_APP_PROFILE=xcuitestdemo
export LIGH_APP_LABEL="XCUITestDemo (OSS third-party)"
export LIGH_APP_PATH="$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
export LIGH_APP_BUNDLE_ID=com.himali.XCUITestDemo
export LIGH_APP_BUILD_SCRIPT=build-xcuitestdemo.sh
export LIGH_MAESTRO_FLOW="$ROOT/fixtures/third-party/XCUITestDemo/maestro-job.yaml"
export LIGH_BAKEOFF_OUT="${LIGH_BAKEOFF_OUT:-$ROOT/docs/assets/third-party-bakeoff-latest.json}"
export LIGH_APP_SETTLE_MS=3500
export LIGH_APP_TIMEOUT_MS=15000
export PATH="${HOME}/.maestro/bin:${PATH}"

# Clean sim state — AX degrades after long Maestro sessions.
UDID=$(xcrun simctl list devices booted 2>/dev/null | grep -oE '[A-F0-9-]{36}' | head -1 || true)
if [[ -n "$UDID" ]]; then
  xcrun simctl shutdown "$UDID" 2>/dev/null || true
  sleep 2
fi
"$ROOT/target/release/ligh" daemon stop 2>/dev/null || true
sleep 1

exec "$ROOT/scripts/gate-app-bakeoff.sh"
