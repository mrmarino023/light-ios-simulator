#!/usr/bin/env bash
# Fail-closed gate: injected failures must return ok=false + explicit fault (never soft-success).
#
# Usage: ./scripts/gate-fail-closed.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_FAIL_CLOSED_OUT:-$ROOT/docs/assets/fail-closed-latest.json}"
APP="${LIGH_APP_PATH:-$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app}"
BUNDLE_ID="${LIGH_APP_BUNDLE_ID:-com.himali.XCUITestDemo}"
SETTLE_MS=3500
TIMEOUT_MS=12000

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
[[ -d "$APP" ]] || "$ROOT/scripts/build-xcuitestdemo.sh"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

eval_case() {
  local id="$1" expect_ok="$2" note="$3" raw_file="$4"
  python3 - "$id" "$expect_ok" "$note" "$raw_file" <<'PY'
import json, sys
cid, expect_ok, note, raw_file = sys.argv[1:5]
expect_ok = expect_ok == "true"
try:
  raw = open(raw_file).read()
  d = json.loads(raw) if raw.strip() else {}
except Exception as e:
  d = {"ok": False, "fault": "infra", "detail": {"error": str(e), "raw_head": open(raw_file).read()[:200]}}
got_ok = bool(d.get("ok"))
fault = d.get("fault") or ("ok" if got_ok else "fail")
detail = d.get("detail") if isinstance(d.get("detail"), dict) else {}
fail_closed = got_ok == expect_ok
if not expect_ok:
  fail_closed = fail_closed and (not got_ok) and fault != "ok"
row = {
  "case": cid, "note": note, "expect_ok": expect_ok, "got_ok": got_ok,
  "fault": fault, "fail_closed": fail_closed, "detail": detail,
  "compact": {"ok": got_ok, "fault": fault, "step": detail.get("step"), "op": detail.get("op")},
}
print(json.dumps(row))
PY
}

run_job() {
  local steps="$1" install_flag="${2:-}"
  local tmp=/tmp/fc-job.json
  local udid
  udid=$("$LIGH" --json status 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("udid") or (d.get("data") or {}).get("udid") or "")' 2>/dev/null || true)
  [[ -n "$udid" ]] || udid=$(xcrun simctl list devices booted | grep -oE '[A-F0-9-]{36}' | head -1 || true)
  if [[ -n "$udid" ]]; then
    xcrun simctl terminate "$udid" "$BUNDLE_ID" 2>/dev/null || true
    sleep 0.3
  fi
  local cmd=("$LIGH" --json cap app-job "$APP" --bundle-id "$BUNDLE_ID" --steps "$steps"
    --settle-ms "$SETTLE_MS" --timeout-ms "$TIMEOUT_MS")
  [[ "$install_flag" == "install" ]] || cmd+=(--no-install)
  for a in "${@:3}"; do cmd+=(--launch-arg="$a"); done
  "${cmd[@]}" >"$tmp" 2>/tmp/fc-err.txt || true
  echo "$tmp"
}

echo "══ fail-closed gate (XCUITestDemo) ══"
sim_clean_reboot "$LIGH"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/fc-first.log 2>&1 || fail "first-loop failed"

ROWS=()
STEPS_OK='[{"op":"wait","id":"usernameTextField"},{"op":"tap","id":"usernameTextField"},{"op":"type","text":"alice"},{"op":"tap","id":"passwordSecureField"},{"op":"type","text":"secret"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"homeTitle"}]'

echo "  ▶ happy_login"
F=$(run_job "$STEPS_OK" install)
ROWS+=("$(eval_case happy_login true "control: valid credentials → homeTitle" "$F")")

echo "  ▶ invalid_credentials"
F=$(run_job '[{"op":"wait","id":"usernameTextField"},{"op":"tap","id":"usernameTextField"},{"op":"type","text":"bob"},{"op":"tap","id":"passwordSecureField"},{"op":"type","text":"wrong"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"homeTitle"}]' "" --ui_test_login_failure)
ROWS+=("$(eval_case invalid_credentials false "launch --ui_test_login_failure → homeTitle must fail" "$F")")

echo "  ▶ missing_username_field"
F=$(run_job '[{"op":"wait","id":"NoSuchUsernameField"},{"op":"tap","id":"usernameTextField"},{"op":"type","text":"alice"},{"op":"tap","id":"passwordSecureField"},{"op":"type","text":"secret"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"homeTitle"}]')
ROWS+=("$(eval_case missing_username_field false "wait nonexistent id at step 0" "$F")")

echo "  ▶ wrong_postcondition"
F=$(run_job '[{"op":"wait","id":"usernameTextField"},{"op":"tap","id":"usernameTextField"},{"op":"type","text":"alice"},{"op":"tap","id":"passwordSecureField"},{"op":"type","text":"secret"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"NoSuchHomeTitle"}]')
ROWS+=("$(eval_case wrong_postcondition false "login ok but final wait id missing" "$F")")

echo "  ▶ empty_credentials"
F=$(run_job '[{"op":"wait","id":"usernameTextField"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"homeTitle"}]')
ROWS+=("$(eval_case empty_credentials false "empty fields → no homeTitle" "$F")")

python3 - "$OUT" "${ROWS[@]}" <<'PY'
import json, sys
out = sys.argv[1]
rows = [json.loads(x) for x in sys.argv[2:]]
passed = [r for r in rows if r.get("fail_closed")]
doc = {
  "gate": "fail_closed",
  "claim": "injected failures return ok=false + explicit fault — never soft-success",
  "app": "XCUITestDemo (OSS third-party)",
  "bundle_id": "com.himali.XCUITestDemo",
  "cases": rows,
  "pass": len(passed),
  "total": len(rows),
  "claim_pass": len(passed) == len(rows),
}
open(out, "w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps({"claim_pass": doc["claim_pass"], "pass": doc["pass"], "total": doc["total"], "out": out}, indent=2))
for r in rows:
  fc = "PASS" if r.get("fail_closed") else "FAIL"
  print("  %s %s expect_ok=%s got_ok=%s fault=%s" % (fc, r["case"], r["expect_ok"], r["got_ok"], r["fault"]))
raise SystemExit(0 if doc["claim_pass"] else 1)
PY
STATUS=$?
echo "══ → $OUT"
exit "$STATUS"
