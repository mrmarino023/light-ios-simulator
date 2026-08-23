#!/usr/bin/env bash
# Canonical autonomous UX gate — perceive/attempt loop, harness verifies success id.
#
# Default arm is control (QA only, no graph). The LLM reads affordances via perceive/attempt;
# the harness independently checks success_id (agent prompt never names it).
#
# Requires: OPENAI_API_KEY, release ligh, Xcode sim, ~500MB free disk for LighOnboard build.
#
# Usage:
#   ./scripts/gate-autonomous-ux.sh
#   LIGH_UX_APP=xcuitestdemo ./scripts/gate-autonomous-ux.sh
#   LIGH_UX_ARM=discover ./scripts/gate-autonomous-ux.sh   # also record ux graph (experimental)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_UX_AGENT_OUT:-$ROOT/docs/assets/autonomous-ux-latest.json}"

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

[[ -x "$LIGH" ]] || fail "build release ligh first (unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon)"

echo "  ▶ sync release ligh + lighd"
( cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon ) \
  || fail "cargo build failed"
LIGH="$ROOT/target/release/ligh"
[[ -x "$ROOT/target/release/lighd" ]] || fail "missing target/release/lighd"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required (export or $ROOT/.env)"

APP_ID="${LIGH_UX_APP:-lighonboard}"
case "$APP_ID" in
  lighonboard)
    APP_NAME="LighOnboard"
    BUNDLE_ID="dev.ligh.Onboard"
    SUCCESS_ID="HomeReady"
    WORKSPACE="$ROOT/fixtures/LighOnboard"
    GOAL="Complete the onboarding flow until the home-ready screen appears. Use perceive to read ids; attempt with expect after each tap."
    ;;
  lighmodal)
    APP_NAME="LighModal"
    BUNDLE_ID="dev.ligh.Modal"
    SUCCESS_ID="ModalConfirmed"
    WORKSPACE="$ROOT/fixtures/LighModal"
    GOAL="Open the sheet and confirm the modal action until the confirmed state appears. Discover controls via perceive only."
    ;;
  lighfeed)
    APP_NAME="LighFeed"
    BUNDLE_ID="dev.ligh.Feed"
    SUCCESS_ID="PostDetail"
    WORKSPACE="$ROOT/fixtures/LighFeed"
    GOAL="From the feed, reach a post detail screen. Scroll with find if targets are off-screen."
    ;;
  lighfixture)
    APP_NAME="LighFixture"
    BUNDLE_ID="dev.ligh.Fixture"
    SUCCESS_ID="LighDone"
    WORKSPACE="$ROOT/fixtures/LighFixture"
    GOAL="Navigate from home to the done screen using QA tools only."
    ;;
  xcuitestdemo)
    APP_NAME="XCUITestDemo"
    BUNDLE_ID="com.himali.XCUITestDemo"
    SUCCESS_ID="homeTitle"
    WORKSPACE="$ROOT/fixtures/third-party/XCUITestDemo"
    GOAL="Log in with username alice and password secret until the home screen appears. No app_job — only perceive/attempt."
    ;;
  *)
    fail "unknown LIGH_UX_APP=$APP_ID (lighonboard|lighmodal|lighfeed|lighfixture|xcuitestdemo)"
    ;;
esac

if [[ "$APP_ID" == "xcuitestdemo" ]]; then
  BUILD_SCRIPT="$ROOT/scripts/build-xcuitestdemo.sh"
  APP_PATH="$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
else
  BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"
  APP_PATH="$ROOT/fixtures/$APP_NAME/build/$APP_NAME.app"
fi

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

UX_ARM="${LIGH_UX_ARM:-control}"

echo "══ Autonomous UX gate (canonical) ══"
echo "  app=$APP_ID bundle=$BUNDLE_ID success_id=$SUCCESS_ID arm=$UX_ARM"
echo "  workspace=$WORKSPACE"

echo "  ▶ build $APP_NAME"
if [[ "$APP_ID" == "xcuitestdemo" ]]; then
  "$BUILD_SCRIPT" >/tmp/autonomous-ux-build.log 2>&1 || {
    tail -30 /tmp/autonomous-ux-build.log >&2
    fail "build failed (disk full? see /tmp/autonomous-ux-build.log)"
  }
else
  "$BUILD_SCRIPT" "$APP_NAME" >/tmp/autonomous-ux-build.log 2>&1 || {
    tail -30 /tmp/autonomous-ux-build.log >&2
    fail "build failed (disk full? see /tmp/autonomous-ux-build.log)"
  }
fi
[[ -d "$APP_PATH" ]] || fail "missing $APP_PATH"

echo "  ▶ cold sim + daemon"
sim_clean_reboot "$LIGH"
if ! "$ROOT/scripts/agent-first-loop.sh" >/tmp/autonomous-ux-first.log 2>&1; then
  "$LIGH" --json ready --settle-ms 3500 --recover-homes 6 >/tmp/autonomous-ux-ready.log 2>&1 \
    || fail "ligh_ready failed"
fi

if [[ "$UX_ARM" != "control" ]]; then
  echo "  ▶ fresh ux graph (discover/replay arm)"
  rm -rf "$WORKSPACE/.ligh"
  mkdir -p "$WORKSPACE/.ligh"
fi

export LIGH_UX_ARM="$UX_ARM"
export LIGH_APP_PATH="$APP_PATH"
export LIGH_APP_BUNDLE_ID="$BUNDLE_ID"
export LIGH_WORKSPACE="$WORKSPACE"
export LIGH_UX_SUCCESS_ID="$SUCCESS_ID"
export LIGH_UX_GOAL="$GOAL"
export LIGH_UX_AGENT_OUT="$OUT"

echo "  ▶ autonomous UX agent (arm=$UX_ARM, no scripted steps)"
PASS=0
if python3 "$ROOT/scripts/autonomous-ux-agent.py"; then
  PASS=1
  ok "autonomous UX verified"
else
  echo "  FAIL — harness did not see $SUCCESS_ID (arm=$UX_ARM)"
fi

python3 - <<PY
import json, os
p = "$OUT"
doc = json.load(open(p)) if os.path.isfile(p) else {}
doc["gate_env"] = "mac_integration"
doc["app_id"] = "$APP_ID"
doc["arm"] = "$UX_ARM"
doc["claim_pass"] = bool(int("$PASS"))
open(p, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({
  "claim_pass": doc.get("claim_pass"),
  "verified": doc.get("verified"),
  "graph_grew": doc.get("graph_grew"),
  "steps": doc.get("steps_used"),
  "out": p,
}, indent=2))
PY

[[ "$PASS" == "1" ]] || exit 1
ok "wrote $OUT"
echo "══ Autonomous UX gate PASS ══"
