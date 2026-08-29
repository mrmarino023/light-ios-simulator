#!/usr/bin/env bash
# Build third-party Kix.app (OSS catalog/tabs demo, not designed for LIGH).
# Upstream: https://github.com/byKosta/Kix-app @ 0bc85035
# No `| tail` — preserve xcodebuild exit / SIGKILL for BuildGovernor.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJ="$ROOT/fixtures/third-party/Kix/Kix.xcodeproj"
OUT="$ROOT/fixtures/third-party/Kix/build"
DERIVED="${LIGH_DERIVED_DATA:-$OUT/DerivedData}"
BUNDLE_ID="mybyKosta.Kix"
LOG="${LIGH_BUILD_LOG:-/tmp/ligh-build-kix.log}"

mkdir -p "$OUT" "$(dirname "$LOG")"
set +e
xcodebuild \
  -project "$PROJ" \
  -scheme Kix \
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

APP=$(find "$DERIVED" -name 'Kix.app' -type d | head -1)
[[ -n "$APP" && -d "$APP" ]] || { echo "✗ Kix.app not found under $DERIVED"; exit 1; }
rm -rf "$OUT/Kix.app"
cp -R "$APP" "$OUT/Kix.app"
/usr/libexec/PlistBuddy -c "Set :MinimumOSVersion 17.0" "$OUT/Kix.app/Info.plist" 2>/dev/null || true

if [[ "${LIGH_TRAIL_FAST:-0}" != "1" ]]; then
  UDID=$(xcrun simctl list devices booted 2>/dev/null | grep -oE '[A-F0-9-]{36}' | head -1 || true)
  if [[ -n "$UDID" ]]; then
    xcrun simctl uninstall "$UDID" "$BUNDLE_ID" 2>/dev/null || true
  fi
fi

echo "✓ $OUT/Kix.app"
echo "$OUT/Kix.app"
