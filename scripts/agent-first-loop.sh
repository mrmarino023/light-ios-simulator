#!/usr/bin/env bash
# Cold-ish path for coding agents: daemon → up → wake SpringBoard AX → one observe.
# Gate: useful on a warm Xcode Mac in well under 5 minutes.
# Usage: ./scripts/agent-first-loop.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first (cargo build --release -p ligh-cli)"

wake_until_ready() {
  local rounds="${1:-30}"
  local i q n out
  for i in $(seq 1 "$rounds"); do
    "$LIGH" home >/dev/null 2>&1 || true
    sleep 0.4
    out=$("$LIGH" --json observe --settle-ms 2000 2>/dev/null || true)
    q=$(printf '%s' "$out" | python3 -c '
import json,sys
try:
  d=json.load(sys.stdin)
  print(d.get("ax_quality"), (d.get("scene") or {}).get("surface"), len(d.get("actionable_topk") or []), d.get("settled"))
except Exception:
  print("bad")
' 2>/dev/null || echo bad)
    echo "  wake $i: $q"
    case "$q" in
      ready\ springboard\ *)
        n=$(echo "$q" | awk '{print $3}')
        if [[ "${n:-0}" -gt 0 ]]; then return 0; fi
        ;;
    esac
  done
  return 1
}

START=$(date +%s)
echo "══ agent-first-loop ══"

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$LIGH" up

if ! wake_until_ready 20; then
  echo "  AX stuck empty — recycling daemon…"
  "$LIGH" daemon stop >/dev/null 2>&1 || true
  sleep 1
  "$LIGH" daemon start
  "$LIGH" up
  wake_until_ready 25 || fail "SpringBoard AX never ready (empty/transition)"
fi

echo "── observe (agent view) ──"
"$LIGH" --json observe --settle-ms 2500 | python3 -c '
import json,sys
d=json.load(sys.stdin)
scene=d.get("scene") or {}
top=d.get("actionable_topk") or []
print(json.dumps({
  "ax_quality": d.get("ax_quality"),
  "settled": d.get("settled"),
  "surface": scene.get("surface"),
  "n_actionable": len(top),
  "labels": [x.get("label") for x in top[:12]],
}, indent=2))
'

ELAPSED=$(( $(date +%s) - START ))
echo "✓ first loop ready in ${ELAPSED}s"
if [[ "$ELAPSED" -gt 300 ]]; then
  echo "⚠ exceeded 5 min soft gate (${ELAPSED}s)" >&2
  exit 2
fi
exit 0
