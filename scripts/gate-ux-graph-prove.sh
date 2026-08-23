#!/usr/bin/env bash
# EXPERIMENTAL — UX graph proof gate (research only, not merge-blocking).
#
# Compares LLM discover vs motor seed, then compile → execute (zero LLM on replay).
# The canonical product gates are gate-autonomous-ux.sh (LLM QA loop) and
# gate-compiled-replay.sh (motor replay, no LLM).
#
# Phase 1 default: seed (motor). Set LIGH_PROVE_PHASE1=discover for LLM graph build.
#
# Usage:
#   ./scripts/gate-ux-graph-prove.sh
#   LIGH_PROVE_PHASE1=discover ./scripts/gate-ux-graph-prove.sh
#   LIGH_PROVE_APP=xcuitestdemo ./scripts/gate-ux-graph-prove.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_UX_PROVE_OUT:-$ROOT/docs/assets/ux-graph-prove-latest.json}"
TRACE_DIR="${LIGH_UX_PROVE_TRACES:-$ROOT/docs/assets/ux-graph-prove-traces}"
PHASE1="${LIGH_PROVE_PHASE1:-seed}"

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

if [[ "$PHASE1" == "discover" ]]; then
  [[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required for discover phase"
fi

echo "  ▶ build release"
( cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon ) || fail "cargo build failed"
LIGH="$ROOT/target/release/ligh"

APP_ID="${LIGH_PROVE_APP:-lighonboard}"
case "$APP_ID" in
  lighonboard)
    APP_NAME="LighOnboard"
    BUNDLE_ID="dev.ligh.Onboard"
    SUCCESS_ID="HomeReady"
    WORKSPACE="$ROOT/fixtures/LighOnboard"
    GOAL='Complete onboarding until the home-ready screen. Use perceive before every action. Do not guess accessibility ids.'
    BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"
    BUILD_ARG="$APP_NAME"
    APP_PATH="$ROOT/fixtures/$APP_NAME/build/$APP_NAME.app"
    ;;
  xcuitestdemo)
    APP_NAME="XCUITestDemo"
    BUNDLE_ID="com.himali.XCUITestDemo"
    SUCCESS_ID="homeTitle"
    WORKSPACE="$ROOT/fixtures/third-party/XCUITestDemo"
    GOAL='Log in with username "alice" and password "secret" until the authenticated home screen. Use perceive before every action.'
    BUILD_SCRIPT="$ROOT/scripts/build-xcuitestdemo.sh"
    BUILD_ARG=""
    APP_PATH="$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
    ;;
  *)
    fail "unknown LIGH_PROVE_APP=$APP_ID (lighonboard|xcuitestdemo)"
    ;;
esac

echo "  ▶ build $APP_NAME"
if [[ -n "$BUILD_ARG" ]]; then
  "$BUILD_SCRIPT" "$BUILD_ARG" >/tmp/ux-prove-build.log 2>&1 || fail "build failed"
else
  "$BUILD_SCRIPT" >/tmp/ux-prove-build.log 2>&1 || fail "build failed"
fi
[[ -d "$APP_PATH" ]] || fail "missing $APP_PATH"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

mkdir -p "$TRACE_DIR"
rm -rf "$WORKSPACE/.ligh"
mkdir -p "$WORKSPACE/.ligh"

prep_sim() {
  sim_clean_reboot "$LIGH"
  "$ROOT/scripts/agent-first-loop.sh" >/tmp/ux-prove-first.log 2>&1 \
    || "$LIGH" --json ready --settle-ms 3500 --recover-homes 6 >/dev/null
}

echo "══ UX graph PROOF gate [EXPERIMENTAL] ($APP_ID) ══"
echo "  phase1=$PHASE1 → compile → execute (0 tokens on replay)"
echo ""

PHASE1_OK=0
PHASE1_MS=0
PHASE1_NODES=0
PHASE1_TOKENS=0
PHASE1_DOC="$TRACE_DIR/phase1.json"

echo "══ 1/3 PHASE1 ($PHASE1) ══"
prep_sim

if [[ "$PHASE1" == "discover" ]]; then
  export LIGH_UX_ARM=discover
  export LIGH_APP_PATH="$APP_PATH"
  export LIGH_APP_BUNDLE_ID="$BUNDLE_ID"
  export LIGH_WORKSPACE="$WORKSPACE"
  export LIGH_UX_SUCCESS_ID="$SUCCESS_ID"
  export LIGH_UX_GOAL="$GOAL"
  export LIGH_UX_AGENT_OUT="$PHASE1_DOC"
  export LIGH_UX_MAX_STEPS="${LIGH_UX_MAX_STEPS:-24}"
  if python3 "$ROOT/scripts/autonomous-ux-agent.py"; then PHASE1_OK=1; fi
  cp "$PHASE1_DOC" "$TRACE_DIR/discover.json"
