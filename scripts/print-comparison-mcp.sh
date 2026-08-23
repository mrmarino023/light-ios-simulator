#!/usr/bin/env bash
# Print Cursor mcp.json with LIGH + Maestro for developer A/B.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
LIGH_MCP="$ROOT/scripts/ligh_mcp.py"
MAESTRO_MCP="$ROOT/scripts/maestro_mcp.py"
python3 - "$ROOT" "$LIGH" "$LIGH_MCP" "$MAESTRO_MCP" <<'PY'
import json, sys, os
root, ligh, ligh_mcp, maestro_mcp = sys.argv[1:5]
maestro = os.path.expanduser("~/.maestro/bin/maestro")
cfg = {
  "mcpServers": {
    "ligh": {
      "command": "python3",
      "args": [ligh_mcp],
      "env": {"LIGH_BIN": ligh},
    },
    "maestro": {
      "command": "python3",
      "args": [maestro_mcp],
      "env": {"MAESTRO_BIN": maestro},
    },
  }
}
print("# Cursor → Settings → MCP (LIGH vs Maestro A/B)")
print("# Repo:", root)
print(json.dumps(cfg, indent=2))
if not os.path.isfile(maestro):
  print("\n# ⚠ Maestro not at", maestro, "— install: https://maestro.mobile.dev")
PY
