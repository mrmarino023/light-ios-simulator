#!/usr/bin/env bash
# Append developer trial feedback to docs/assets/developer-feedback.jsonl
#
# Usage:
#   ./scripts/developer-feedback.sh
#   DEVELOPER_ID=alice PREFERENCE=ligh ONE_SENTENCE="…" ./scripts/developer-feedback.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/docs/assets/developer-feedback.jsonl"

read -r -p "Developer id (handle): " DEV_ID < /dev/tty
DEV_ID="${DEVELOPER_ID:-$DEV_ID}"
read -r -p "Preference [ligh/maestro/neither/tie]: " PREF < /dev/tty
PREF="${PREFERENCE:-$PREF}"
read -r -p "One sentence (why/why not): " SENT < /dev/tty
SENT="${ONE_SENTENCE:-$SENT}"
read -r -p "Agent understood LIGH faults? [y/n]: " AU < /dev/tty
AU="${AGENT_UNDERSTOOD:-$AU}"

python3 - "$OUT" "$DEV_ID" "$PREF" "$SENT" "$AU" <<'PY'
import json, sys, datetime
out, did, pref, sent, au = sys.argv[1:6]
row = {
  "developer_id": did or "anonymous",
  "date": datetime.date.today().isoformat(),
  "preference": pref,
  "one_sentence": sent,
  "agent_understood_faults": au.lower().startswith("y") if au else None,
}
open(out, "a").write(json.dumps(row)+"\n")
print("✓ appended →", out)
print(json.dumps(row, indent=2))
PY
