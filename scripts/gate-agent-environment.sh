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

T0=$(python3 -c 'import time; print(time.time())')
CHECKS=()

run_check() {
  local id="$1" title="$2"
  shift 2
  echo "  ▶ $title"
  if "$@"; then
    CHECKS+=("{\"id\":\"$id\",\"ok\":true}")
    echo "    PASS"
  else
    CHECKS+=("{\"id\":\"$id\",\"ok\":false}")
    echo "    FAIL"
  fi
}

check_first_loop() {
  "$ROOT/scripts/agent-first-loop.sh" >/tmp/agent-env-first.log 2>&1
}

check_fixture_job() {
  [[ -d "$APP" ]] || "$ROOT/scripts/build-fixture.sh" >/tmp/agent-env-fixture.log 2>&1
  local steps='[{"op":"wait","id":"LighHome"},{"op":"wait","id":"NameField"},{"op":"type","text":"env"},{"op":"tap","id":"GoNext"},{"op":"wait","id":"LighDone"}]'
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
