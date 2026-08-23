#!/usr/bin/env bash
# Compiled replay gate — motor seed → compile → execute (zero LLM).
#
# Builds on what works: QA perceive/attempt records a graph, compile turns intent_met
# edges into motor steps, execute replays without any LLM calls.
#
# Requires: release ligh, Xcode sim. No OPENAI_API_KEY.
#
# Usage:
#   ./scripts/gate-compiled-replay.sh
#   LIGH_REPLAY_APP=xcuitestdemo ./scripts/gate-compiled-replay.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_REPLAY_OUT:-$ROOT/docs/assets/compiled-replay-latest.json}"
TRACE_DIR="${LIGH_REPLAY_TRACES:-$ROOT/docs/assets/compiled-replay-traces}"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

echo "  ▶ build release"
( cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon ) || fail "cargo build failed"
LIGH="$ROOT/target/release/ligh"
[[ -x "$ROOT/target/release/lighd" ]] || fail "missing target/release/lighd"

APP_ID="${LIGH_REPLAY_APP:-lighonboard}"
case "$APP_ID" in
  lighonboard)
    APP_NAME="LighOnboard"
    BUNDLE_ID="dev.ligh.Onboard"
    SUCCESS_ID="HomeReady"
    WORKSPACE="$ROOT/fixtures/LighOnboard"
    BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"
    BUILD_ARG="$APP_NAME"
    APP_PATH="$ROOT/fixtures/$APP_NAME/build/$APP_NAME.app"
    ;;
  xcuitestdemo)
    APP_NAME="XCUITestDemo"
    BUNDLE_ID="com.himali.XCUITestDemo"
    SUCCESS_ID="homeTitle"
    WORKSPACE="$ROOT/fixtures/third-party/XCUITestDemo"
    BUILD_SCRIPT="$ROOT/scripts/build-xcuitestdemo.sh"
    BUILD_ARG=""
    APP_PATH="$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
    ;;
  *)
    fail "unknown LIGH_REPLAY_APP=$APP_ID (lighonboard|xcuitestdemo)"
    ;;
esac

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

mkdir -p "$TRACE_DIR"
rm -rf "$WORKSPACE/.ligh"
mkdir -p "$WORKSPACE/.ligh"

prep_sim() {
  sim_clean_reboot "$LIGH"
  "$ROOT/scripts/agent-first-loop.sh" >/tmp/compiled-replay-first.log 2>&1 \
    || "$LIGH" --json ready --settle-ms 3500 --recover-homes 6 >/dev/null
}

echo "══ Compiled replay gate ($APP_ID) ══"
echo "  seed (motor) → compile → execute (0 LLM tokens)"
echo ""

echo "  ▶ build $APP_NAME"
if [[ -n "$BUILD_ARG" ]]; then
  "$BUILD_SCRIPT" "$BUILD_ARG" >/tmp/compiled-replay-build.log 2>&1 || fail "build failed"
else
  "$BUILD_SCRIPT" >/tmp/compiled-replay-build.log 2>&1 || fail "build failed"
fi
[[ -d "$APP_PATH" ]] || fail "missing $APP_PATH"

echo "══ 1/3 SEED (motor QA, no LLM) ══"
prep_sim
export LIGH_SEED_APP="$APP_ID"
export LIGH_SEED_OUT="$TRACE_DIR/seed.json"
SEED_OK=0
if python3 "$ROOT/scripts/seed-ux-graph.py"; then SEED_OK=1; fi
[[ "$SEED_OK" == "1" ]] || fail "seed failed — see $TRACE_DIR/seed.json"

SEED_MS=$(python3 -c "import json; print(json.load(open('$TRACE_DIR/seed.json'))['total_ms'])")
SEED_NODES=$(python3 -c "import json; print(json.load(open('$TRACE_DIR/seed.json')).get('nodes',0))")

