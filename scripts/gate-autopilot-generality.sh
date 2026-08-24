#!/usr/bin/env bash
# Autopilot generality gate — one policy, many apps, zero per-app steps.
#
# The falsifiable claim: `ligh cap autopilot` reaches an acceptance target on
# apps with structurally different flows (form, wizard, sheet, list drill-down,
# login) using only the target id plus typed data. No recorded path, no label
# list, no app-specific branch anywhere in the host.
#
# If any app needs a special case, the architecture has failed — that is the point.
#
# Requires: release ligh, Xcode sim. No OPENAI_API_KEY (zero LLM tokens).
#
# Usage:
#   ./scripts/gate-autopilot-generality.sh
#   LIGH_PILOT_APPS="lighfixture lighmodal" ./scripts/gate-autopilot-generality.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${LIGH_PILOT_OUT:-$ROOT/docs/assets/autopilot-generality-latest.json}"
TRACE_DIR="${LIGH_PILOT_TRACES:-$ROOT/docs/assets/autopilot-generality-traces}"
APPS="${LIGH_PILOT_APPS:-lighfixture lighonboard lighmodal lighfeed xcuitestdemo kix}"
MAX_STEPS="${LIGH_PILOT_MAX_STEPS:-24}"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

echo "  ▶ build release"
( cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon ) \
  || fail "cargo build failed"
LIGH="$ROOT/target/release/ligh"
[[ -x "$ROOT/target/release/lighd" ]] || fail "missing target/release/lighd"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

mkdir -p "$TRACE_DIR" "$(dirname "$OUT")"

# Per app: NAME BUNDLE GOAL_ID FLOW_SHAPE PARAMS…
# GOAL_ID is the acceptance target (the task spec). PARAMS are data. Nothing else.
app_config() {
  case "$1" in
    lighfixture)
      NAME="LighFixture"; BUNDLE="dev.ligh.Fixture"; GOAL="LighDone"
      SHAPE="form"; PARAMS=(--param "pilot")
      BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"; BUILD_ARG="$NAME"
      APP_PATH="$ROOT/fixtures/$NAME/build/$NAME.app"
      WORKSPACE="$ROOT/fixtures/$NAME" ;;
    lighonboard)
      NAME="LighOnboard"; BUNDLE="dev.ligh.Onboard"; GOAL="HomeReady"
      SHAPE="multi_step_wizard"; PARAMS=(--param "pilot")
      BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"; BUILD_ARG="$NAME"
      APP_PATH="$ROOT/fixtures/$NAME/build/$NAME.app"
      WORKSPACE="$ROOT/fixtures/$NAME" ;;
    lighmodal)
      NAME="LighModal"; BUNDLE="dev.ligh.Modal"; GOAL="ModalConfirmed"
      SHAPE="sheet_overlay"; PARAMS=()
      BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"; BUILD_ARG="$NAME"
      APP_PATH="$ROOT/fixtures/$NAME/build/$NAME.app"
      WORKSPACE="$ROOT/fixtures/$NAME" ;;
    lighfeed)
      NAME="LighFeed"; BUNDLE="dev.ligh.Feed"; GOAL="PostDetail"
      SHAPE="list_drill_down"; PARAMS=()
      BUILD_SCRIPT="$ROOT/scripts/build-workflow-app.sh"; BUILD_ARG="$NAME"
      APP_PATH="$ROOT/fixtures/$NAME/build/$NAME.app"
      WORKSPACE="$ROOT/fixtures/$NAME" ;;
    xcuitestdemo)
      NAME="XCUITestDemo"; BUNDLE="com.himali.XCUITestDemo"; GOAL="homeTitle"
      SHAPE="login_credentials"; PARAMS=(--param "alice" --param "secure:secret")
      BUILD_SCRIPT="$ROOT/scripts/build-xcuitestdemo.sh"; BUILD_ARG=""
      APP_PATH="$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
      WORKSPACE="$ROOT/fixtures/third-party/XCUITestDemo" ;;
    kix)
      NAME="Kix"; BUNDLE="mybyKosta.Kix"; GOAL="tab_home"
      SHAPE="catalog_auth_tabs"; PARAMS=(--param "test@kixapp.com" --param "secure:password")
      BUILD_SCRIPT="$ROOT/scripts/build-kix.sh"; BUILD_ARG=""
      APP_PATH="$ROOT/fixtures/third-party/Kix/build/Kix.app"
      WORKSPACE="$ROOT/fixtures/third-party/Kix" ;;
    *) fail "unknown app id: $1" ;;
  esac
}

echo "══ Autopilot generality gate ══"
echo "  one generic policy · zero per-app steps · zero LLM tokens"
echo "  apps: $APPS"
echo ""

