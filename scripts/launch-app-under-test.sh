#!/usr/bin/env bash
# Launch an app under test for coding-agent flows.
# Usage:
#   ./scripts/launch-app-under-test.sh /path/to/MyApp.app [--bundle-id id]
#   ./scripts/launch-app-under-test.sh --bundle-id com.apple.calculator
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "missing $LIGH"

APP=""
BID=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-id) BID="${2:-}"; shift 2 ;;
    -*) fail "unknown arg $1" ;;
    *) APP="$1"; shift ;;
  esac
done

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$LIGH" up >/dev/null

if [[ -n "$APP" ]]; then
  if [[ -n "$BID" ]]; then
    "$LIGH" --json run "$APP" --bundle-id "$BID"
  else
    "$LIGH" --json run "$APP"
  fi
elif [[ -n "$BID" ]]; then
  UDID=$("$LIGH" --json status | python3 -c 'import json,sys;d=json.load(sys.stdin);print((d.get("session")or{}).get("udid")or"")')
  [[ -n "$UDID" ]] || fail "no udid"
  xcrun simctl launch "$UDID" "$BID"
  echo "{\"ok\":true,\"bundle_id\":\"$BID\",\"via\":\"simctl\"}"
else
  fail "pass .app path and/or --bundle-id"
fi

"$LIGH" --json observe --settle-ms 2500 | python3 -c '
import json,sys
d=json.load(sys.stdin)
print(json.dumps({
  "ax_quality": d.get("ax_quality"),
  "surface": (d.get("scene")or{}).get("surface"),
  "labels": [x.get("label") for x in (d.get("actionable_topk")or[])[:10]],
}, indent=2))
'
