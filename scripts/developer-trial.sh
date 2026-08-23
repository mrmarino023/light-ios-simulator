#!/usr/bin/env bash
# Developer trial: install smoke + MCP + app-job + agent loop.
#
# Usage:
#   ./scripts/developer-trial.sh
#   LIGH_APP_PATH=…/My.app LIGH_APP_BUNDLE_ID=com.you LIGH_APP_STEPS='[…]' ./scripts/developer-trial.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_DEV_TRIAL_OUT:-$ROOT/docs/assets/developer-trial-local.json}"

fail() { echo "✗ $*" >&2; exit 1; }

echo "══ LIGH developer trial ══"
echo "  docs: $ROOT/docs/DEVELOPER_TRIAL.md · $ROOT/docs/AGENT_ENV.md"
echo

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS required"
xcode-select -p &>/dev/null || fail "Xcode/CLT required"

ensure_binaries() {
  if [[ ! -x "$LIGH" ]] || [[ ! -x "${LIGH%d/ligh}/lighd" ]]; then
    echo "  ▶ building release…"
    (cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release --locked -p ligh-cli -p ligh-daemon) \
      || fail "cargo build failed"
  fi
  [[ -x "$LIGH" ]] || fail "ligh missing at $LIGH (unset CARGO_TARGET_DIR && cargo build --release)"
  "$LIGH" daemon stop >/dev/null 2>&1 || true
  pkill -x lighd 2>/dev/null || true
  sleep 1
  "$LIGH" daemon start
}

ensure_binaries

APP="${LIGH_APP_PATH:-$ROOT/fixtures/LighFixture/build/LighFixture.app}"
BID="${LIGH_APP_BUNDLE_ID:-dev.ligh.Fixture}"
if [[ -z "${LIGH_APP_STEPS:-}" ]]; then
  STEPS='[{"op":"wait","id":"LighHome"},{"op":"wait","id":"NameField"},{"op":"type","text":"dev"},{"op":"tap","id":"GoNext"},{"op":"wait","id":"LighDone"}]'
else
  STEPS="$LIGH_APP_STEPS"
fi

[[ -d "$APP" ]] || "$ROOT/scripts/build-fixture.sh" >/tmp/dev-trial-build.log 2>&1

T0=$(python3 -c 'import time; print(time.time())')
"$LIGH" up --device iphone-15-pro >/dev/null 2>&1 || "$LIGH" up
if ! "$ROOT/scripts/agent-first-loop.sh" >/tmp/dev-trial-first.log 2>&1; then
  echo "  ▶ SpringBoard wake slow — ligh_ready"
  "$LIGH" --json ready --settle-ms 4000 --recover-homes 6 >/tmp/dev-trial-ready.json 2>&1 \
    || fail "ligh_ready failed — see /tmp/dev-trial-ready.json"
fi

echo "  ▶ app-job smoke"
JOB_FILE=/tmp/dev-trial-job.json
"$LIGH" --json cap app-job "$APP" --bundle-id "$BID" --steps "$STEPS" \
  --settle-ms 4000 --timeout-ms 20000 >"$JOB_FILE" 2>/tmp/dev-trial-job.err || true
JOB=$(cat "$JOB_FILE" 2>/dev/null || echo "{}")

echo "  ▶ agent-unified-loop (scripted)"
LOOP_OK=0
python3 "$ROOT/scripts/agent-unified-loop.py" >/tmp/dev-trial-loop.log 2>&1 && LOOP_OK=1 || true

python3 - "$OUT" "$T0" "$APP" "$BID" "$JOB" "$LOOP_OK" <<'PY'
import json, sys, time, platform
out, t0, app, bid, raw, loop_ok = sys.argv[1:7]
try:
  job = json.loads(raw) if raw.strip() else {}
except Exception:
  job = {"parse_error": True, "raw_head": raw[:300]}
doc = {
  "trial": "developer_smoke",
  "ok": bool(job.get("ok")) and loop_ok == "1",
  "app_job_ok": bool(job.get("ok")),
  "agent_loop_ok": loop_ok == "1",
  "fault": job.get("fault"),
  "app": app,
  "bundle_id": bid,
  "platform": platform.platform(),
  "total_ms": int((time.time()-float(t0))*1000),
  "job_compact": {
    "ok": job.get("ok"),
    "fault": job.get("fault"),
    "step": (job.get("detail") or {}).get("step") if isinstance(job.get("detail"), dict) else None,
  },
}
open(out, "w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps({"ok": doc["ok"], "app_job": doc["app_job_ok"], "agent_loop": doc["agent_loop_ok"], "out": out}, indent=2))
PY

echo
echo "── Cursor MCP (LIGH) ──"
"$ROOT/scripts/print-cursor-mcp.sh"
echo
echo "── Validate full environment ──"
echo "  ./scripts/gate-agent-environment.sh"
echo
echo "── Agent prompt ──"
echo "  See: $ROOT/docs/CURSOR_PROMPT.md"
echo
echo "── Feedback ──"
echo "  ./scripts/developer-feedback.sh"
echo "══ trial complete ══"
