#!/usr/bin/env bash
# TRAIL holy-shit gate — unified hot path, no golden/template.
#
# Wall = prove+fix+build+certify (infra inject/build broken excluded).
# Pass: verified && wall_ms ≤ LIGH_TRAIL_WALL_MS (default 120000).
#
# Usage:
#   ./scripts/gate-trail-holy.sh
#   LIGH_TRAIL_TASK=fixtures/frozen/tasks/login-never-navigates/task.json ./scripts/gate-trail-holy.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK="${LIGH_TRAIL_TASK:-$ROOT/fixtures/frozen/tasks/kix-notes-tab-missing/task.json}"
OUT="${LIGH_TRAIL_HOLY_OUT:-$ROOT/docs/assets/trail-holy-latest.json}"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
REUSE="${LIGH_TRAIL_REUSE_SESSION:-0}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -f "$TASK" ]] || fail "missing task $TASK"

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

PATCH_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['bug_patch']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
SOURCE_ROOT=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['source_root']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
BUILD_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['build_script']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")

echo "══ TRAIL holy-shit ══"
echo "  task=$TASK"

INFRA_START=$(python3 -c 'import time; print(int(time.time()*1000))')

if [[ ! -x "$ROOT/target/release/ligh" ]]; then
  ( cd "$ROOT" && cargo build --release -p ligh-cli -p ligh-daemon ) || fail "cargo build failed"
fi
LIGH="$ROOT/target/release/ligh"

export LIGH_UI="${LIGH_UI:-sim}"
unset LIGH_WDA_UDID LIGH_WDA_BUNDLE LIGH_WDA_URL LIGH_WDA_SESSION || true

if [[ "$REUSE" != "1" ]] || ! "$LIGH" daemon status >/dev/null 2>&1; then
  "$LIGH" daemon stop --json >/dev/null 2>&1 || true
  pkill -x lighd 2>/dev/null || true
  sleep 0.5
  rm -f "${HOME}/.ligh/lighd.sock"
  nohup env -u LIGH_WDA_UDID -u LIGH_WDA_BUNDLE -u LIGH_WDA_URL -u LIGH_WDA_SESSION \
    LIGH_UI=sim "$ROOT/target/release/lighd" >>/tmp/lighd-trail-holy.log 2>&1 &
  sleep 1.5
  "$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}" >/dev/null 2>&1 || "$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}"
  # Fast warm — skip long agent-first loop when AX already ready.
  if ! "$LIGH" --json ready --settle-ms 1200 --recover-homes 2 >/dev/null 2>&1; then
    "$ROOT/scripts/agent-first-loop.sh" >/tmp/trail-holy-warm.log 2>&1 || true
  fi
else
  "$LIGH" --json ready --settle-ms 800 --recover-homes 1 >/dev/null 2>&1 || true
fi

BACKUP=$(mktemp -d)
restore_tree() { rsync -a "$BACKUP/" "$SOURCE_ROOT/"; }
trap 'restore_tree' EXIT

echo "  ▶ inject + build broken (infra)"
rsync -a "$SOURCE_ROOT/" "$BACKUP/"
patch -p1 -d "$ROOT" < "$PATCH_ABS"
"$BUILD_ABS" >/tmp/trail-holy-build-broken.log 2>&1 || fail "build broken failed"

INFRA_MS=$(python3 -c "import time; print(int(time.time()*1000) - $INFRA_START)")

export LIGH_IDENTITY_SOURCE="$BACKUP"
export LIGH_KILLER_TASK="$TASK"
export LIGH_TRAIL_TASK="$TASK"
export LIGH_APP_PATH="$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['app_path']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")"
export LIGH_APP_BUNDLE_ID="$(python3 -c "import json; print(json.load(open('$TASK'))['bundle_id'])")"
export LIGH_TRAIL_INFRA_MS="$INFRA_MS"
export LIGH_TRAIL_HOLY_OUT="$OUT"
export LIGH_TRAIL_FAST=1
export LIGH_TRAIL_SETTLE_CAP_MS="${LIGH_TRAIL_SETTLE_CAP_MS:-1200}"
export LIGH_BIN="$LIGH"
export LIGH_TRAIL_WALL_MS="${LIGH_TRAIL_WALL_MS:-120000}"

PASS=0
if python3 "$ROOT/scripts/trail_holy.py"; then PASS=1; fi

python3 - "$OUT" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
mark = "✓" if d.get("holy_shit") else ("~" if d.get("verified") else "✗")
print(f"   {mark} verified={d.get('verified')} holy_shit={d.get('holy_shit')} wall={d.get('wall_ms')}ms infra={d.get('infra_ms')}ms mode={d.get('mode')} path={d.get('primary_path')} tokens={d.get('llm_tokens')}")
PY

[[ "$PASS" -eq 1 ]] || fail "trail holy failed — see $OUT"
echo "✓ TRAIL holy → $OUT"
