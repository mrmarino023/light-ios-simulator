#!/usr/bin/env bash
# Build third-party XCUITestDemo.app (OSS, not designed for LIGH).
# No `| tail` — pipes swallow xcodebuild exit / SIGKILL under BuildGovernor.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJ="$ROOT/fixtures/third-party/XCUITestDemo/XCUITestDemo.xcodeproj"
OUT="$ROOT/fixtures/third-party/XCUITestDemo/build"
DERIVED="${LIGH_DERIVED_DATA:-$OUT/DerivedData}"
LOG="${LIGH_BUILD_LOG:-/tmp/ligh-build-xcuitestdemo.log}"

mkdir -p "$OUT" "$(dirname "$LOG")"
set +e
xcodebuild \
  -project "$PROJ" \
  -scheme XCUITestDemo \
  -configuration Debug \
  -sdk iphonesimulator \
  -derivedDataPath "$DERIVED" \
  -destination 'generic/platform=iOS Simulator' \
  ONLY_ACTIVE_ARCH=YES \
  ARCHS=arm64 \
  EXCLUDED_ARCHS=x86_64 \
  CODE_SIGNING_ALLOWED=NO \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGN_IDENTITY="" \
  build >"$LOG" 2>&1
rc=$?
set -e
tail -40 "$LOG" || true
[[ "$rc" -eq 0 ]] || exit "$rc"

APP=$(find "$DERIVED" -name 'XCUITestDemo.app' -type d | head -1)
[[ -n "$APP" && -d "$APP" ]] || { echo "✗ XCUITestDemo.app not found under $DERIVED"; exi