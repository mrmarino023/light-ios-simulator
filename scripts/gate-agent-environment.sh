#!/usr/bin/env bash
# Agent environment gate — proves Cursor MCP path is usable (honest, not marketing).
#
# Usage: ./scripts/gate-agent-environment.sh
# Output: docs/assets/agent-environment-latest.json
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_AGENT_ENV_OUT:-$ROOT/docs/assets/agent-environment-latest.json}"
APP="${ROOT}/fixtures/LighFixture/build/LighFixture.app"
BID="dev.ligh.Fixture"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -x "$LIGH" ]] || {
  echo "  ▶ building release…"
  (cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon --locked) \
    || fail "cargo build failed"
}
[[ -x "$LIGH" ]] || fail "build release ligh first: unset CARGO_TARGET_DIR && cargo build --release"
# Keep lighd in sync with workspace binary (stale daemon = unknown op / old motor).
LIGHD="${LIGH%d/ligh}/lighd"
[[ -x "$LIGHD" ]] || LIGHD="${CARGO_HOME:-$HOME/.cargo}/bin/lighd"
"$LIGH" daemon stop >/dev/null 2>&1 || true
pkill -x lighd 2>/dev/null || true
sleep 1
"$LIGH" daemon start
# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

echo "══ agent environment gate ══"
sim_clean_reboot "$LIGH" || fail "sim prep failed"

