#!/usr/bin/env bash
# Build third-party XCUITestDemo.app (OSS, not designed for LIGH).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJ="$ROOT/fixtures/third-party/XCUITestDemo/XCUITestDemo.xcodeproj"
OUT="$ROOT/fixtures/third-party/XCUITestDemo/build"
DERIVED="$OUT/DerivedData"

mkdir -p "$OUT"
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
  build \
  2>&1 | tail -40

APP=$(find "$DERIVED" -name 'XCUITestDemo.app' -type d | head -1)
[[ -n "$APP" && -d "$APP" ]] || { echo "✗ XCUITestDemo.app not found under $DERIVED"; exit 1; }
rm -rf "$OUT/XCUITestDemo.app"
cp -R "$APP" "$OUT/XCUITestDemo.app"
/usr/libexec/PlistBuddy -c "Set :MinimumOSVersion 18.0" "$OUT/XCUITestDemo.app/Info.plist" 2>/dev/null || true
echo "✓ $OUT/XCUITestDemo.app"
echo "$OUT/XCUITestDemo.app"
