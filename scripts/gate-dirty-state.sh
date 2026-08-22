#!/usr/bin/env bash
# Dirty-state reliability: N app-jobs back-to-back on ONE sim session (no reboot).
# Product bar: survive agent build→test→fix loops without AX death spiral.
#
# Usage: LIGH_DIRTY_N=50 ./scripts/gate-dirty-state.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_DIRTY_N:-50}"
OUT="${LIGH_DIRTY_OUT:-$ROOT/docs/assets/dirty-state-latest.json}"
APP="${LIGH_APP_PATH:-$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app}"
BUNDLE_ID="${LIGH_APP_BUNDLE_ID:-com.himali.XCUITestDemo}"
SETTLE_MS="${LIGH_APP_SETTLE_MS:-3500}"
TIMEOUT_MS="${LIGH_APP_TIMEOUT_MS:-15000}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
[[ -d "$APP" ]] || "$ROOT/scripts/build-xcuitestdemo.sh"

STEPS='[{"op":"wait","id":"usernameTextField"},{"op":"tap","id":"usernameTextField"},{"op":"type","text":"alice"},{"op":"tap","id":"passwordSecureField"},{"op":"type","text":"secret"},{"op":"tap","id":"loginButton"},{"op":"wait","id":"homeTitle"}]'

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

echo "══ dirty-state N=$N (no sim reboot between iters) ══"
sim_clean_reboot "$LIGH"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/dirty-first.log 2>&1 || fail "first-loop failed"

RESULTS_FILE=$(mktemp)
trap 'rm -f "$RESULTS_FILE"' EXIT
T0=$(python3 -c 'import time; print(time.time())')

for i in $(seq 1 "$N"); do
  IT0=$(python3 -c 'import time; print(time.time())')
  J=$("$LIGH" --json cap app-job "$APP" --bundle-id "$BUNDLE_ID" --steps "$STEPS" \
      --settle-ms "$SETTLE_MS" --timeout-ms "$TIMEOUT_MS" \
      $([[ "$i" -eq 1 ]] || echo --no-install) 2>/tmp/dirty-err.txt) || true
  MS=$(python3 -c "import time; print(int((time.time()-float('$IT0'))*1000))")
  python3 - "$MS" "$i" "$J" >> "$RESULTS_FILE" <<'PY'
import json, sys
ms, i, raw = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
try:
  d = json.loads(raw) if raw.strip() else {}
except Exception as e:
  print(json.dumps({"i":i,"ok":False,"fault":"infra","ms":ms,"error":str(e)}))
  raise SystemExit
ok = bool(d.get("ok"))
fault = d.get("fault") or ("ok" if ok else "fail")
detail = d.get("detail") if isinstance(d.get("detail"), dict) else {}
obs = d.get("observe") if isinstance(d.get("observe"), dict) else {}
aq = obs.get("ax_quality") or ""
def dig_attempt(obj):
  if not isinstance(obj, dict): return None
  a = obj.get("attempt")
  if a not in (None, "1", 1): return a
  for v in obj.values():
    if isinstance(v, dict):
      x = dig_attempt(v)
      if x not in (None, "1", 1): return x
  return a
print(json.dumps({
  "i": i, "ok": ok, "fault": fault, "ms": ms,
  "step": detail.get("step"), "op": detail.get("op"),
  "ax_empty": aq in ("empty", "transition", "error") or bool(obs.get("eyes_unusable")),
  "recovered": dig_attempt(detail) not in (None, "1", 1),
  "timeout": fault == "timeout",
}))
PY
  ROW=$(tail -1 "$RESULTS_FILE")
  echo "$ROW" | python3 -c 'import json,sys; r=json.load(sys.stdin); print("  #%d %s fault=%s %dms ax_empty=%s" % (r["i"], "PASS" if r["ok"] else "FAIL", r["fault"], r["ms"], r.get("ax_empty")))'
done

python3 - "$OUT" "$N" "$RESULTS_FILE" "$T0" "$APP" "$BUNDLE_ID" <<'PY'
import json, sys, statistics, time
out, n, path, t0, app, bid = sys.argv[1:7]
n = int(n)
rows = [json.loads(l) for l in open(path) if l.strip()]
passed = [r for r in rows if r.get("ok")]
failed = [r for r in rows if not r.get("ok")]
warm_ms = [r["ms"] for r in rows[1:] if r.get("ok")]
def pct(xs, p):
  if not xs: return None
  xs = sorted(xs)
  k = min(len(xs)-1, max(0, int((p/100.0)*len(xs)+0.5)-1))
  return xs[k]
faults = {}
for r in failed:
  k = r.get("fault") or "fail"
  faults[k] = faults.get(k, 0) + 1
# streak: first failure index
first_fail = next((r["i"] for r in rows if not r.get("ok")), None)
ax_empty = sum(1 for r in rows if r.get("ax_empty"))
recovered = sum(1 for r in rows if r.get("recovered"))
claim_pass = len(passed) == n and ax_empty == 0
doc = {
  "gate": "dirty_state",
  "claim": "N app-jobs on one sim session — no reboot between iterations",
  "app": "XCUITestDemo (OSS third-party)",
  "bundle_id": bid,
  "app_path": app,
  "n": n,
  "pass": len(passed),
  "fail": len(failed),
  "pass_rate": round(len(passed)/float(n), 4) if n else 0,
  "first_failure_at": first_fail,
  "ax_empty_events": ax_empty,
  "recovery_events": recovered,
  "timeout_events": sum(1 for r in rows if r.get("timeout")),
  "fault_taxonomy": faults,
  "latency_ms": {
    "warm_p50": pct(warm_ms, 50),
    "warm_p95": pct(warm_ms, 95),
    "min": min((r["ms"] for r in rows), default=None),
    "max": max((r["ms"] for r in rows), default=None),
  },
  "claim_pass": claim_pass,
  "product_blocker": ax_empty > 0 or len(failed) > 0,
  "total_ms": int((time.time()-float(t0))*1000),
  "results": rows,
}
open(out, "w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps({
  "claim_pass": claim_pass,
  "pass": len(passed), "n": n,
  "ax_empty_events": ax_empty,
  "first_failure_at": first_fail,
  "out": out,
}, indent=2))
raise SystemExit(0 if claim_pass else 1)
PY

echo "══ → $OUT"
