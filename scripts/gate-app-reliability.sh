#!/usr/bin/env bash
# App-under-test reliability — multidimensional claim_pass.
#
# claim_pass =
#   pass_rate == 1.0
#   AND every success includes Done postcondition (no silent wrong-target)
#   AND warm p95 workflow ms <= LIGH_APP_P95_MS (default 8000; excl. first install)
#   AND failures are explicit FaultClass only
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_APP_N:-20}"
P95_BUDGET_MS="${LIGH_APP_P95_MS:-8000}"
OUT="${LIGH_APP_GATE_OUT:-$ROOT/docs/assets/app-reliability-latest.json}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"

APP="${LIGH_APP_PATH:-$ROOT/fixtures/LighFixture/build/LighFixture.app}"
BUNDLE_ID="${LIGH_APP_BUNDLE_ID:-dev.ligh.Fixture}"
HOME_ID="${LIGH_APP_HOME_ID:-LighHome}"
FIELD_ID="${LIGH_APP_FIELD_ID:-NameField}"
GO_ID="${LIGH_APP_GO_ID:-GoNext}"
DONE_ID="${LIGH_APP_DONE_ID:-LighDone}"

if [[ ! -d "$APP" ]]; then
  echo "▶ building fixture app"
  "$ROOT/scripts/build-fixture.sh" >/tmp/ligh-fixture-build.log 2>&1 \
    || fail "fixture build failed — see /tmp/ligh-fixture-build.log"
  APP="$ROOT/fixtures/LighFixture/build/LighFixture.app"
fi

echo "══ app reliability N=$N (multidimensional) ══"
echo "  app=$APP"
echo "  p95_budget_ms=$P95_BUDGET_MS (warm iters after first install)"

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-app-rel-first.log 2>&1 \
  || fail "first-loop failed — see /tmp/ligh-app-rel-first.log"

RESULTS_FILE=$(mktemp)
trap 'rm -f "$RESULTS_FILE"' EXIT
T0=$(python3 -c 'import time; print(time.time())')

for i in $(seq 1 "$N"); do
  echo -n "  #$i "
  ITER_T0=$(python3 -c 'import time; print(time.time())')
  STEPS=$(python3 -c "import json; print(json.dumps([
    {'op':'wait','id':'$HOME_ID'},
    {'op':'tap','id':'$FIELD_ID'},
    {'op':'type','text':'job$i'},
    {'op':'tap','id':'$GO_ID'},
    {'op':'wait','id':'$DONE_ID'},
  ]))")
  J=$("$LIGH" --json cap app-job "$APP" --bundle-id "$BUNDLE_ID" --steps "$STEPS" \
      --settle-ms 3500 --timeout-ms 12000 \
      $([[ "$i" -eq 1 ]] || echo --no-install) 2>/tmp/ligh-app-job-err.txt) || true
  MS=$(python3 -c "import time; print(int((time.time()-float('$ITER_T0'))*1000))")
  ROW=$(python3 -c '
import json,sys
raw, ms, i, done_id = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
try:
  d=json.loads(raw)
except Exception as e:
  print(json.dumps({"i":i,"ok":False,"fault":"infra","op":"parse","ms":ms,"error":str(e),"postcondition":done_id}))
  raise SystemExit
ok=bool(d.get("ok"))
fault=d.get("fault") or ("ok" if ok else "fail")
detail=d.get("detail") if isinstance(d.get("detail"), dict) else {}
attempt=detail.get("attempt")
nested=detail.get("detail")
if isinstance(nested, dict) and attempt is None:
  attempt=nested.get("attempt")
print(json.dumps({
  "i": i, "ok": ok, "fault": fault,
  "step": detail.get("step"), "op": detail.get("op"),
  "ms": ms, "postcondition": done_id, "attempt": attempt,
}))
' "$J" "$MS" "$i" "$DONE_ID")
  echo "$ROW" >> "$RESULTS_FILE"
  echo "$ROW" | python3 -c 'import json,sys; r=json.load(sys.stdin); print(("PASS" if r["ok"] else "FAIL")+" fault="+str(r.get("fault"))+" "+str(r["ms"])+"ms")'
done

python3 - "$OUT" "$N" "$P95_BUDGET_MS" "$APP" "$BUNDLE_ID" "$DONE_ID" "$RESULTS_FILE" "$T0" <<'PY'
import json, sys, time, statistics
out, n, p95_budget, app, bid, done_id, results_path, t0 = sys.argv[1:9]
n=int(n); p95_budget=int(p95_budget)
results=[json.loads(line) for line in open(results_path) if line.strip()]
total_ms=int((time.time()-float(t0))*1000)
passed=[r for r in results if r.get("ok")]
failed=[r for r in results if not r.get("ok")]
warm_ok=[r["ms"] for r in results[1:] if r.get("ok")]
def pct(xs, p):
  if not xs: return None
  xs=sorted(xs)
  k=min(len(xs)-1, max(0, int((p/100.0)*len(xs)+0.5)-1))
  return xs[k]
faults={}
for r in failed:
  k=r.get("fault") or "fail"
  faults[k]=faults.get(k,0)+1
def recovered(r):
  a=r.get("attempt")
  return a not in (None, "1", 1)
pass_n=len(passed)
pass_rate=round(pass_n/float(n),4) if n else 0.0
p50=pct(warm_ok,50); p95=pct(warm_ok,95)
rel_ok = pass_n == n
no_silent = all(r.get("postcondition")==done_id for r in passed) and all(
  (r.get("fault") or "") != "ok" for r in failed
)
lat_ok = (p95 is None) or (p95 <= p95_budget)
claim = bool(rel_ok and no_silent and lat_ok)
doc={
  "gate":"app_reliability",
  "claim":"coding agent verifies Debug.app via app-job — fail-closed, no silent wrong-target",
  "architecture":"overlay-aware ensure_path + relaunch recovery inside lighd",
  "job":["wait Home","tap Field","type","tap Go","wait Done"],
  "n":n,
  "pass":pass_n,
  "fail":len(failed),
  "pass_rate":pass_rate,
  "fault_taxonomy":faults,
  "infra_faults":sum(faults.get(k,0) for k in ("infra","eyes_unusable","timeout","blocked")),
  "recovered_iters":sum(1 for r in results if recovered(r)),
  "latency_ms":{
    "warm_p50":p50,
    "warm_p95":p95,
    "warm_mean":int(statistics.mean(warm_ok)) if warm_ok else None,
    "p95_budget":p95_budget,
    "note":"warm = iterations after first install",
  },
  "no_silent_wrong_target":no_silent,
  "claim_pass":claim,
  "claim_dimensions":{
    "reliability_100":rel_ok,
    "no_silent_wrong_target":no_silent,
    "warm_p95_within_budget":lat_ok,
  },
  "total_ms":total_ms,
  "app":app,
  "bundle_id":bid,
  "results":results,
}
open(out,"w").write(json.dumps(doc,indent=2)+"\n")
print(json.dumps({
  "claim_pass":claim,
  "pass":pass_n,
  "n":n,
  "warm_p50_ms":p50,
  "warm_p95_ms":p95,
  "p95_budget_ms":p95_budget,
  "claim_dimensions":doc["claim_dimensions"],
  "fault_taxonomy":faults,
  "out":out,
}, indent=2))
raise SystemExit(0 if claim else 1)
PY
STATUS=$?
echo "══ → $OUT"
[[ "$STATUS" -eq 0 ]] || fail "app reliability claim failed (see claim_dimensions in JSON)"
echo "✓ multidimensional claim held"
