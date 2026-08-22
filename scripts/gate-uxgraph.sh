#!/usr/bin/env bash
# UX Graph unit gate — persistence, baseline, regress, explore planning, source hints.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${LIGH_GATE_OUT:-$ROOT/docs/assets}/uxgraph-latest.json"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

echo "══ UX Graph gate (unit) ══"
command -v cargo >/dev/null || fail "cargo required"
cd "$ROOT"
cargo test -p ligh-core -- --nocapture 2>&1 | tee /tmp/ligh-uxgraph-test.log
ok "ligh-core uxgraph + qa tests"

for f in docs/UX_GRAPH.md crates/ligh-core/src/uxgraph.rs scripts/ligh_mcp.py; do
  [[ -f "$ROOT/$f" ]] || fail "missing $f"
done
ok "UX graph files present"

python3 - <<PY
import json, time
mcp = open("$ROOT/scripts/ligh_mcp.py").read()
for t in ("ligh_ux_status", "ligh_ux_baseline", "ligh_ux_regress", "ligh_ux_explore", "ligh_ux_hint"):
    assert t in mcp, t
report = {
  "gate": "uxgraph_unit",
  "ts": time.time(),
  "claim": "computational UX graph — persist, baseline, regress, explore, source hints",
  "store": ".ligh/uxgraph.json",
  "mcp_tools": ["ligh_ux_status", "ligh_ux_baseline", "ligh_ux_regress", "ligh_ux_explore", "ligh_ux_hint"],
  "claim_pass": True,
}
open("$OUT", "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY
ok "wrote $OUT"
echo "══ UX Graph gate PASS ══"