echo ""
echo "══ 2/3 COMPILE (graph → motor steps) ══"
COMPILE_JSON=$(mktemp)
if ! "$LIGH" --json uxgraph compile-flow "$SUCCESS_ID" --workspace "$WORKSPACE" >"$COMPILE_JSON" 2>&1; then
  cat "$COMPILE_JSON" >&2
  fail "compile-flow failed"
fi
COMPILE_STEPS=$(python3 -c "import json; print(json.load(open('$COMPILE_JSON')).get('detail',{}).get('steps',0))")
cp "$COMPILE_JSON" "$TRACE_DIR/compile.json"
ok "compiled $COMPILE_STEPS steps"

echo ""
echo "══ 3/3 EXECUTE (zero LLM replay) ══"
prep_sim
EXEC_T0=$(python3 -c 'import time; print(int(time.time()*1000))')
EXEC_JSON=$(mktemp)
EXEC_OK=0
if "$LIGH" --json uxgraph execute-compiled "$SUCCESS_ID" "$APP_PATH" \
  --bundle-id "$BUNDLE_ID" --workspace "$WORKSPACE" --settle-ms 3500 --timeout-ms 22000 \
  >"$EXEC_JSON" 2>&1; then
  EXEC_OK=1
fi
EXEC_T1=$(python3 -c 'import time; print(int(time.time()*1000))')
EXEC_MS=$((EXEC_T1 - EXEC_T0))
cp "$EXEC_JSON" "$TRACE_DIR/execute.json"

python3 - "$OUT" "$TRACE_DIR" "$SEED_OK" "$SEED_MS" "$SEED_NODES" "$COMPILE_STEPS" "$EXEC_OK" "$EXEC_MS" "$APP_ID" "$SUCCESS_ID" <<'PY'
import json, os, sys

out, trace_dir, s_ok, s_ms, s_nodes, c_steps, e_ok, e_ms, app_id, success_id = sys.argv[1:11]
s_ok = int(s_ok)
s_ms, s_nodes = int(s_ms), int(s_nodes)
c_steps, e_ok, e_ms = int(c_steps), int(e_ok), int(e_ms)

seed = json.load(open(os.path.join(trace_dir, "seed.json")))
execute = json.load(open(os.path.join(trace_dir, "execute.json")))
compile_doc = json.load(open(os.path.join(trace_dir, "compile.json")))

exec_verified = bool(execute.get("ok")) and (execute.get("detail") or {}).get("verified")
exec_tokens = (execute.get("detail") or {}).get("llm_tokens", 0)

claim_pass = (
    s_ok == 1
    and seed.get("verified")
    and s_nodes >= 2
    and c_steps >= 1
    and e_ok == 1
    and exec_verified
    and exec_tokens == 0
)

report = {
    "gate": "compiled_replay",
    "claim": "Motor seed → compile → execute with zero LLM tokens",
    "app_id": app_id,
    "harness_success_id": success_id,
    "phases": {
        "seed": {
            "verified": bool(seed.get("verified")),
            "nodes": s_nodes,
            "edges": seed.get("edges"),
            "ms": s_ms,
            "llm_tokens": 0,
        },
        "compile": {
            "steps": c_steps,
            "confidence": (compile_doc.get("detail") or {}).get("confidence"),
            "path": (compile_doc.get("detail") or {}).get("path"),
        },
        "execute": {
            "verified": exec_verified,
            "ms": e_ms,
            "llm_tokens": exec_tokens,
            "ok": bool(execute.get("ok")),
        },
    },
    "verdict": {
        "seed_ok": s_ok == 1 and seed.get("verified"),
        "compile_ok": c_steps >= 1,
        "execute_ok": e_ok == 1 and exec_verified,
        "zero_llm": exec_tokens == 0,
        "execute_faster_than_seed": e_ms < s_ms,
    },
    "claim_pass": claim_pass,
}
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(0 if claim_pass else 1)
PY

ok "compiled replay gate → $OUT"
echo "══ Compiled replay gate PASS ══"
