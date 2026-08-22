#!/usr/bin/env bash
# Cold start → first app-job green. Publish bar: < 5 min (300000 ms).
#
# Measures: daemon bounce → up → SpringBoard AX ready → one app-job.
# Usage: ./scripts/gate-cold-start.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
BUDGET_MS="${LIGH_COLD_BUDGET_MS:-300000}"
OUT="${LIGH_COLD_OUT:-$ROOT/docs/assets/cold-start-latest.json}"
APP="${LIGH_APP_PATH:-$ROOT/fixtures/LighFixture/build/LighFixture.app}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
[[ -d "$APP" ]] || "$ROOT/scripts/build-fixture.sh"

echo "══ cold start → app-job ══"
echo "  budget_ms=$BUDGET_MS"

T0=$(python3 -c 'import time; print(time.time())')

"$LIGH" daemon stop 2>/dev/null || true
sleep 1
"$LIGH" daemon start
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-cold-first.log 2>&1 || fail "agent-first-loop failed (see /tmp/ligh-cold-first.log)"

STEPS='[{"op":"wait","id":"LighHome"},{"op":"tap","id":"NameField"},{"op":"type","text":"cold"},{"op":"tap","id":"GoNext"},{"op":"wait","id":"LighDone"}]'
J=""
for attempt in 1 2; do
  if J=$("$LIGH" --json cap app-job "$APP" --bundle-id dev.ligh.Fixture --steps "$STEPS" \
      --settle-ms 3500 --timeout-ms 25000 2>/tmp/ligh-cold-err.txt); then
    echo "$J" | python3 -c 'import json,sys; d=json.load(sys.stdin); raise SystemExit(0 if d.get("ok") else 1)' && break
  fi
  echo "  app-job attempt $attempt failed — retry after settle" >&2
  sleep 2
  "$LIGH" --json observe --settle-ms 3000 >/dev/null 2>&1 || true
done

MS=$(python3 -c "import time; print(int((time.time()-float('$T0'))*1000))")

python3 - "$OUT" "$J" "$MS" "$BUDGET_MS" "$APP" <<'PY'
import json, sys
out, raw, ms, budget, app = sys.argv[1:6]
ms=int(ms); budget=int(budget)
err_tail = ""
try:
  err_tail = open("/tmp/ligh-cold-err.txt").read()[-400:]
except OSError:
  pass
try:
  d=json.loads(raw)
  ok=bool(d.get("ok"))
  fault=d.get("fault") or ("ok" if ok else "fail")
except Exception as e:
  ok=False; fault="infra"
  d={"error": str(e), "raw": (raw or "")[:500], "stderr_tail": err_tail}
claim = ok and ms <= budget
doc={
  "gate":"cold_start",
  "claim":"daemon bounce→up→AX ready→app-job under 5 min",
  "ms":ms,
  "budget_ms":budget,
  "claim_pass":claim,
  "app_job_ok":ok,
  "fault":fault if not ok else "ok",
  "detail":d.get("detail") if isinstance(d, dict) else None,
  "app":app,
}
open(out,"w").write(json.dumps(doc,indent=2)+"\n")
print(json.dumps({"claim_pass":claim,"ms":ms,"budget_ms":budget,"app_job_ok":ok,"out":out},indent=2))
raise SystemExit(0 if claim else 1)
PY

echo "✓ cold start gate done"
