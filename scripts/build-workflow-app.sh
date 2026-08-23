#!/usr/bin/env bash
# Build a workflow fixture app (LighFeed, LighOnboard, …).
# Usage: ./scripts/build-workflow-app.sh LighFeed
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${1:?app name}"
PROJ="$ROOT/fixtures/$NAME/$NAME.xcodeproj"
OUT="$ROOT/fixtures/$NAME/build"
DERIVED="$OUT/DerivedData"

[[ -d "$PROJ" ]] || { echo "✗ missing $PROJ"; exit 1; }
mkdir -p "$OUT"
xcodebuild \
  -project "$PROJ" \
  -scheme "$NAME" \
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

APP=$(find "$DERIVED" -name "$NAME.app" -type d | head -1)
[[ -n "$APP" && -d "$APP" ]] || { echo "✗ $NAME.app not found"; exit 1; }
rm -rf "$OUT/$NAME.app"
cp -R "$APP" "$OUT/$NAME.app"
echo "✓ $OUT/$NAME.app"
echo "$OUT/$NAME.app"
