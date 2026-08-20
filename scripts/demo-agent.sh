#!/usr/bin/env bash
# Screen-record friendly agent loop (~25–35s with pauses).
# Requires: lighd + booted session. Prefer `ligh gui` visible in the frame.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
PAUSE="${DEMO_PAUSE:-1.1}"

if ! "$LIGH" daemon status &>/dev/null; then
  echo "error: lighd not running — ligh daemon start" >&2
  exit 1
fi

echo "════════════════════════════════════════"
echo " LIGH — observe → act → verify"
echo "════════════════════════════════════════"
echo "  tip: keep the Metal GUI (or Simulator) in frame"
echo

say() { echo; echo "▶ $*"; sleep 0.25; }
pause() { sleep "$PAUSE"; }

# Locale
SETTINGS=Settings
GENERAL=General
SEARCH=Search
if "$LIGH" exists --label Impostazioni &>/dev/null; then
  SETTINGS=Impostazioni
  GENERAL=Generali
  SEARCH=Cerca
fi

say "home"
"$LIGH" home
pause
"$LIGH" home
pause

say "wait  $SETTINGS"
"$LIGH" wait --label "$SETTINGS" --timeout-ms 8000
pause

say "tap   $SETTINGS"
"$LIGH" tap --label "$SETTINGS" --timeout-ms 4000
pause

say "wait  $GENERAL  (verify Settings opened)"
"$LIGH" wait --label "$GENERAL" --timeout-ms 8000
pause

say "observe  (structured AX + frame)"
"$LIGH" --json observe | python3 -c '
import json,sys
d=json.load(sys.stdin)
ax=d.get("accessibility_tree") or {}
n=0
if isinstance(ax, dict):
  n=ax.get("element_count") or len(ax.get("nodes") or ax.get("elements") or [])
print(f"    → observe ok · ~{n} AX elements")
' 2>/dev/null || echo "    → observe ok"
pause

say "tap   $SEARCH"
"$LIGH" tap --label "$SEARCH" --timeout-ms 4000
pause

say "type  ligh"
"$LIGH" type --text "ligh"
pause

say "verify  (search field / clear control)"
if "$LIGH" exists --label "Cancella testo" &>/dev/null \
  || "$LIGH" exists --label "Clear text" &>/dev/null \
  || "$LIGH" exists --label "ligh" &>/dev/null; then
  echo "    → verify ok"
else
  "$LIGH" --json observe >/dev/null && echo "    → observe after type ok"
fi
pause

say "screenshot"
"$LIGH" screenshot -o /tmp/ligh-demo-final.png
echo "    → /tmp/ligh-demo-final.png"

echo
echo "════════════════════════════════════════"
echo " ✓ demo complete — stop recording"
echo "════════════════════════════════════════"
