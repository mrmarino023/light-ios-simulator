#!/usr/bin/env bash
# Rigorous third-party bakeoff: clean sim → LIGH×N → clean sim → Maestro×N.
#
# Protocol (fair — no cross-contamination between tools):
#   1. clean reboot + agent-first-loop
#   2. LIGH app-job × N
#   3. clean reboot + install app for Maestro
#   4. Maestro × N
#
# Usage: LIGH_APP_N=20 ./scripts/gate-third-party-rigor.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_APP_N:-20}"
OUT="${LIGH_RIGOR_OUT:-$ROOT/docs/assets/third-party-rigor-latest.json}"
APP="${LIGH_APP_PATH:-$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app}"
BUNDLE_ID="${LIGH_APP_BUNDLE_ID:-com.himali.XCUITestDemo}"
MAESTRO_FLOW="${LIGH_MAESTRO_FLOW:-$ROOT/fixtures/third-party/XCUITestDemo/maestro-job.yaml}"
SETTLE_MS="${LIGH_APP_SETTLE_MS:-3500}"
TIMEOUT_MS="${LIGH_APP_TIMEOUT_MS:-15000}"
MAESTRO_BIN="${MAESTRO_BIN:-$HOME/.maestro/bin/maestro}"
export MAESTRO_CLI_NO_ANALYTICS=1 MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED=1
export PATH="$HOME/.maestro/bin:$PATH"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first (unset CARGO_TARGET_DIR && cargo build --release)"
[[ -x "$ROOT/scripts/build-xcuitestdemo.sh" ]] && "$ROOT/scripts/build-xcuitestdemo.sh" >/tmp/xcd-build.log 2>&1 || true
[[ -d "$APP" ]] || fail "app missing: $APP — run ./scripts/build-xcuitestdemo.sh"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

STEPS='[{"op":"wait","id":"usernameTextField"},{"op":"tap","id":"usernameTextField"},{"op":"type","text":"alice"},{"op":"tap","id":"passwordSecureField"},{"op":"type","text":"secret"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"homeTitle"}]'

parse_ligh_row() {
  python3 - "$1" "$2" "$3" <<'PY'
import json, sys
raw, ms, i = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
row = {"i": i, "ms": ms, "tool": "ligh"}
try:
  d = json.loads(raw) if raw.strip() else {}
except Exception as e:
  row.update({"ok": False, "fault": "infra", "parse_error": str(e)})
  print(json.dumps(row))
  raise SystemExit
row["ok"] = bool(d.get("ok"))
row["fault"] = d.get("fault") or ("ok" if row["ok"] else "fail")
detail = d.get("detail") if isinstance(d.get("detail"), dict) else {}
row["step"] = detail.get("step")
row["op"] = detail.get("op")
obs = d.get("observe") if isinstance(d.get("observe"), dict) else {}
aq = obs.get("ax_quality") or ""
row["ax_empty"] = aq in ("empty", "transition", "error") or bool(obs.get("eyes_unusable"))
row["ax_quality"] = aq
# recovery: attempt > 1 in nested detail
def dig_attempt(obj):
  if not isinstance(obj, dict): return None
  if obj.get("attempt") not in (None, "1", 1): return obj.get("attempt")
  for v in obj.values():
    a = dig_attempt(v) if isinstance(v, dict) else None
    if a not in (None, "1", 1): return a
  return obj.get("attempt")
att = dig_attempt(detail)
row["recovered"] = att not in (None, "1", 1)
row["timeout"] = row["fault"] == "timeout"
row["postcondition"] = "homeTitle" if row["ok"] else None
row["detail"] = detail
print(json.dumps(row))
PY
}

echo "══ third-party rigor N=$N ══"
echo "  app=XCUITestDemo (OSS)  job=login→homeTitle"
echo "  protocol: clean→LIGH×$N → clean→Maestro×$N"

T0=$(python3 -c 'import time; print(time.time())')

# ── Arm A: LIGH ───────────────────────────────────────────────────────────
echo "── Arm A: clean reboot → LIGH × $N ──"
sim_clean_reboot "$LIGH"
ligh_first_loop >/tmp/ligh-rigor-first.log 2>&1

LIGH_ROWS=()
for i in $(seq 1 "$N"); do
  IT0=$(python3 -c 'import time; print(time.time())')
  J=$("$LIGH" --json cap app-job "$APP" --bundle-id "$BUNDLE_ID" --steps "$STEPS" \
      --settle-ms "$SETTLE_MS" --timeout-ms "$TIMEOUT_MS" \
      $([[ "$i" -eq 1 ]] || echo --no-install) 2>/tmp/ligh-rigor-err.txt) || true
  MS=$(python3 -c "import time; print(int((time.time()-float('$IT0'))*1000))")
  ROW=$(parse_ligh_row "$J" "$MS" "$i")
  LIGH_ROWS+=("$ROW")
  echo "$ROW" | python3 -c 'import json,sys; r=json.load(sys.stdin); print(("  ligh #%d %s fault=%s %dms ax_empty=%s recovered=%s") % (r["i"], "PASS" if r["ok"] else "FAIL", r["fault"], r["ms"], r.get("ax_empty"), r.get("recovered")))'
done