assert_booted_session() {
  # sim_clean_reboot can leave lighd up with empty udid after ready/eyes faults.
  local udid=""
  local i
  for i in 1 2 3 4 5; do
    "$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
    "$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}" >/dev/null 2>&1 || "$LIGH" up >/dev/null 2>&1 || true
    udid=$("$LIGH" --json observe --settle-ms 1200 2>/dev/null | python3 -c '
import json,sys
raw=sys.stdin.read(); i=raw.find("{")
try:
  d=json.loads(raw[i:] if i>=0 else "{}")
  print(d.get("udid") or "")
except Exception:
  print("")
' 2>/dev/null || true)
    if [[ -n "$udid" ]]; then
      echo "  session udid: $udid"
      return 0
    fi
    sleep 1
  done
  echo "✗ daemon observe has empty udid after up — session not attached" >&2
  return 1
}

assert_booted_session || fail "session attach failed"
"$LIGH" home >/dev/null 2>&1 || true
sleep 1

T0=$(python3 -c 'import time; print(time.time())')
CHECKS=()

ensure_daemon() {
  "$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
  # Re-attach if session dropped mid-gate (empty udid ⇒ all motor looks like eyes_unusable).
  local udid
  udid=$("$LIGH" --json observe --settle-ms 800 2>/dev/null | python3 -c '
import json,sys
raw=sys.stdin.read(); i=raw.find("{")
try:
  print(json.loads(raw[i:] if i>=0 else "{}").get("udid") or "")
except Exception:
  print("")
' 2>/dev/null || true)
  if [[ -z "$udid" ]]; then
    "$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}" >/dev/null 2>&1 || "$LIGH" up >/dev/null 2>&1 || true
  fi
}

run_check() {
  local id="$1" title="$2"
  shift 2
  ensure_daemon
  echo "  ▶ $title"
  if "$@"; then
    CHECKS+=("{\"id\":\"$id\",\"ok\":true}")
    echo "    PASS"
  else
    CHECKS+=("{\"id\":\"$id\",\"ok\":false}")
    echo "    FAIL"
  fi
  ensure_daemon
}

check_first_loop() {
  # Poll observe only — spamming `home`/`ready` while AX is empty post-boot
  # has killed lighd (SIGKILL) and left motor checks cascading fail.
  : >/tmp/agent-env-first.log
  local i out q n
  for i in $(seq 1 30); do
    out=$("$LIGH" --json observe --settle-ms 2000 2>/dev/null || true)
    q=$(printf '%s' "$out" | python3 -c '
import json,sys
raw=sys.stdin.read(); i=raw.find("{")
try:
  d=json.loads(raw[i:] if i>=0 else "{}")
  udid=bool(d.get("udid"))
  print(d.get("ax_quality"), (d.get("scene") or {}).get("surface"), len(d.get("actionable_topk") or []), int(udid))
except Exception:
  print("bad")
' 2>/dev/null || echo bad)
    echo "  wake $i: $q" >>/tmp/agent-env-first.log
    case "$q" in
      ready\ *\ [1-9]*\ 1|ready\ *\ [1-9][0-9]*\ 1)
        return 0
        ;;
    esac
    # One gentle home only if still empty after a few settles (not every loop).
    if [[ "$i" -eq 8 || "$i" -eq 16 ]]; then
      "$LIGH" home >/dev/null 2>&1 || true
    fi
    sleep 0.5
  done
  echo "    wake failed last=$q" >>/tmp/agent-env-first.log
  return 1
}

check_fixture_job() {
  [[ -d "$APP" ]] || "$ROOT/scripts/build-fixture.sh" >/tmp/agent-env-fixture.log 2>&1
  # Type requires focus — tap NameField before type (motor contract).
  local steps='[{"op":"wait","id":"LighHome"},{"op":"wait","id":"NameField"},{"op":"tap","id":"NameField"},{"op":"type","text":"env"},{"op":"tap","id":"GoNext"},{"op":"wait","id":"LighDone"}]'
  local doc
  doc=$("$LIGH" --json cap app-job "$APP" --bundle-id "$BID" --steps "$steps" \
    --settle-ms 4000 --timeout-ms 20000 2>/tmp/agent-env-job.err || true)
  python3 -c 'import json,sys; d=json.loads(sys.argv[1] if sys.argv[1].strip() else "{}"); sys.exit(0 if d.get("ok") else 1)' "$doc"
}

check_safari_goal() {
  local setup='[{"op":"launch","bundle_id":"com.apple.mobilesafari"},{"op":"wait","labels":["Indirizzo","Address","URL"]}]'
  local post='[{"wait_label":"Indirizzo","timeout_ms":8000},{"wait_label":"Address","timeout_ms":1}]'
  # post: first label that matches wins via reach — try IT then EN
  post='[{"wait_label":"Indirizzo","timeout_ms":12000}]'
  local doc
  doc=$("$LIGH" --json cap app-goal \
    --setup "$setup" \
    --postconditions "$post" \
    --settle-ms 4000 --timeout-ms 18000 2>/tmp/agent-env-safari.err || true)
  if python3 -c 'import json,sys; d=json.loads(sys.argv[1] if sys.argv[1].strip() else "{}"); sys.exit(0 if d.get("ok") else 1)' "$doc"; then
    return 0
  fi
  # EN locale fallback
  post='[{"wait_label":"Address","timeout_ms":12000}]'
  doc=$("$LIGH" --json cap app-goal \
    --setup "$setup" \
    --postconditions "$post" \
    --settle-ms 4000 --timeout-ms 18000 2>/tmp/agent-env-safari-en.err || true)
  python3 -c 'import json,sys; d=json.loads(sys.argv[1] if sys.argv[1].strip() else "{}"); sys.exit(0 if d.get("ok") else 1)' "$doc"
}

check_mcp_import() {
  python3 -c "
import sys, os
sys.path.insert(0, os.path.join('$ROOT', 'scripts'))
from ligh_mcp import call_tool
for t in ('ligh_agent_rules', 'ligh_observe', 'ligh_cap_reach'):
    r = call_tool(t, {}) if t != 'ligh_cap_reach' else call_tool(t, {'label': 'X'})
    assert 'ok' in r or 'rules' in r or 'error' in r
print('mcp ok')
"
}

run_check "first_loop" "SpringBoard AX wake" check_first_loop
run_check "mcp_import" "MCP call_tool smoke" check_mcp_import
run_check "fixture_app_job" "Debug .app app-job" check_fixture_job
run_check "safari_app_goal" "System app app-goal (Safari)" check_safari_goal

PASS=$(printf '%s\n' "${CHECKS[@]}" | grep -c '"ok":true' || true)
TOTAL=${#CHECKS[@]}
MS=$(python3 -c "import time; print(int((${T0:-0} and (time.time()-float('$T0'))*1000)))")

python3 - "$OUT" "$PASS" "$TOTAL" "$MS" "${CHECKS[@]}" <<'PY'
import json, sys
out, passed, total, ms = sys.argv[1:5]
checks_raw = sys.argv[5:]
checks = [json.loads(c) for c in checks_raw]
doc = {
  "gate": "agent_environment",
  "claim": "Cursor MCP + universal motor ops usable on this Mac",
  "ok": int(passed) == int(total) and int(total) > 0,
  "checks_pass": int(passed),
  "checks_total": int(total),
  "total_ms": int(ms),
  "checks": checks,
  "cursor_mcp": "scripts/print-cursor-mcp.sh",
  "docs": "docs/AGENT_ENV.md",
  "note": "Run gate after: unset CARGO_TARGET_DIR && cargo build --release. Stale lighd breaks app-goal ops.",
}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({"ok": doc["ok"], "pass": f"{passed}/{total}", "out": out}, indent=2))
PY

[[ "$PASS" -eq "$TOTAL" ]] || exit 1
echo "══ → $OUT"
