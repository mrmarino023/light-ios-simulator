#!/usr/bin/env bash
# Blind bakeoff protocol — same agent prompt, measure turns (Mac required to execute).
#
# Usage (Mac):
#   export LIGH_WORKSPACE=/path/to/repo
#   export OPENAI_API_KEY=…
#   LIGH_BAKEOFF_ARM=ligh ./scripts/gate-blind-bakeoff.sh
#   LIGH_BAKEOFF_ARM=raw ./scripts/gate-blind-bakeoff.sh   # ligh_observe + ligh_tap only
#
# This script writes protocol JSON even on Linux (dry-run = claim_pass false).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${LIGH_GATE_OUT:-$ROOT/docs/assets}/blind-bakeoff-latest.json"
ARM="${LIGH_BAKEOFF_ARM:-ligh}"
GOAL="${LIGH_BAKEOFF_GOAL:-Complete XCUITestDemo login (alice/secret) and reach home screen.}"
export ROOT OUT ARM GOAL

fail() { echo "✗ $*" >&2; exit 1; }

python3 - <<'PY'
import json, os, platform, time

root = os.environ.get("ROOT", ".")
arm = os.environ.get("LIGH_BAKEOFF_ARM", "ligh")
goal = os.environ.get("GOAL") or os.environ.get("LIGH_BAKEOFF_GOAL", "")
is_mac = platform.system() == "Darwin"
has_key = bool(os.environ.get("OPENAI_API_KEY", "").strip())
can_run = is_mac and has_key

report = {
    "gate": "blind_bakeoff",
    "ts": time.time(),
    "claim": "same prompt — measure llm_turns + success; ligh vs raw MCP",
    "arm": arm,
    "goal": goal,
    "protocol": [
        "no host_policy",
        "no goal-string cap matching",
        "no pre-built maestro yaml",
        "score: success, llm_turns, tokens, wrong_actions",
    ],
    "can_run": can_run,
    "is_mac": is_mac,
    "has_openai_key": has_key,
    "claim_pass": False,
    "note": "Execute on Mac with gate-autonomous-qa.sh for ligh arm; raw arm TBD",
    "arms": {
        "ligh": "perceive + attempt + ux (autonomous-login-agent-qa.py)",
        "raw": "ligh_observe + ligh_tap loop (not implemented — honest gap)",
        "computer_use": "Cursor computer use on Simulator — manual A/B",
    },
}
out = os.environ.get("OUT", "docs/assets/blind-bakeoff-latest.json")
os.makedirs(os.path.dirname(out), exist_ok=True)
open(out, "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
if not can_run:
    print("dry-run: Mac + OPENAI_API_KEY required for live bakeoff", file=__import__("sys").stderr)
PY

if [[ "$(uname -s)" == "Darwin" && -n "${OPENAI_API_KEY:-}" && "$ARM" == "ligh" ]]; then
  echo "── Running ligh arm ──"
  export LIGH_INJECT_BUG="${LIGH_INJECT_BUG:-1}"
  if ./scripts/gate-autonomous-qa.sh; then
    python3 - <<PY
import json
out = "$OUT"
doc = json.load(open(out))
doc["claim_pass"] = True
doc["ligh_arm"] = json.load(open("$ROOT/docs/assets/autonomous-agent-qa-latest.json"))
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
PY
  fi
fi

echo "wrote $OUT"
