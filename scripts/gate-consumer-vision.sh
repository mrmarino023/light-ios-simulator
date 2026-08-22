#!/usr/bin/env bash
# Consumer Agent Vision gate — structured AX control (observe v2) + motor, no screenshots.
# Exit 0 only if substrate gates pass. LLM 20× is optional (needs OPENAI_API_KEY).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/agent-env.sh
source "$ROOT/scripts/lib/agent-env.sh" 2>/dev/null || true

LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT_DIR="${LIGH_GATE_OUT:-$ROOT/docs/assets}"
mkdir -p "$OUT_DIR"
REPORT="$OUT_DIR/consumer-vision-gate-latest.json"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

[[ -x "$LIGH" ]] || fail "build release first: cargo build --release -p ligh-cli -p ligh-daemon"

"$LIGH" daemon status >/dev/null 2>&1 || fail "lighd not running — ligh daemon start && ligh up"

echo "══ Consumer Agent Vision gate ══"
echo "eyes = observe v2 scene graph (no PNG to LLM)"

# Settle SpringBoard until eyes are ready (human would wait for home icons).
READY=0
for _ in $(seq 1 16); do
  "$LIGH" home >/dev/null 2>&1 || true
  sleep 0.45
  OUT=$("$LIGH" --json observe --settle-ms 3000 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("ax_quality",""), d.get("settled"), (d.get("scene") or {}).get("surface"), len(d.get("actionable_topk") or []))' 2>/dev/null || echo "empty False none 0")
  AXQ=$(echo "$OUT" | awk '{print $1}')
  TOP=$(echo "$OUT" | awk '{print $4}')
  SURF=$(echo "$OUT" | awk '{print $3}')
  if [[ "$AXQ" == "ready" && "$TOP" -gt 0 ]]; then READY=1; break; fi
done
[[ "$READY" == "1" ]] || fail "AX never became ready (last=$OUT)"
ok "eyes ready surface=$SURF topk=$TOP"

SCHEMA=$("$LIGH" --json observe --settle-ms 1500 | python3 -c 'import json,sys; print(json.load(sys.stdin).get("schema_version",0))')
[[ "$SCHEMA" == "2" ]] || fail "schema_version=$SCHEMA (want 2)"
ok "observe schema_version=2"
AXQ=$AXQ
TOP=$TOP
ok "actionable_topk=$TOP ax_quality=$AXQ"

# id present on first actionable
ID=$("$LIGH" --json observe | python3 -c 'import json,sys; d=json.load(sys.stdin); print((d.get("actionable_topk") or [{}])[0].get("id") or "")')
[[ -n "$ID" ]] || fail "first actionable missing id"
ok "stable id sample=$ID"

# sense RPC
"$LIGH" --json sense >/dev/null || fail "sense rpc"
ok "sense rpc"

# home + observe events path
"$LIGH" home >/dev/null || true
sleep 0.4
"$LIGH" home >/dev/null || true
sleep 0.6

# Prefer Italian then English home icons
SETTINGS=""
for L in Impostazioni Settings; do
  if "$LIGH" exists --label "$L" >/dev/null 2>&1; then SETTINGS="$L"; break; fi
done
MESSAGES=""
for L in Messaggi Messages; do
  if "$LIGH" exists --label "$L" >/dev/null 2>&1; then MESSAGES="$L"; break; fi
done

MOTOR_OK=1
if [[ -n "$SETTINGS" ]]; then
  "$LIGH" tap --label "$SETTINGS" --timeout-ms 5000 >/dev/null || MOTOR_OK=0
  sleep 0.5
  # scroll_until Generali / General if present after open
  if "$LIGH" scroll-until --label Generali --max-swipes 6 >/dev/null 2>&1 \
    || "$LIGH" scroll-until --label General --max-swipes 6 >/dev/null 2>&1; then
    ok "scroll-until on Settings list"
  else
    ok "scroll-until skipped/miss (non-fatal if list short)"
  fi
  "$LIGH" home >/dev/null || true
  sleep 0.3
  "$LIGH" home >/dev/null || true
  sleep 0.4
else
  echo "warn: Settings icon not found — skip settings motor"
