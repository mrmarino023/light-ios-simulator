#!/usr/bin/env bash
# TRAIL end-to-end: trace prove/localize -> constrained fixer -> build -> strict verify.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK="${LIGH_TRAIL_TASK:-$ROOT/fixtures/frozen/tasks/kix-notes-tab-missing/task.json}"
TRAIL_OUT="${LIGH_TRAIL_OUT:-$ROOT/docs/assets/trail-latest.json}"
FIX_OUT="${LIGH_TRAIL_FIX_OUT:-$ROOT/docs/assets/trail-fix-latest.json}"
VERIFY_OUT="${LIGH_TRAIL_VERIFY_OUT:-$ROOT/docs/assets/trail-verify-latest.json}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -f "$TASK" ]] || fail "missing task $TASK"

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

SOURCE_ROOT=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['source_root']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")

echo "══ TRAIL fix gate ══"
echo "  task=$TASK"

BACKUP=$(mktemp -d)
restore_tree() { rsync -a "$BACKUP/" "$SOURCE_ROOT/"; }
trap 'restore_tree' EXIT
rsync -a "$SOURCE_ROOT/" "$BACKUP/"

"$ROOT/scripts/gate-trail.sh" || fail "gate-trail failed"
python3 "$ROOT/scripts/trail_fixer.py" --trail "$TRAIL_OUT" --out "$FIX_OUT" || fail "trail_fixer produced no change"

BUILD_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['build_script']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
"$BUILD_ABS" >/tmp/trail-fix-build.log 2>&1 || fail "build failed"

export LIGH_KILLER_TASK="$TASK"
export LIGH_APP_PATH="$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['app_path']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")"
export LIGH_APP_BUNDLE_ID="$(python3 -c "import json; print(json.load(open('$TASK'))['bundle_id'])")"

python3 "$ROOT/scripts/killer_loop_verify.py" --task "$TASK" >"$VERIFY_OUT" || true
python3 - "$VERIFY_OUT" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
print(json.dumps({
    "verified": d.get("verified"),
    "reason": d.get("reason"),
    "exercise": [(s.get("action"), s.get("id"), s.get("ok")) for s in d.get("exercise_trace", [])],
}, indent=2))
sys.exit(0 if d.get("verified") else 1)
PY
