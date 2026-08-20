#!/usr/bin/env bash
# Real-app typing demo: Messages compose.
# Opens Messaggi → new message → types the agent pitch into the body field.
#
# Requires: lighd + booted session. Prefer `ligh gui` in frame.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
PAUSE="${DEMO_PAUSE:-1.0}"
# Avoid apostrophe: IT sim keyboard turns I'm into "làm" via IndigoHID.
if [[ -n "${DEMO_MSG:-}" ]]; then
  MSG="$DEMO_MSG"
else
  MSG="Hi everybody, I am an ai agent and i go faster with ligh"
fi

if ! "$LIGH" daemon status &>/dev/null; then
  echo "error: lighd not running — run: ligh daemon start" >&2
  exit 1
fi

UDID=""
for SESSION in "$HOME/.ligh/session.json" "$HOME/Library/Application Support/ligh/session.json"; do
  if [[ -f "$SESSION" ]]; then
    UDID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["udid"])' "$SESSION")"
    break
  fi
done
if [[ -z "$UDID" ]]; then
  echo "error: no session UDID — run ligh up first" >&2
  exit 1
fi

echo "════════════════════════════════════════"
echo " LIGH — Messaggi type demo"
echo "════════════════════════════════════════"
echo "  message: $MSG"
echo

say() { echo; echo "▶ $*"; sleep 0.2; }
pause() { sleep "$PAUSE"; }

MESSAGES=Messages
NEW_MSG="New Message"
BODY=Message

say "home"
"$LIGH" home
pause
"$LIGH" home
pause

if "$LIGH" exists --label Messaggi &>/dev/null; then
  MESSAGES=Messaggi
  NEW_MSG="Nuovo messaggio"
  BODY=Messaggio
fi

say "wait  $MESSAGES"
"$LIGH" wait --label "$MESSAGES" --timeout-ms 8000
pause

say "tap   $MESSAGES"
"$LIGH" tap --label "$MESSAGES" --timeout-ms 4000
pause

say "compose  (sms:)"
xcrun simctl openurl "$UDID" "sms:"
pause

say "wait  compose (A:)"
# "A:" is more reliable than the title label on some boots
if ! "$LIGH" wait --label "A:" --timeout-ms 8000; then
  "$LIGH" wait --label "$NEW_MSG" --timeout-ms 4000
fi
pause

say "tap   $BODY"
"$LIGH" tap --label "$BODY" --timeout-ms 4000
sleep 0.4
pause

say "type  (agent pitch)"
"$LIGH" type --text "$MSG"
pause

say "verify"
if "$LIGH" exists --label "$BODY" &>/dev/null \
  || "$LIGH" exists --label Invia &>/dev/null \
  || "$LIGH" exists --label Send &>/dev/null; then
  echo "    → compose field active"
else
  "$LIGH" --json observe >/dev/null && echo "    → observe ok after type"
fi
pause

say "screenshot"
"$LIGH" screenshot -o /tmp/ligh-messages-demo.png
echo "    → /tmp/ligh-messages-demo.png"

echo
echo "════════════════════════════════════════"
echo " ✓ demo complete — stop recording"
echo "════════════════════════════════════════"
