#!/usr/bin/env bash
# Run the generated app-job from .ligh/project.json (agent re-test loop).
#
# Usage:
#   ./scripts/ligh-test.sh                    # uses $LIGH_WORKSPACE/.ligh or cwd/.ligh
#   LIGH_WORKSPACE=/path/to/app ./scripts/ligh-test.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
WS="${LIGH_WORKSPACE:-$(pwd)}"
LIGH_DIR="$WS/.ligh"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -f "$LIGH_DIR/project.json" ]] || fail "missing $LIGH_DIR/project.json — run ./scripts/ligh-paradise.sh first"

APP=$(python3 -c "import json; print(json.load(open('$LIGH_DIR/project.json')).get('app_path') or '')")
BID=$(python3 -c "import json; print(json.load(open('$LIGH_DIR/project.json')).get('bundle_id') or '')")
STEPS=$(python3 -c "import json; print(json.dumps(json.load(open('$LIGH_DIR/app-job.json'))))")

[[ -d "$APP" ]] || fail "app not built: $APP (run ligh-paradise.sh --build)"
[[ -n "$BID" ]] || fail "bundle_id missing in project.json"

[[ -x "$LIGH" ]] || fail "build ligh first: ./scripts/ligh-init.sh"

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-test-first.log 2>&1 || true

OUT="${LIGH_TEST_OUT:-/tmp/ligh-test-latest.json}"
echo "══ ligh test ══"
echo "  app=$APP"
"$LIGH" --json cap app-job "$APP" --bundle-id "$BID" --steps "$STEPS" \
  --settle-ms 3000 --timeout-ms 25000 | tee "$OUT"

python3 -c "import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if d.get('ok') else 1)" "$OUT" \
  && echo "✓ verified" || fail "test failed — fault in $OUT"