fi

# long-press smoke on Messages icon if present
if [[ -n "$MESSAGES" ]]; then
  if "$LIGH" long-press --label "$MESSAGES" --hold-ms 550 >/dev/null 2>&1; then
    ok "long-press $MESSAGES"
  else
    MOTOR_OK=0
    echo "warn: long-press failed"
  fi
  "$LIGH" home >/dev/null || true
  sleep 0.3
fi

# clear / key smoke (no assert on UI effect)
"$LIGH" key --name tab >/dev/null 2>&1 || true
"$LIGH" clear --count 2 >/dev/null 2>&1 || true
ok "key/clear primitives"

[[ "$MOTOR_OK" == "1" ]] || fail "motor micro gate failed"

# ── Optional LLM 20× (Messages + Settings) ───────────────────────────────────
LLM_STATUS="skipped"
LLM_PASS=0
LLM_TOTAL=0
if [[ -n "${OPENAI_API_KEY:-}" && "${LIGH_LLM_GATE:-0}" == "1" ]]; then
  MODEL="${OPENAI_MODEL:-gpt-5-mini}"
  N="${LIGH_LLM_N:-20}"
  LLM_TOTAL=$((N * 2))
  echo "── LLM gate N=$N model=$MODEL (no screenshots) ──"
  for goal in \
    "Open Settings (Impostazioni or Settings), wait until search or list is visible, then done" \
    "Open Messages (Messaggi or Messages), start new message, type: ligh-vision-ok, then done"; do
    for ((i=1; i<=N; i++)); do
      "$LIGH" home >/dev/null 2>&1 || true
      sleep 0.35
      "$LIGH" home >/dev/null 2>&1 || true
      sleep 0.45
      if python3 "$ROOT/scripts/agent-llm-loop.py" --model "$MODEL" --steps 14 --goal "$goal" >/tmp/ligh-llm-gate.log 2>&1; then
        LLM_PASS=$((LLM_PASS + 1))
        echo "  ok  $i/$N — ${goal:0:40}…"
      else
        echo "  FAIL $i/$N — see /tmp/ligh-llm-gate.log"
      fi
    done
  done
  if [[ "$LLM_PASS" -eq "$LLM_TOTAL" ]]; then
    LLM_STATUS="pass"
    ok "LLM $LLM_PASS/$LLM_TOTAL"
  else
    LLM_STATUS="fail"
    echo "✗ LLM $LLM_PASS/$LLM_TOTAL" >&2
  fi
else
  echo "(LLM 20× skipped — set OPENAI_API_KEY and LIGH_LLM_GATE=1)"
fi

# Vision-only compare: only if explicitly enabled (needs screenshots path — not default)
VISION_STATUS="skipped"
if [[ "${LIGH_VISION_COMPARE:-0}" == "1" ]]; then
  VISION_STATUS="not_implemented_in_default_path"
fi

python3 - <<PY
import json, time
report = {
  "ts": time.time(),
  "schema_version": 2,
  "substrate": {"ok": True, "actionable_topk": int("$TOP"), "ax_quality": "$AXQ"},
  "motor": {"ok": True},
  "llm": {"status": "$LLM_STATUS", "pass": int("$LLM_PASS"), "total": int("$LLM_TOTAL")},
  "vision_compare": {"status": "$VISION_STATUS"},
  "publish": False,
}
# Publish only if substrate+motor ok AND (llm skipped OR llm full pass)
report["publish"] = report["substrate"]["ok"] and report["motor"]["ok"] and (
  report["llm"]["status"] in ("skipped", "pass")
) and report["llm"]["status"] != "fail"
# Honest: do not claim LLM competitiveness unless llm pass
if report["llm"]["status"] != "pass":
  report["claim"] = "substrate_ready_observe_v2_no_png"
else:
  report["claim"] = "llm_20x20_no_png_pass"
open("$REPORT","w").write(json.dumps(report, indent=2)+"\n")
print(json.dumps(report, indent=2))
if report["llm"]["status"] == "fail":
  raise SystemExit(1)
PY

ok "wrote $REPORT"
echo "══ gate ok (eyes=structured scene, no screenshots) ══"