for app_id in $APPS; do
  app_config "$app_id"
  echo "  ▶ build $NAME"
  if [[ -n "$BUILD_ARG" ]]; then
    "$BUILD_SCRIPT" "$BUILD_ARG" >"/tmp/pilot-build-$app_id.log" 2>&1 || fail "build $NAME failed"
  else
    "$BUILD_SCRIPT" >"/tmp/pilot-build-$app_id.log" 2>&1 || fail "build $NAME failed"
  fi
  [[ -d "$APP_PATH" ]] || fail "missing $APP_PATH"
done

for app_id in $APPS; do
  app_config "$app_id"
  echo ""
  echo "══ $NAME ($SHAPE) → goal=$GOAL ══"

  rm -rf "$WORKSPACE/.ligh"
  sim_clean_reboot "$LIGH"
  "$ROOT/scripts/agent-first-loop.sh" >"/tmp/pilot-first-$app_id.log" 2>&1 \
    || "$LIGH" --json ready --settle-ms 3500 --recover-homes 6 >/dev/null

  T0=$(python3 -c 'import time; print(int(time.time()*1000))')
  RUN_JSON="$TRACE_DIR/$app_id.json"
  RUN_OK=0
  if "$LIGH" --json cap autopilot \
      --app "$APP_PATH" --bundle-id "$BUNDLE" \
      --goal-id "$GOAL" ${PARAMS[@]+"${PARAMS[@]}"} \
      --workspace "$WORKSPACE" \
      --max-steps "$MAX_STEPS" --settle-ms 1500 --timeout-ms 8000 \
      >"$RUN_JSON" 2>"$TRACE_DIR/$app_id.stderr.log"; then
    RUN_OK=1
  fi
  T1=$(python3 -c 'import time; print(int(time.time()*1000))')

  python3 - "$RUN_JSON" "$app_id" "$NAME" "$SHAPE" "$GOAL" "$RUN_OK" "$((T1 - T0))" \
    "${#PARAMS[@]:-0}" "$TRACE_DIR" <<'PY'
import json, os, sys

path, app_id, name, shape, goal, run_ok, ms, n_param_args, trace_dir = sys.argv[1:10]
doc = json.load(open(path))
detail = doc.get("detail") or {}
row = {
    "app_id": app_id,
    "app": name,
    "flow_shape": shape,
    "goal_id": goal,
    "reached": bool(detail.get("reached")),
    "ok": int(run_ok) == 1,
    "steps": detail.get("steps"),
    "wall_ms": int(ms),
    "pilot_ms": detail.get("elapsed_ms"),
    "llm_tokens": detail.get("llm_tokens", 0),
    # Inputs given to the host: acceptance target + typed data. No path.
    "spec_inputs": {"goal_id": goal, "params": int(n_param_args) // 2},
    "stop_code": detail.get("stop_code"),
    "diagnosis": detail.get("diagnosis"),
    "trace": detail.get("trace"),
}
open(os.path.join(trace_dir, f"{app_id}-row.json"), "w").write(json.dumps(row, indent=2) + "\n")
mark = "✓" if row["reached"] else "✗"
extra = "" if row["reached"] else f" — {(row.get('diagnosis') or {}).get('code')}"
print(f"  {mark} {name}: reached={row['reached']} steps={row['steps']} {row['wall_ms']}ms{extra}")
PY
done

python3 - "$OUT" "$TRACE_DIR" "$APPS" <<'PY'
import json, os, sys

out, trace_dir, apps = sys.argv[1:4]
app_ids = apps.split()
rows = []
for a in app_ids:
    p = os.path.join(trace_dir, f"{a}-row.json")
    rows.append(json.load(open(p)) if os.path.exists(p) else {"app_id": a, "reached": False})

reached = [r for r in rows if r.get("reached")]
shapes = sorted({r.get("flow_shape") for r in reached if r.get("flow_shape")})
zero_llm = all(r.get("llm_tokens", 0) == 0 for r in rows)
# Generality: every app reached its goal from the same policy, and the flows
# exercised are structurally different (not five variants of one screen).
claim_pass = len(reached) == len(rows) and len(shapes) >= 3 and zero_llm

report = {
    "gate": "autopilot_generality",
    "claim": "One generic Feel-IR policy reaches the acceptance target on every app, "
             "with no per-app steps and zero LLM tokens",
    "apps_total": len(rows),
    "apps_reached": len(reached),
    "flow_shapes_covered": shapes,
    "per_app": rows,
    "verdict": {
        "all_reached": len(reached) == len(rows),
        "distinct_flow_shapes": len(shapes),
        "zero_llm": zero_llm,
        "median_wall_ms": sorted(r.get("wall_ms", 0) for r in rows)[len(rows) // 2] if rows else None,
    },
    "claim_pass": claim_pass,
}
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print("")
print(json.dumps({k: v for k, v in report.items() if k != "per_app"}, indent=2))
raise SystemExit(0 if claim_pass else 1)
PY

ok "autopilot generality → $OUT"
echo "══ Autopilot generality gate PASS ══"
