#!/usr/bin/env bash
# Print a ready-to-paste Cursor mcp.json snippet with absolute paths for this repo.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
MCP="$ROOT/scripts/ligh_mcp.py"
python3 - "$ROOT" "$LIGH" "$MCP" <<'PY'
import json, sys
root, ligh, mcp = sys.argv[1:4]
cfg = {
  "mcpServers": {
    "ligh": {
      "command": "python3",
      "args": [mcp],
      "env": {"LIGH_BIN": ligh}
    }
  }
}
print("# Add to Cursor → Settings → MCP (or ~/.cursor/mcp.json)")
print("# Repo:", root)
print(json.dumps(cfg, indent=2))
print()
print("# Then: ./scripts/agent-first-loop.sh")
print("# App under test: ./scripts/app-under-test.sh --bundle-id com.apple.Maps --assert-label Mappa")
PY
