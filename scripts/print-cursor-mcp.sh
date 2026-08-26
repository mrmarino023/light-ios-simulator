#!/usr/bin/env bash
# Print a ready-to-paste Cursor mcp.json snippet with absolute paths for this repo.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
MCP="$ROOT/scripts/ligh_mcp.py"
WORKSPACE="${LIGH_WORKSPACE:-$ROOT}"
python3 - "$ROOT" "$LIGH" "$MCP" "$WORKSPACE" <<'PY'
import json, sys
root, ligh, mcp, workspace = sys.argv[1:5]
env = {"LIGH_BIN": ligh, "LIGH_WORKSPACE": workspace}
cfg = {
  "mcpServers": {
    "ligh": {
      "command": "python3",
      "args": [mcp],
      "env": env
    }
  }
}
print("# Add to Cursor → Settings → MCP (or ~/.cursor/mcp.json)")
print("# Repo:", root)
print("# LIGH_WORKSPACE:", workspace)
print(json.dumps(cfg, indent=2))
print()
print("# Agent paradise: ./scripts/ligh-paradise.sh /path/to/YourApp.xcodeproj --build")
print("# Re-test:         LIGH_WORKSPACE=/path/to/app ./scripts/ligh-test.sh")
PY
