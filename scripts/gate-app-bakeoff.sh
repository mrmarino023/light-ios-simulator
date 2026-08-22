#!/usr/bin/env bash
# Fair bakeoff: same Debug .app job on LIGH app-job vs Maestro (if installed).
# Competitive wedge — not Settings cosplay.
#
# Job: Home → type → GoNext → Done (LighFixture)
#
# Usage:
#   ./scripts/gate-app-bakeoff.sh
#   LIGH_APP_N=10 ./scripts/gate-app-bakeoff.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_APP_N:-10}"
OUT="${LIGH_BAKEOFF_OUT:-$ROOT/docs/assets/app-bakeoff-latest.json}"
APP="${LIGH_APP_PATH:-$ROOT/fixtures/LighFixture/build/LighFixture.app}"
BUNDLE_ID="${LIGH_APP_BUNDLE_ID:-dev.ligh.Fixture}"
APP_LABEL="${LIGH_APP_LABEL:-LighFixture}"
MAESTRO_FLOW="${LIGH_MAESTRO_FLOW:-$ROOT/fixtures/LighFixture/maestro-job.yaml}"
SETTLE_MS="${LIGH_APP_SETTLE_MS:-3500}"
TIMEOUT_MS="${LIGH_APP_TIMEOUT_MS:-12000}"
BUILD_SCRIPT="${LIGH_APP_BUILD_SCRIPT:-}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
if [[ ! -d "$APP" ]]; then
  if [[ -n "$BUILD_SCRIPT" && -x "$ROOT/scripts/$(basename "$BUILD_SCRIPT")" ]]; then
    "$ROOT/scripts/$(basename "$BUILD_SCRIPT")"
  elif [[ -x "$BUILD_SCRIPT" ]]; then
    "$BUILD_SCRIPT"
  else
    "$ROOT/scripts/build-fixture.sh"
  fi
  APP="${LIGH_APP_PATH:-$ROOT/fixtures/LighFixture/build/LighFixture.app}"
fi
[[ -d "$APP" ]] || fail "app not found: $APP"

steps_json() {
  local i="$1"
  case "${LIGH_APP_PROFILE:-fixture}" in
    xcuitestdemo)
      python3 -c "import json; print(json.dumps([
        {'op':'wait','id':'usernameTextField'},
        {'op':'tap','id':'usernameTextField'},
        {'op':'type','text':'alice'},
        {'op':'tap','id':'passwordSecureField'},
        {'op':'type','text':'secret'},
        {'op':'tap','id':'loginButton'},
        {'op':'wait','id':'homeTitle'},
      ]))"
      ;;
    *)
      python3 -c "import json; print(json.dumps([
        {'op':'wait','id':'LighHome'},
        {'op':'tap','id':'NameField'},
        {'op':'type','text':'bake$i'},
        {'op':'tap','id':'GoNext'},
        {'op':'wait','id':'LighDone'},
      ]))"
      ;;
  esac
}

job_description() {
  case "${LIGH_APP_PROFILE:-fixture}" in
    xcuitestdemo) echo "login: user+pass → homeTitle" ;;
    *) echo "wait LighHome → tap NameField → type → tap GoNext → wait LighDone" ;;
  esac
}

JOB_DESC=$(job_description)

echo "══ app bakeoff N=$N ══"
echo "  app_label=$APP_LABEL"
echo "  job: $JOB_DESC"
echo "  app=$APP"

"$LIGH" daemon status >/dev/null 2>&1 || "$LIGH" daemon start
"$ROOT/scripts/agent-first-loop.sh" >/tmp/ligh-bakeoff-first.log 2>&1 \
  || fail "first-loop failed"

# ── LIGH arm ──────────────────────────────────────────────────────────────
LIGH_PASS=0
LIGH_MS=()
for i in $(seq 1 "$N"); do
  STEPS=$(steps_json "$i")
  T0=$(python3 -c 'import time; print(time.time())')
  if "$LIGH" --json cap app-job "$APP" --bundle-id "$BUNDLE_ID" --steps "$STEPS" \
      --settle-ms "$SETTLE_MS" --timeout-ms "$TIMEOUT_MS" \
      $([[ "$i" -eq 1 ]] || echo --no-install) >/tmp/ligh-bake-iter.json 2>/tmp/ligh-bake-err.txt; then
    OK=$(python3 -c 'import json; print(1 if json.load(open("/tmp/ligh-bake-iter.json")).get("ok") else 0)')
  else
    OK=0
  fi
  MS=$(python3 -c "import time; print(int((time.time()-float('$T0'))*1000))")
  LIGH_MS+=("$MS")
  if [[ "$OK" -eq 1 ]]; then
    LIGH_PASS=$((LIGH_PASS + 1))
    echo "  ligh #$i PASS ${MS}ms"
  else
    echo "  ligh #$i FAIL ${MS}ms"
  fi
