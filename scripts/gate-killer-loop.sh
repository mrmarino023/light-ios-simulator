#!/usr/bin/env bash
# Killer loop gate — frozen OSS task, inject bug, agent fix+verify.
#
# Agent sees task.json only. ground-truth.json is for humans/scoring — never in prompt.
#
# Usage:
#   ./scripts/gate-killer-loop.sh
#   LIGH_KILLER_ARM=baseline ./scripts/gate-killer-loop.sh
#   LIGH_KILLER_ARM=hybrid ./scripts/gate-killer-loop.sh
#   LIGH_KILLER_AB_HYBRID=1 ./scripts/gate-killer-loop-ab.sh   # A + B + hybrid
#   LIGH_KILLER_HONEST=1 ./scripts/gate-killer-loop-ab.sh      # XCUITestDemo login, no exercise_app
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ "${LIGH_KILLER_HONEST:-0}" == "1" ]]; then
  TASK="${LIGH_KILLER_TASK:-$ROOT/fixtures/frozen/tasks/login-never-navigates/task.json}"
else
  TASK="${LIGH_KILLER_TASK:-$ROOT/fixtures/frozen/tasks/onboarding-home-broken/task.json}"
fi
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
ARM="${LIGH_KILLER_ARM:-ligh}"
OUT="${LIGH_KILLER_OUT:-$ROOT/docs/assets/killer-loop-${ARM}-latest.json}"

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

[[ -f "$TASK" ]] || fail "missing task $TASK"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

PATCH=$(python3 -c "import json; print(json.load(open('$TASK'))['bug_patch'])")
BUILD=$(python3 -c "import json; print(json.load(open('$TASK'))['build_script'])")
SOURCE_ROOT=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['source_root']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
PATCH_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['bug_patch']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
BUILD_ABS=$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['build_script']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")
[[ -f "$PATCH_ABS" ]] || fail "missing bug patch $PATCH_ABS"

echo "  ▶ build release ligh"
( cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon ) || fail "cargo build failed"
LIGH="$ROOT/target/release/ligh"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

BACKUP=$(mktemp -d)
restore_tree() {
  rsync -a "$BACKUP/" "$SOURCE_ROOT/"
}
trap 'restore_tree' EXIT

echo "══ Killer loop (arm=$ARM) ══"
echo "  task=$TASK"
echo "  ground-truth: $(dirname "$TASK")/ground-truth.json (NOT sent to agent)"

echo "  ▶ backup frozen sources"
rsync -a "$SOURCE_ROOT/" "$BACKUP/"

echo "  ▶ inject bug (human-only ground truth in ground-truth.json)"
patch -p1 -d "$ROOT" < "$PATCH_ABS"
"$BUILD_ABS" >/tmp/killer-loop-build-broken.log 2>&1 || fail "build broken app failed"

sim_clean_reboot "$LIGH"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/killer-loop-first.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 3500 --recover-homes 6 >/dev/null

export LIGH_KILLER_TASK="$TASK"
export LIGH_KILLER_ARM="$ARM"
export LIGH_KILLER_OUT="$OUT"
export LIGH_KILLER_HONEST="${LIGH_KILLER_HONEST:-0}"
# Killer / holy-shit are simulator product proofs. Never let a live DevDriver
# (e.g. Mae on LAN) steal AX dump under LIGH_UI=auto.
export LIGH_UI="${LIGH_UI:-sim}"
export LIGH_APP_PATH="$(python3 -c "import json,os; t=json.load(open('$TASK')); p=t['app_path']; print(p if os.path.isabs(p) else os.path.join('$ROOT', p))")"
export LIGH_APP_BUNDLE_ID="$(python3 -c "import json; print(json.load(open('$TASK'))['bundle_id'])")"
# ~/.ligh/wda.env often points at a phone (Mae). Strip WDA identity in sim mode
# so a restarted daemon cannot warm physical arms mid-killer.
if [[ "$LIGH_UI" == "sim" || "$LIGH_UI" == "simulator" ]]; then
  unset LIGH_WDA_UDID LIGH_WDA_BUNDLE LIGH_WDA_URL LIGH_WDA_SESSION || true
fi

# Daemon must inherit LIGH_UI=sim — restart if a physical session could be live.
if [[ "${LIGH_KILLER_RESTART_DAEMON:-1}" == "1" ]]; then
  "$LIGH" daemon stop --json >/dev/null 2>&1 || true
  pkill -x lighd 2>/dev/null || true
  sleep 1
  nohup env -u LIGH_WDA_UDID -u LIGH_WDA_BUNDLE -u LIGH_WDA_URL -u LIGH_WDA_SESSION \
    LIGH_UI=sim "$ROOT/target/release/lighd" >>/tmp/lighd-killer.log 2>&1 &
  sleep 2
  "$LIGH" up --device iphone-15-pro >/dev/null 2>&1 || "$LIGH" up --device iphone-15-pro
fi

echo "  ▶ protocol $([ "${LIGH_KILLER_HONEST}" = 1 ] && echo honest || echo product) v2: verify deterministic initial state (preconditions) [LIGH_UI=$LIGH_UI]"
python3 "$ROOT/scripts/killer_loop_verify.py" --task "$TASK" --phase setup \
  >/tmp/killer-loop-initial-state.log 2>&1 || fail "initial state setup failed — see /tmp/killer-loop-initial-state.log"

PASS=0
if python3 "$ROOT/scripts/autonomous-killer-loop-agent.py"; then PASS=1; fi

restore_tree
trap - EXIT
"$BUILD_ABS" >/tmp/killer-loop-restore.log 2>&1 || true

python3 - "$OUT" "$PASS" "$ARM" "$TASK" <<'PY'
import json, sys
out, passed, arm, task = sys.argv[1:5]
doc = json.load(open(out))
doc["source_restored"] = True
doc["claim_pass"] = bool(int(passed)) and doc.get("verified") and not doc.get("false_success")
doc["task_file"] = task
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({
    "arm": arm,
    "claim_pass": doc["claim_pass"],
    "verified": doc.get("verified"),
    "false_success": doc.get("false_success"),
    "verification_reason": doc.get("verification_reason"),
    "legacy_weak_pass": doc.get("legacy_weak_pass"),
    "build_attempts": doc.get("build_attempts"),
    "final_state": doc.get("final_state"),
    "tokens": doc.get("llm_tokens"),
    "out": out,
}, indent=2))
raise SystemExit(0 if doc["claim_pass"] else 1)
PY

ok "killer loop ($ARM) → $OUT"
