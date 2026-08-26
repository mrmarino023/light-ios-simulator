#!/usr/bin/env bash
# Agent paradise — one command from zero → MCP + audit + smoke + agent prompt.
#
# Usage:
#   ./scripts/ligh-paradise.sh                                    # demo fixture
#   ./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj           # detect + audit
#   ./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj --build   # + xcodebuild
#   ./scripts/ligh-paradise.sh /path/to/MyApp.app
#   ./scripts/ligh-paradise.sh fixtures/frozen/tasks/.../task.json
#
# Writes: <workspace>/.ligh/project.json · app-job.json · AGENT_PROMPT.md
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
TARGET="${1:-$ROOT/fixtures/LighFixture/build/LighFixture.app}"
BUILD=0
[[ "${2:-}" == "--build" ]] && BUILD=1
[[ "$TARGET" == *"--build"* ]] && BUILD=1

fail() { echo "✗ $*" >&2; exit 1; }

echo "══ LIGH agent paradise ══"
echo "  target: $TARGET"
echo "  docs:   $ROOT/docs/AGENT_PARADISE.md"
echo

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS + Xcode required"
xcode-select -p &>/dev/null || fail "install Xcode CLT"

if [[ ! -x "$LIGH" ]] || [[ ! -x "${LIGH%d/ligh}/lighd" ]]; then
  echo "▶ build release…"
  (cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release --locked -p ligh-cli -p ligh-daemon) \
    || fail "cargo build failed"
fi

echo "▶ detect project + accessibility audit"
DETECT_JSON=/tmp/ligh-paradise-detect.json
BUILD_FLAG=""
[[ "$BUILD" -eq 1 ]] && BUILD_FLAG="--build"
if ! PYTHONPATH="$ROOT/scripts" python3 "$ROOT/scripts/ligh_project.py" "$TARGET" $BUILD_FLAG --json >"$DETECT_JSON"; then
  echo "  ⚠ audit grade below agent-ready threshold — continuing anyway"
  PYTHONPATH="$ROOT/scripts" python3 "$ROOT/scripts/ligh_project.py" "$TARGET" $BUILD_FLAG --json >"$DETECT_JSON" || true
fi

LIGH_DIR=$(python3 -c "import json; print(json.load(open('$DETECT_JSON')).get('ligh_dir',''))")
WORKSPACE=$(python3 -c "import json; print(json.load(open('$DETECT_JSON')).get('workspace',''))")
APP=$(python3 -c "import json; d=json.load(open('$DETECT_JSON')); print(d.get('app_path') or '')")
BID=$(python3 -c "import json; print(json.load(open('$DETECT_JSON')).get('bundle_id') or '')")
GRADE=$(python3 -c "import json; print(json.load(open('$DETECT_JSON')).get('audit',{}).get('readiness_grade','?'))")

echo "  → $LIGH_DIR (grade $GRADE)"

# Demo fixture: ensure .app exists
if [[ -z "$APP" || ! -d "$APP" ]]; then
  if [[ "$TARGET" == *LighFixture* ]]; then
    echo "▶ build LighFixture demo"
    "$ROOT/scripts/build-fixture.sh" >/tmp/ligh-paradise-build.log 2>&1
    APP="$ROOT/fixtures/LighFixture/build/LighFixture.app"
    BID="dev.ligh.Fixture"
    PYTHONPATH="$ROOT/scripts" python3 "$ROOT/scripts/ligh_project.py" "$APP" --json >"$DETECT_JSON"
    LIGH_DIR=$(python3 -c "import json; print(json.load(open('$DETECT_JSON')).get('ligh_dir',''))")
  else
    echo "  ⚠ no .app yet — re-run with --build for xcodeproj"
  fi
fi

echo "▶ daemon + sim warm"
"$LIGH" daemon stop >/dev/null 2>&1 || true
pkill -x lighd 2>/dev/null || true
sleep 0.5
nohup "${LIGH%d/ligh}/lighd" >>/tmp/lighd-paradise.log 2>&1 &
sleep 1.2
"$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}" >/dev/null 2>&1 || "$LIGH" up --device "${LIGH_DEVICE:-iphone-15-pro}"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-paradise-first.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 2000 --recover-homes 2 >/dev/null 2>&1 || true

SMOKE_OK=0
if [[ -n "$APP" && -d "$APP" && -n "$BID" ]]; then
  echo "▶ smoke app-job"
  STEPS=$(python3 -c "import json; print(json.dumps(json.load(open('$LIGH_DIR/app-job.json'))))")
  JOB_OUT=/tmp/ligh-paradise-job.json
  if "$LIGH" --json cap app-job "$APP" --bundle-id "$BID" --steps "$STEPS" \
    --settle-ms 3000 --timeout-ms 20000 >"$JOB_OUT" 2>/tmp/ligh-paradise-job.err; then
    SMOKE_OK=1
  elif python3 -c "import json; d=json.load(open('$JOB_OUT')); exit(0 if d.get('ok') else 1)" 2>/dev/null; then
    SMOKE_OK=1
  else
    echo "  ⚠ smoke failed — edit $LIGH_DIR/app-job.json and run: ./scripts/ligh-test.sh"
    python3 -c "import json; d=json.load(open('$JOB_OUT')); print('  fault:', d.get('fault'))" 2>/dev/null || true
  fi
fi

PARADISE_OUT="${LIGH_PARADISE_OUT:-$ROOT/docs/assets/agent-paradise-latest.json}"
python3 - "$PARADISE_OUT" "$DETECT_JSON" "$SMOKE_OK" <<'PY'
import json, sys, time
out, detect_path, smoke_ok = sys.argv[1:4]
doc = json.load(open(detect_path))
bundle = {
  "gate": "agent_paradise",
  "ok": bool(smoke_ok == "1") or doc.get("kind") == "xcodeproj",
  "smoke_ok": smoke_ok == "1",
  "ligh_dir": doc.get("ligh_dir"),
  "workspace": doc.get("workspace"),
  "app_path": doc.get("app_path"),
  "bundle_id": doc.get("bundle_id"),
  "readiness_grade": (doc.get("audit") or {}).get("readiness_grade"),
  "readiness_score": (doc.get("audit") or {}).get("readiness_score"),
  "identity_count": (doc.get("audit") or {}).get("identity_count"),
  "agent_ready": (doc.get("audit") or {}).get("agent_ready"),
}
json.dump(bundle, open(out, "w"), indent=2)
open(out, "a").write("\n")
print(json.dumps(bundle, indent=2))
PY

echo
echo "── Cursor MCP (set LIGH_WORKSPACE=$WORKSPACE) ──"
LIGH_WORKSPACE="$WORKSPACE" "$ROOT/scripts/print-cursor-mcp.sh"
echo
echo "── Agent prompt ──"
echo "  $LIGH_DIR/AGENT_PROMPT.md"
echo "  $ROOT/docs/AGENT_PARADISE.md"
echo
echo "── Test your app ──"
echo "  LIGH_WORKSPACE=$WORKSPACE ./scripts/ligh-test.sh"
echo "  ./scripts/gate-agent-environment.sh   # full MCP gate"
echo
if [[ "$SMOKE_OK" -eq 1 ]]; then
  echo "✓ paradise ready — smoke passed"
else
  echo "~ paradise scaffold ready — fix app-job steps then ligh-test.sh"
fi
echo "  artifact → $PARADISE_OUT"