done

# ── Maestro arm (optional) ─────────────────────────────────────────────────
MAESTRO_BIN="${MAESTRO_BIN:-$(command -v maestro 2>/dev/null || true)}"
[[ -n "$MAESTRO_BIN" ]] || MAESTRO_BIN="${HOME}/.maestro/bin/maestro"
MAESTRO_PASS=0
MAESTRO_SKIP=1
MAESTRO_MS=()

if [[ -n "$MAESTRO_BIN" && -x "$MAESTRO_BIN" ]]; then
  MAESTRO_SKIP=0
  # Ensure app installed for Maestro too
  UDID=$("$LIGH" --json status 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); print((d.get("data") or d).get("udid") or "")' 2>/dev/null || true)
  if [[ -z "$UDID" ]]; then
    UDID=$(xcrun simctl list devices booted | grep -oE '[A-F0-9-]{36}' | head -1 || true)
  fi
  [[ -n "$UDID" ]] || fail "no booted sim UDID for Maestro"
  xcrun simctl install "$UDID" "$APP" >/dev/null
  for i in $(seq 1 "$N"); do
    T0=$(python3 -c 'import time; print(time.time())')
    if "$MAESTRO_BIN" test --udid "$UDID" "$MAESTRO_FLOW" >/tmp/maestro-bake.log 2>&1; then
      MAESTRO_PASS=$((MAESTRO_PASS + 1))
      MS=$(python3 -c "import time; print(int((time.time()-float('$T0'))*1000))")
      MAESTRO_MS+=("$MS")
      echo "  maestro #$i PASS ${MS}ms"
    else
      MS=$(python3 -c "import time; print(int((time.time()-float('$T0'))*1000))")
      MAESTRO_MS+=("$MS")
      echo "  maestro #$i FAIL ${MS}ms"
    fi
  done
else
  echo "  maestro SKIP (install maestro CLI to compare — https://maestro.mobile.dev)"
fi

python3 - "$OUT" "$N" "$LIGH_PASS" "$MAESTRO_PASS" "$MAESTRO_SKIP" "$APP" "$APP_LABEL" "$BUNDLE_ID" "$JOB_DESC" "${LIGH_APP_PROFILE:-fixture}" "${LIGH_MS[@]}" -- "${MAESTRO_MS[@]}" <<'PY'
import json, sys, statistics
out, n, lp, mp, skip, app, label, bid, job_desc, profile = sys.argv[1:11]
sep = sys.argv.index("--")
ligh_ms = [int(x) for x in sys.argv[11:sep]]
maestro_ms = [int(x) for x in sys.argv[sep+1:]]
n = int(n); lp=int(lp); mp=int(mp); skip=int(skip)
def stats(xs):
  if not xs: return None
  xs=sorted(xs)
  return {"p50_ms": xs[len(xs)//2], "p95_ms": xs[max(0,int(len(xs)*0.95)-1)], "mean_ms": int(statistics.mean(xs))}
doc = {
  "gate": "app_bakeoff",
  "claim": "same Debug.app job: LIGH app-job vs Maestro",
  "profile": profile,
  "app_label": label,
  "bundle_id": bid,
  "n": n,
  "app": app,
  "job": job_desc,
  "ligh": {"pass": lp, "total": n, "pass_rate": round(lp/n,4) if n else 0, "latency": stats(ligh_ms)},
  "maestro": {
    "skip": bool(skip),
    "pass": mp if not skip else None,
    "total": n if not skip else None,
    "pass_rate": (round(mp/n,4) if n and not skip else None),
    "latency": stats(maestro_ms) if not skip else None,
  },
  "ligh_wins_reliability": (lp >= mp) if not skip else None,
  "ligh_wins_latency_p50": (
    stats(ligh_ms)["p50_ms"] < stats(maestro_ms)["p50_ms"]
    if (not skip and ligh_ms and maestro_ms) else None
  ),
  "maestro_wins_reliability": (mp > lp) if not skip else None,
}
open(out,"w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps(doc, indent=2))
PY

echo "══ wrote $OUT"
