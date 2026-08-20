#!/usr/bin/env bash
# End-to-end proof for LIGH v3 — run on macOS with Xcode + iOS runtime.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== LIGH proof ==="
echo "host: $(uname -m) · $(sw_vers -productVersion)"
echo "xcode: $(xcodebuild -version | head -1)"
echo

cargo build --release -q
LIGH="$ROOT/target/release/ligh"

echo "--- doctor ---"
"$LIGH" doctor
echo

echo "--- shutdown any prior session ---"
"$LIGH" down 2>/dev/null || true
xcrun simctl shutdown all 2>/dev/null || true
sleep 3
echo

echo "--- probe (private boot + IOSurface→Metal + HID) ---"
"$LIGH" probe --device iphone-15-pro
echo

echo "--- gui verify (Metal window + present) ---"
"$LIGH" gui --device iphone-15-pro --verify
echo

echo "--- sim memory (scoped) ---"
"$LIGH" sim measure || true
echo

echo "--- booted devices ---"
xcrun simctl list devices booted
echo

echo "=== proof complete ==="
