#!/usr/bin/env bash
# Holy-shit demo: control-plane Bluetooth in one shot (no LLM, no PNG).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
[[ -x "$LIGH" ]] || { echo "build release ligh first"; exit 1; }

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-demo-first.log 2>&1

echo "══ control-plane demo: Settings → search Bluetooth ══"
START=$(date +%s)
"$LIGH" --json ready --settle-ms 2500 | python3 -c 'import json,sys;d=json.load(sys.stdin);assert d.get("ok"),d;print("✓ ready",d.get("phase"),d.get("surface"))'
"$LIGH" --json cap settings-search Bluetooth --settle-ms 3000 | python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok") and (d.get("detail") or {}).get("hit"), d
print("✓ settings_search Bluetooth hit=true fault=", d.get("fault"))
'
"$LIGH" --json cap assert-surface settings --settle-ms 1500 | python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok"), d
print("✓ assert_surface settings")
'
ELAPSED=$(( $(date +%s) - START ))
echo "✓ HOLY SHIT control-plane Bluetooth in ${ELAPSED}s (no LLM, no screenshot)"
