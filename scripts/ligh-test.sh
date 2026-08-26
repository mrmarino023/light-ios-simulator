#!/usr/bin/env bash
# Run verify from .ligh/ — goal-first (default) or explicit job steps.
#
# Usage:
#   ./scripts/ligh-test.sh
#   LIGH_TEST_MODE=job ./scripts/ligh-test.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PYTHONPATH="$ROOT/scripts"
MODE="${LIGH_TEST_MODE:-goal}"
python3 - "$MODE" <<'PY'
import json, os, sys
sys.path.insert(0, os.environ.get("PYTHONPATH", "."))
from ligh_agent_api import run_test
mode = sys.argv[1] if len(sys.argv) > 1 else "goal"
out = run_test(mode=mode)
out_path = os.environ.get("LIGH_TEST_OUT", "/tmp/ligh-test-latest.json")
json.dump(out, open(out_path, "w"), indent=2)
print(json.dumps({"ok": out.get("ok"), "mode": mode, "fault": out.get("fault"), "out": out_path}, indent=2))
sys.exit(0 if out.get("ok") else 1)
PY
