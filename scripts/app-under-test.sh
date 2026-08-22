#!/usr/bin/env bash
# One-command app-under-test: launch Debug .app (or bundle id) → settle → optional agent goal.
#
# Usage:
#   ./scripts/app-under-test.sh /path/to/MyApp.app
#   ./scripts/app-under-test.sh --bundle-id com.apple.Maps
#   ./scripts/app-under-test.sh --bundle-id com.apple.Maps --assert-label Mappa
#   ./scripts/app-under-test.sh --bundle-id com.apple.Maps --goal "Ensure Maps chrome visible, done"
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"

APP=""
BID=""
GOAL=""
ASSERT_LABEL="${LIGH_AUT_ASSERT_LABEL:-}"
STEPS="${LIGH_AUT_STEPS:-14}"
MODEL="${OPENAI_MODEL:-gpt-5-mini}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-id) BID="${2:-}"; shift 2 ;;
    --goal) GOAL="${2:-}"; shift 2 ;;
    --assert-label) ASSERT_LABEL="${2:-}"; shift 2 ;;
    -*) fail "unknown arg $1" ;;
    *) APP="$1"; shift ;;
  esac
done

[[ -n "$APP" || -n "$BID" ]] || fail "pass .app path and/or --bundle-id"

echo "══ app-under-test ══"
"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-aut-first-loop.log 2>&1 \
  || fail "first-loop failed — see /tmp/ligh-aut-first-loop.log"

if [[ -n "$APP" ]]; then
  echo "▶ run $APP"
  if [[ -n "$BID" ]]; then
    "$LIGH" --json run "$APP" --bundle-id "$BID" | tee /tmp/ligh-aut-run.json
  else
    "$LIGH" --json run "$APP" | tee /tmp/ligh-aut-run.json
  fi
else
  echo "▶ launch $BID"
  "$ROOT/scripts/launch-app-under-test.sh" --bundle-id "$BID" | tee /tmp/ligh-aut-launch.log
fi

sleep 0.6
OBS=$("$LIGH" --json observe --settle-ms 2500)
python3 - "$OBS" "$ASSERT_LABEL" <<'PY'
import json, sys
d = json.loads(sys.argv[1])
want = sys.argv[2] if len(sys.argv) > 2 else ""
aq = d.get("ax_quality")
settled = d.get("settled")
surface = (d.get("scene") or {}).get("surface")
labels = [x.get("label") or "" for x in (d.get("actionable_topk") or [])]
print(json.dumps({"ax_quality": aq, "settled": settled, "surface": surface, "labels": labels[:12]}, indent=2))
if aq in ("empty", "transition", "error") or not settled:
    raise SystemExit("eyes_unusable after launch — fail closed")
if surface == "transition":
    raise SystemExit("surface=transition — fail closed")
if want and want not in labels and not any(want in l for l in labels):
    raise SystemExit(f"assert-label missing: {want!r}")
print("✓ launch settled" + (f" · assert {want!r}" if want else ""))
PY

if [[ -n "$GOAL" ]]; then
  [[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required for --goal"
  echo "▶ agent goal (policy=llm): $GOAL"
  python3 "$ROOT/scripts/agent-llm-loop.py" --policy llm --model "$MODEL" --steps "$STEPS" --goal "$GOAL"
fi

echo "✓ app-under-test ok"
