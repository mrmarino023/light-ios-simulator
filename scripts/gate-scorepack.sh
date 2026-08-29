#!/usr/bin/env bash
# Agent Scorepack gate — eval/CI truth machine (not Cursor paradise).
#
# Usage:
#   ./scripts/gate-scorepack.sh           # full TRAIL scorepack (needs OPENAI_API_KEY + Mac)
#   ./scripts/gate-scorepack.sh --dry-run # contract only
#
# Artifact: docs/assets/scorepack-latest.json  (schema ligh.scorepack.result.v1)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PYTHONPATH="$ROOT/scripts"
OUT="${LIGH_SCOREPACK_OUT:-$ROOT/docs/assets/scorepack-latest.json}"
MANIFEST="${LIGH_SCOREPACK_MANIFEST:-$ROOT/scorepack/v1/manifest.json}"

if [[ "${1:-}" == "--dry-run" ]]; then
  python3 "$ROOT/scripts/ligh_scorepack.py" --manifest "$MANIFEST" --out "$OUT" --dry-run
  exit 0
fi

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"
load_openai_env "$ROOT"
[[ -n "${OPENAI_API_KEY:-}" ]] || {
  echo "✗ OPENAI_API_KEY required for full scorepack (or use --dry-run)" >&2
  exit 1
}

echo "══ LIGH agent scorepack ══"
echo "  manifest=$MANIFEST"
echo "  buyer=eval_harness | agent_platform | ci_agent_pr"
python3 "$ROOT/scripts/ligh_scorepack.py" --manifest "$MANIFEST" --out "$OUT"
echo "══ → $OUT"