else
  export LIGH_SEED_APP="$APP_ID"
  export LIGH_SEED_OUT="$PHASE1_DOC"
  if python3 "$ROOT/scripts/seed-ux-graph.py"; then PHASE1_OK=1; fi
  cp "$PHASE1_DOC" "$TRACE_DIR/seed.json"
fi

[[ "$PHASE1_OK" == "1" ]] || fail "phase1 ($PHASE1) failed — see $PHASE1_DOC"

PHASE1_MS=$(python3 -c "import json; d=json.load(open('$PHASE1_DOC')); print(d.get('total_ms',0))")
if [[ "$PHASE1" == "discover" ]]; then
  PHASE1_NODES=$(python3 -c "import json; d=json.load(open('$PHASE1_DOC')); print((d.get('harness_verify') or {}).get('ux_graph',{}).get('node_count',0))")
  PHASE1_TOKENS=$(python3 -c "import json; d=json.load(open('$PHASE1_DOC')); print((d.get('tokens') or {}).get('total',0))")
else
  PHASE1_NODES=$(python3 -c "import json; d=json.load(open('$PHASE1_DOC')); print(d.get('nodes',0))")
  PHASE1_TOKENS=0
fi

echo ""
echo "══ 2/3 COMPILE (graph → motor steps) ══"
COMPILE_JSON=$(mktemp)
if ! "$LIGH" --json uxgraph compile-flow "$SUCCESS_ID" --workspace "$WORKSPACE" >"$COMPILE_JSON" 2>&1; then
  cat "$COMPILE_JSON" >&2
  fail "compile-flow failed"
fi
COMPILE_STEPS=$(python3 -c "import json; print(json.load(open('$COMPILE_JSON')).get('detail',{}).get('steps',0))")
cp "$COMPILE_JSON" "$TRACE_DIR/compile.json"
ok "compiled $COMPILE_STEPS steps → $WORKSPACE/.ligh/compiled/$SUCCESS_ID.json"

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

python3 - "$OUT" "$TRACE_DIR" "$PHASE1" "$PHASE1_OK" "$PHASE1_MS" "$PHASE1_NODES" "$PHASE1_TOKENS" "$COMPILE_STEPS" "$EXEC_OK" "$EXEC_MS" "$APP_ID" "$SUCCESS_ID" <<'PY'
import json, os, sys

(out, trace_dir, phase1, p1_ok, p1_ms, p1_nodes, p1_tokens,
 c_steps, e_ok, e_ms, app_id, success_id) = sys.argv[1:13]
p1_ok = int(p1_ok)
p1_ms, p1_nodes, p1_tokens = int(p1_ms), int(p1_nodes), int(p1_tokens)
c_steps, e_ok, e_ms = int(c_steps), int(e_ok), int(e_ms)

phase1_doc = json.load(open(os.path.join(trace_dir, "phase1.json")))
execute = json.load(open(os.path.join(trace_dir, "execute.json")))
compile_doc = json.load(open(os.path.join(trace_dir, "compile.json")))

exec_verified = bool(execute.get("ok")) and (execute.get("detail") or {}).get("verified")
exec_tokens = (execute.get("detail") or {}).get("llm_tokens", 0)

claim_pass = (
    p1_ok == 1
    and p1_nodes >= 2
    and c_steps >= 1
    and e_ok == 1
    and exec_verified
    and exec_tokens == 0
)

report = {
    "gate": "ux_graph_prove_v2",
    "experimental": True,
    "claim": "Graph → compile → execute replay with zero LLM tokens on replay",
    "app_id": app_id,
    "harness_success_id": success_id,
    "phase1": phase1,
    "phase1_llm_tokens": p1_tokens if phase1 == "discover" else 0,
    "phases": {
        "phase1": {
            "mode": phase1,
            "verified": bool(phase1_doc.get("verified")),
            "nodes": p1_nodes,
            "ms": p1_ms,
            "tokens": phase1_doc.get("tokens") if phase1 == "discover" else {"total": 0},
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
        "phase1_ok": p1_ok == 1,
        "compile_ok": c_steps >= 1,
        "execute_ok": e_ok == 1 and exec_verified,
        "zero_llm_replay": exec_tokens == 0,
        "execute_faster_than_phase1": e_ms < p1_ms,
        "speedup_ratio": round(p1_ms / max(e_ms, 1), 2),
    },
    "claim_pass": claim_pass,
    "honest_if_fail": "Publish JSON anyway — negative results are valuable. Canonical gates: gate-autonomous-ux.sh, gate-compiled-replay.sh",
}
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(0 if claim_pass else 1)
PY

ok "PROOF gate → $OUT"
echo "══ UX graph PROOF gate done ══"
