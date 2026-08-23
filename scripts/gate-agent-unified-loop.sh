#!/usr/bin/env bash
# Agent unified loop gate — scripted (always) + LLM (if OPENAI_API_KEY).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_AGENT_LOOP_GATE_OUT:-$ROOT/docs/assets/agent-unified-loop-gate-latest.json}"

fail() { echo "✗ $*" >&2; exit 1; }

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"

[[ -x "$LIGH" ]] || (cd "$ROOT" && unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon --locked) || fail "build failed"
# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"
sim_clean_reboot "$LIGH" || fail "sim prep"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/agent-loop-first.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 4000 --recover_homes 6 >/dev/null \
  || fail "ligh_ready"

echo "  ▶ agent-unified-loop (scripted)"
PASS=0
if python3 "$ROOT/scripts/agent-unified-loop.py"; then PASS=1; fi

LLM_PASS="skipped"
if [[ -n "${OPENAI_API_KEY:-}" ]]; then
  echo "  ▶ agent-unified-loop (LLM)"
  export LIGH_AGENT_GOAL="Run app job on LighFixture: type a name and verify LighDone."
  if python3 "$ROOT/scripts/agent-unified-loop.py"; then LLM_PASS="pass"; else LLM_PASS="fail"; fi
fi

python3 - "$OUT" "$PASS" "$LLM_PASS" <<'PY'
import json, sys, time
out, scripted, llm = sys.argv[1:4]
doc = {
  "gate": "agent_unified_loop",
  "scripted_ok": scripted == "1",
  "llm_ok": llm,
  "ts": time.time(),
}
open(out, "w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps(doc, indent=2))
PY

[[ "$PASS" -eq 1 ]] || exit 1
echo "══ → $OUT"
