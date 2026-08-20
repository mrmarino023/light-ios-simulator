#!/usr/bin/env bash
# Shared helpers for agent workloads / reliability.
# shellcheck disable=SC2034

agent_require_ligh() {
  if ! "$LIGH" daemon status &>/dev/null; then
    echo "error: lighd not running — ligh daemon start" >&2
    return 1
  fi
  local booted
  booted="$("$LIGH" --json status 2>/dev/null | python3 -c 'import json,sys
try:
  d=json.load(sys.stdin)
  print("1" if d.get("booted") else "0")
except Exception:
  print("0")' 2>/dev/null || echo 0)"
  if [[ "$booted" != "1" ]]; then
    echo "error: simulator not booted — run: ligh up" >&2
    return 1
  fi
}

# Double-home and wait until a known SpringBoard icon is visible.
agent_wake_springboard() {
  local timeout_ms="${1:-20000}"
  local deadline end now
  end=$(( $(python3 -c 'import time; print(int(time.time()*1000))') + timeout_ms ))

  "$LIGH" home >/dev/null 2>&1 || true
  sleep 0.4
  "$LIGH" home >/dev/null 2>&1 || true
  sleep 0.6

  while true; do
    for L in Impostazioni Settings Messaggi Messages Safari; do
      if "$LIGH" exists --label "$L" &>/dev/null; then
        echo "$L"
        return 0
      fi
    done
    now=$(python3 -c 'import time; print(int(time.time()*1000))')
    if (( now >= end )); then
      echo "error: SpringBoard AX empty — no home icons after ${timeout_ms}ms (boot/AX flaky)" >&2
      return 1
    fi
    "$LIGH" home >/dev/null 2>&1 || true
    sleep 0.8
  done
}

# Sets SETTINGS SEARCH MESSAGES BODY CANCEL for IT/EN after SpringBoard is up.
agent_resolve_locale() {
  SETTINGS=Settings
  SEARCH=Search
  MESSAGES=Messages
  BODY=Message
  CANCEL=Cancel
  if "$LIGH" exists --label Impostazioni &>/dev/null; then
    SETTINGS=Impostazioni
    SEARCH=Cerca
  fi
  if "$LIGH" exists --label Messaggi &>/dev/null; then
    MESSAGES=Messaggi
    BODY=Messaggio
    CANCEL=Annulla
  fi
}

# Wait for either of two labels (IT/EN).
agent_wait_any() {
  local a="$1" b="$2" ms="${3:-12000}"
  if "$LIGH" wait --label "$a" --timeout-ms "$ms" >/dev/null 2>&1; then
    return 0
  fi
  "$LIGH" wait --label "$b" --timeout-ms "$ms" >/dev/null
}
