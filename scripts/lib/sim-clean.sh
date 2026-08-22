#!/usr/bin/env bash
# Clean sim + daemon for reproducible gates (AX degrades after long Maestro runs).
set -euo pipefail

sim_clean_reboot() {
  local ligh="${LIGH_BIN:-${1:-}}"
  [[ -n "$ligh" ]] || ligh="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/target/release/ligh"
  local udid
  udid=$(xcrun simctl list devices booted 2>/dev/null | grep -oE '[A-F0-9-]{36}' | head -1 || true)
  if [[ -n "$udid" ]]; then
    echo "  ▶ sim shutdown $udid"
    xcrun simctl shutdown "$udid" 2>/dev/null || true
    sleep 2
  fi
  "$ligh" daemon stop 2>/dev/null || true
  sleep 1
  "$ligh" daemon start
  "$ligh" up --device "${LIGH_DEVICE:-iphone-15-pro}" 2>/dev/null || "$ligh" up
  "$ligh" home >/dev/null 2>&1 || true
  sleep 1
}

ligh_first_loop() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  "$root/scripts/agent-first-loop.sh"
}
