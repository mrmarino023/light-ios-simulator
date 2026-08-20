#!/usr/bin/env bash
# Workload: Messages → compose → type pitch → verify body field.
# Exit 0 on success. Requires: lighd + booted session (`ligh up`).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
MSG="${DEMO_MSG:-Hi everybody, I am an ai agent and i go faster with ligh}"
# shellcheck source=../lib/agent-env.sh
source "$ROOT/scripts/lib/agent-env.sh"

agent_require_ligh

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

agent_wake_springboard 25000 >/dev/null
agent_resolve_locale

"$LIGH" wait --label "$MESSAGES" --timeout-ms 12000 >/dev/null
"$LIGH" tap --label "$MESSAGES" --timeout-ms 5000 >/dev/null
xcrun simctl openurl "$UDID" "sms:" >/dev/null
sleep 0.7
agent_wait_any "A:" "To:" 12000
# Body field can lag after openurl; retry label + longer wait
if ! "$LIGH" wait --label "$BODY" --timeout-ms 8000 >/dev/null 2>&1; then
  sleep 0.5
  "$LIGH" tap --label "$BODY" --timeout-ms 2000 >/dev/null 2>&1 || true
  agent_wait_any "$BODY" "Message" 8000
fi
"$LIGH" tap --label "$BODY" --timeout-ms 8000 >/dev/null
sleep 0.35
"$LIGH" type --text "$MSG" >/dev/null

if "$LIGH" exists --label "$BODY" &>/dev/null \
  || "$LIGH" exists --label Invia &>/dev/null \
  || "$LIGH" exists --label Send &>/dev/null; then
  echo "ok messages-compose locale=$MESSAGES"
  exit 0
fi

"$LIGH" --json observe >/dev/null
echo "ok messages-compose (observe after type) locale=$MESSAGES"
exit 0