# ── Arm B: Maestro ────────────────────────────────────────────────────────
echo "── Arm B: clean reboot → Maestro × $N ──"
sim_clean_reboot "$LIGH"
UDID=$("$LIGH" --json status 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("udid") or (d.get("data") or {}).get("udid") or "")' 2>/dev/null || true)
[[ -n "$UDID" ]] || UDID=$(xcrun simctl list devices booted | grep -oE '[A-F0-9-]{36}' | head -1 || true)
[[ -n "$UDID" ]] || fail "no booted UDID"
xcrun simctl install "$UDID" "$APP" >/dev/null

MAESTRO_ROWS=()
MAESTRO_SKIP=0
if [[ ! -x "$MAESTRO_BIN" ]]; then
  echo "  maestro SKIP (not installed)"
  MAESTRO_SKIP=1
else
  for i in $(seq 1 "$N"); do
    IT0=$(python3 -c 'import time; print(time.time())')
    if "$MAESTRO_BIN" test --udid "$UDID" "$MAESTRO_FLOW" >/tmp/maestro-rigor.log 2>&1; then
      OK=1; FAULT="ok"
    else
      OK=0; FAULT="fail"
      if grep -qi timeout /tmp/maestro-rigor.log 2>/dev/null; then FAULT="timeout"; fi
    fi
    MS=$(python3 -c "import time; print(int((time.time()-float('$IT0'))*1000))")
    ROW=$(python3 -c "import json; print(json.dumps({'i':$i,'ms':$MS,'tool':'maestro','ok':bool($OK),'fault':'$FAULT','timeout':('$FAULT'=='timeout')}))")
    MAESTRO_ROWS+=("$ROW")
    echo "  maestro #$i $( [[ $OK -eq 1 ]] && echo PASS || echo FAIL ) ${MS}ms"
  done
fi

python3 - "$OUT" "$N" "$MAESTRO_SKIP" "$APP" "$BUNDLE_ID" "$T0" "${LIGH_ROWS[@]}" -- "${MAESTRO_ROWS[@]}" <<'PY'
import json, sys, statistics, time

def stats_ms(xs):
  if not xs: return None
  xs = sorted(xs)
  def pct(p):
    k = min(len(xs)-1, max(0, int((p/100.0)*len(xs)+0.5)-1))
    return xs[k]
  return {
    "min_ms": xs[0], "max_ms": xs[-1],
    "p50_ms": pct(50), "p95_ms": pct(95),
    "mean_ms": int(statistics.mean(xs)),
  }

def summarize(rows, tool):
  passed = [r for r in rows if r.get("ok")]
  failed = [r for r in rows if not r.get("ok")]
  ms = [r["ms"] for r in rows]
  faults = {}
  for r in failed:
    k = r.get("fault") or "fail"
    faults[k] = faults.get(k, 0) + 1
  out = {
    "tool": tool,
    "pass": len(passed),
    "total": len(rows),
    "pass_rate": round(len(passed)/float(len(rows)), 4) if rows else 0,
    "latency": stats_ms(ms),
    "fault_taxonomy": faults,
    "results": rows,
  }
  if tool == "ligh":
    out["ax_empty_events"] = sum(1 for r in rows if r.get("ax_empty"))
    out["recovery_events"] = sum(1 for r in rows if r.get("recovered"))
    out["timeout_events"] = sum(1 for r in rows if r.get("timeout"))
    out["wrong_target_silent"] = sum(1 for r in passed if r.get("postcondition") != "homeTitle")
  return out

out, n, skip, app, bid, t0 = sys.argv[1:7]
sep = sys.argv.index("--")
ligh_rows = [json.loads(x) for x in sys.argv[7:sep]]
maestro_rows = [json.loads(x) for x in sys.argv[sep+1:]]
n = int(n); skip = int(skip)
ligh = summarize(ligh_rows, "ligh")
maestro = summarize(maestro_rows, "maestro") if not skip else {"skip": True}
doc = {
  "gate": "third_party_rigor",
  "claim": "XCUITestDemo OSS — isolated arms on clean sim, N=%d each" % n,
  "protocol": ["clean reboot", "LIGH app-job × N", "clean reboot", "Maestro × N"],
  "app_label": "XCUITestDemo (OSS third-party)",
  "bundle_id": bid,
  "app": app,
  "job": "login: alice/secret → homeTitle",
  "n_per_arm": n,
  "total_ms": int((time.time()-float(t0))*1000),
  "ligh": ligh,
  "maestro": maestro,
  "comparison": {
    "reliability_ligh_wins": (ligh["pass"] >= maestro.get("pass", 0)) if not skip else None,
    "latency_p50_ratio": (
      round(maestro["latency"]["p50_ms"] / float(ligh["latency"]["p50_ms"]), 2)
      if (not skip and ligh.get("latency") and maestro.get("latency")) else None
    ),
  },
}
open(out, "w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps({
  "out": out,
  "ligh": "%d/%d p50=%s" % (ligh["pass"], ligh["total"], ligh["latency"]["p50_ms"] if ligh.get("latency") else "?"),
  "maestro": ("%d/%d p50=%s" % (maestro["pass"], maestro["total"], maestro["latency"]["p50_ms"])) if not skip else "skip",
}, indent=2))
PY

echo "══ → $OUT"
