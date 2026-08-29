#!/usr/bin/env bash
# Real agent environment loop — daemon → ready → certify from .ligh/
#
# Usage:
#   LIGH_WORKSPACE=/path/to/app ./scripts/ligh-agent-loop.sh
#   LIGH_WORKSPACE=/path/to/app ./scripts/ligh-agent-loop.sh --print-mcp
#   LIGH_WORKSPACE=/path/to/app LIGH_TEST_MODE=job ./scripts/ligh-agent-loop.sh
#
# Success = .ligh/last-certify.json with ok:true. Never claim from screenshots.
# On app_crashed / app_not_running: open crash_report_path — do NOT TRAIL.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
WORKSPACE="${LIGH_WORKSPACE:-}"
PRINT_MCP=0
for arg in "$@"; do
  case "$arg" in
    --print-mcp) PRINT_MCP=1 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
  esac
done

fail() { echo "✗ $*" >&2; exit 1; }
[[ -n "$WORKSPACE" ]] || fail "set LIGH_WORKSPACE to the app/project root that owns .ligh/"
[[ -d "$WORKSPACE/.ligh" ]] || fail "missing $WORKSPACE/.ligh — run ./scripts/ligh-paradise.sh first"
[[ -x "$LIGH" ]] || fail "missing $LIGH — cargo build --release -p ligh-cli -p ligh-daemon"

export LIGH_WORKSPACE="$WORKSPACE"
export PYTHONPATH="$ROOT/scripts"
export LIGH_BIN="$LIGH"

echo "══ ligh agent loop ══"
echo "  workspace: $WORKSPACE"

# MCP snippet for Cursor (also written under .ligh/mcp.json)
LIGH_WORKSPACE="$WORKSPACE" "$ROOT/scripts/print-cursor-mcp.sh" \
  >/tmp/ligh-agent-loop-mcp.txt
python3 - "$WORKSPACE" /tmp/ligh-agent-loop-mcp.txt <<'PY'
import json, re, sys
ws, path = sys.argv[1:3]
blob = open(path).read()
m = re.search(r"\{[\s\S]*\"mcpServers\"[\s\S]*\}", blob)
if not m:
    raise SystemExit("mcp json missing from print-cursor-mcp.sh")
cfg = json.loads(m.group(0))
out = f"{ws}/.ligh/mcp.json"
open(out, "w").write(json.dumps(cfg, indent=2) + "\n")
print(f"  wrote {out}")
PY

if [[ "$PRINT_MCP" -eq 1 ]]; then
  cat /tmp/ligh-agent-loop-mcp.txt
  exit 0
fi

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$LIGH" up >/dev/null 2>&1 || true

echo "── ligh_test (mode=${LIGH_TEST_MODE:-goal}) ──"
set +e
LIGH_TEST_OUT="${LIGH_TEST_OUT:-$WORKSPACE/.ligh/last-test.json}" \
  "$ROOT/scripts/ligh-test.sh"
rc=$?
set -e

CERT="$WORKSPACE/.ligh/last-certify.json"
if [[ -f "$CERT" ]]; then
  python3 - "$CERT" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))
print(json.dumps({
  "certify": sys.argv[1],
  "ok": d.get("ok"),
  "fault": d.get("fault"),
  "trail_allowed": d.get("trail_allowed"),
  "repair_allowed": d.get("repair_allowed"),
  "process_health": {
    k: (d.get("process_health") or {}).get(k)
    for k in ("running","crashed_recently","crash_report_path","bundle_id")
  },
}, indent=2))
PY
else
  echo "⚠ no last-certify.json written" >&2
fi

exit "$rc"
