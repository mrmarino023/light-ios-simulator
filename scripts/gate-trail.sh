#!/usr/bin/env bash
# TRAIL gate — trace prove + hybrid localize (R1–R3). No golden diff.
#
# Pass: TraceFailure captured + composition path localized within prove budget.
# Full verify awaits cap_repair_job fixer+certify (R4–R5).
#
# Usage:
#   ./scripts/gate-trail.sh
#   LIGH_TRAIL_TASK=fixtures/frozen/tasks/kix-notes-tab-missing/task.json ./scripts/gate-trail.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK="${LIGH_TRAIL_TASK:-${LIGH_REPAIR_JOB_TASK:-$ROOT/fixtures/frozen/tasks/kix-notes-tab-missing/task.json}}"
OUT="${LIGH_TRAIL_OUT:-$ROOT/docs/assets/trail-latest.json}"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
HOT="${LIGH_TRAIL_HOT:-1}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -f "$TASK" ]] || fail "missing task $TASK"

PATCH_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['bug_patch']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
SOURCE_ROOT=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['source_root']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
BUILD_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['build_script']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")

echo "══ TRAIL gate (trace prove + hybrid localize) ══"
echo "  task=$TASK"

INFRA_START=$(python3 -c 'import time; print(int(time.time()*1000))')

if [[ "$HOT" != "1" ]] || [[ ! -x "$LIGH" ]]; then
  echo "  ▶ cargo build release"
  ( cd "$ROOT" && cargo build --release -p ligh-cli -p ligh-daemon ) || fail "cargo build failed"
fi
LIGH="$ROOT/target/release/ligh"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

export LIGH_UI="${LIGH_UI:-sim}"
unset LIGH_WDA_UDID LIGH_WDA_BUNDLE LIGH_WDA_URL LIGH_WDA_SESSION || true

"$LIGH" daemon stop --json >/dev/null 2>&1 || true
pkill -x lighd 2>/dev/null || true
sleep 1
rm -f "${HOME}/.ligh/lighd.sock"
nohup env -u LIGH_WDA_UDID -u LIGH_WDA_BUNDLE -u LIGH_WDA_URL -u LIGH_WDA_SESSION \
  LIGH_UI=sim "$ROOT/target/release/lighd" >>/tmp/lighd-trail.log 2>&1 &
sleep 2
"$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}" >/dev/null 2>&1 || "$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}"

echo "  ▶ warm AX"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/trail-first-loop.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 3500 --recover-homes 6 >/dev/null

BACKUP=$(mktemp -d)
restore_tree() { rsync -a "$BACKUP/" "$SOURCE_ROOT/"; }
trap 'restore_tree' EXIT

echo "  ▶ inject bug + build broken app (infra)"
rsync -a "$SOURCE_ROOT/" "$BACKUP/"
patch -p1 -d "$ROOT" < "$PATCH_ABS"
"$BUILD_ABS" >/tmp/trail-build-broken.log 2>&1 || fail "build broken app failed"

INFRA_MS=$(python3 -c "import time; print(int(time.time()*1000) - $INFRA_START)")

unset LIGH_IDENTITY_SOURCE || true
export LIGH_KILLER_TASK="$TASK"
export LIGH_APP_PATH="$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['app_path']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")"
export LIGH_APP_BUNDLE_ID="$(python3 -c "import json; print(json.load(open('$TASK'))['bundle_id'])")"
export LIGH_TRAIL_INFRA_MS="$INFRA_MS"
export LIGH_TRAIL_T0_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
export LIGH_TRAIL_OUT="$OUT"
export LIGH_REPAIR_JOB_TASK="$TASK"
export LIGH_BIN="$LIGH"

PASS=0
if python3 "$ROOT/scripts/repair_job.py"; then PASS=1; fi

python3 - "$OUT" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
ok = bool(d.get("localization_ok"))
mark = "✓" if ok else "✗"
budget = "ok" if d.get("within_prove_budget") else "soft_over"
print(f"   {mark} localize={d.get('localization_ok')} trail_wall={d.get('trail_wall_ms')}ms prove_budget={budget} mode={d.get('repair_bundle',{}).get('mode')} primary={d.get('repair_bundle',{}).get('scope',{}).get('primary_path')} identity={d.get('failed_identity')}")
print(f"   verified={d.get('verified')} (fixer+certify pending)")
PY

[[ "$PASS" -eq 1 ]] || fail "TRAIL gate failed — see $OUT"
echo "✓ TRAIL R1–R3 → $OUT"
