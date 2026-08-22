#!/usr/bin/env bash
# Build LighFixture.app for iphonesimulator (Debug).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJ="$ROOT/fixtures/LighFixture/LighFixture.xcodeproj"
OUT="$ROOT/fixtures/LighFixture/build"
DERIVED="$OUT/DerivedData"

mkdir -p "$OUT"
xcodebuild \
  -project "$PROJ" \
  -scheme LighFixture \
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

APP=$(find "$DERIVED" -name 'LighFixture.app' -type d | head -1)
[[ -n "$APP" && -d "$APP" ]] || { echo "✗ LighFixture.app not found under $DERIVED"; exit 1; }
rm -rf "$OUT/LighFixture.app"
cp -R "$APP" "$OUT/LighFixture.app"
echo "✓ $OUT/LighFixture.app"
echo "$OUT/LighFixture.app"
