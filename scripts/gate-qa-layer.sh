#!/usr/bin/env bash
# QA layer unit gate — screen fingerprint, affordances, attempt verdict logic.
# Runs on any platform (no Simulator required).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${LIGH_GATE_OUT:-$ROOT/docs/assets}/qa-layer-latest.json"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

echo "══ QA layer gate (unit) ══"

if ! command -v cargo >/dev/null 2>&1; then
  fail "cargo not installed"
fi

cd "$ROOT"
cargo test -p ligh-core qa:: -- --nocapture 2>&1 | tee /tmp/ligh-qa-layer-test.log
ok "ligh-core qa tests"

# Static contract checks
for f in docs/QA_LAYER.md scripts/ligh_mcp.py crates/ligh-core/src/qa.rs; do
  [[ -f "$ROOT/$f" ]] || fail "missing $f"
done
ok "QA layer files present"

python3 - <<PY
import json, time, re
root = "$ROOT"
mcp = open(f"{root}/scripts/ligh_mcp.py").read()
for tool in ("ligh_perceive", "ligh_attempt", "ligh_find", "ligh_dismiss"):
    assert tool in mcp, f"MCP missing {tool}"
report = {
  "gate": "qa_layer_unit",
  "ts": time.time(),
  "claim": "screen fingerprint + affordances + attempt verdict (host-owned QA loop)",
  "tests": "ligh-core qa::",
  "mcp_tools": ["ligh_perceive", "ligh_attempt", "ligh_find", "ligh_dismiss"],
  "claim_pass": True,
}
open("$OUT", "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY

ok "wrote $OUT"
echo "══ QA layer unit gate PASS ══"
