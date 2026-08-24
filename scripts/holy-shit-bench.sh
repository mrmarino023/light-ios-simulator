#!/usr/bin/env bash
# Holy-shit bench — sim microbench + optional killer-loop A/B + physical micro.
#
# Publishes docs/assets/holy-shit-bench-latest.json
#
# Usage:
#   ./scripts/holy-shit-bench.sh
#   LIGH_HOLY_SKIP_KILLER=1 ./scripts/holy-shit-bench.sh
#   LIGH_HOLY_PHYSICAL=1 ./scripts/holy-shit-bench.sh   # needs phone + wda.env
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_HOLY_OUT:-$ROOT/docs/assets/holy-shit-bench-latest.json}"
ITER="${LIGH_HOLY_ITER:-12}"
SKIP_KILLER="${LIGH_HOLY_SKIP_KILLER:-0}"
PHYSICAL="${LIGH_HOLY_PHYSICAL:-0}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -x "$LIGH" ]] || { echo "building…"; (cd "$ROOT" && cargo build --release -p ligh-cli -p ligh-daemon); }
[[ -x "$LIGH" ]] || fail "missing $LIGH"

echo "══ Holy-shit bench ══"
"$LIGH" daemon stop --json 2>/dev/null || true
pkill -x lighd 2>/dev/null || true
sleep 1
nohup "$ROOT/target/release/lighd" >> /tmp/lighd-holy.log 2>&1 &
sleep 1

echo "▶ sim up"
"$LIGH" up --device iphone-15-pro >/dev/null 2>&1 || "$LIGH" up --device iphone-15-pro

echo "▶ sim agent microbench (iter=$ITER)"
MICRO_JSON="/tmp/ligh-holy-micro.json"
"$LIGH" --json bench agent --iterations "$ITER" --with-micro --micro-only --no-wda >"$MICRO_JSON" 2>/tmp/ligh-holy-micro.log || true

KILLER_JSON=""
if [[ "$SKIP_KILLER" != "1" && -n "${OPENAI_API_KEY:-}" ]]; then
  echo "▶ killer-loop A/B v2 (needs OPENAI_API_KEY)"
  if "$ROOT/scripts/gate-killer-loop-ab-v2.sh" >/tmp/ligh-holy-killer.log 2>&1; then
    KILLER_JSON="$ROOT/docs/assets/killer-loop-ab-v2-latest.json"
  else
    echo "  ⚠ killer loop skipped/failed — see /tmp/ligh-holy-killer.log"
  fi
else
  echo "▶ killer-loop skipped (set OPENAI_API_KEY or unset LIGH_HOLY_SKIP_KILLER=0)"
fi

PHYSICAL_JSON="null"
if [[ "$PHYSICAL" == "1" ]]; then
  echo "▶ physical micro (device wait + tab taps)"
  export LIGH ROOT ITER
  PHYSICAL_JSON="$(python3 - <<'PY' 2>/tmp/ligh-holy-phys.log || echo 'null'
import json, subprocess, time, os, statistics
root = os.environ.get("ROOT", ".")
ligh = os.environ["LIGH"]
iter_n = int(os.environ.get("ITER", "5"))

def run(cmd):
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    return time.time() - t0, r.stdout, r.returncode

subprocess.run(f"{ligh} device wait --timeout 45", shell=True, check=False)

obs, tap = [], []
for i in range(iter_n):
    dt, out, rc = run(f'{ligh} observe --json --no-settle')
    if rc == 0: obs.append(dt * 1000)
    dt, out, rc = run(f'{ligh} tap --json --label TabProfile 2>/dev/null || {ligh} tap --json --label TabEventsHome')
    if rc == 0:
        try:
            j = json.loads(out)
            tap.append(dt * 1000)
        except: pass
    time.sleep(0.3)

def stats(xs):
    if not xs: return {"n": 0, "p50_ms": None, "p95_ms": None}
    xs = sorted(xs)
    p50 = xs[len(xs)//2]
    p95 = xs[max(0, int(len(xs)*0.95)-1)]
    return {"n": len(xs), "p50_ms": round(p50, 1), "p95_ms": round(p95, 1)}

print(json.dumps({"observe_ms": stats(obs), "tap_ms": stats(tap)}))
PY
)"
fi

python3 - "$OUT" "$MICRO_JSON" "$KILLER_JSON" "$PHYSICAL_JSON" <<'PY'
import json, sys, time, platform, os

out, micro_path, killer_path, physical_raw = sys.argv[1:5]
micro = {}
try:
    micro = json.load(open(micro_path))
except Exception as e:
    micro = {"error": str(e), "log": "/tmp/ligh-holy-micro.log"}

killer = None
if killer_path and os.path.isfile(killer_path):
    killer = json.load(open(killer_path))

physical = None
if physical_raw.strip() not in ("", "null"):
    try:
        physical = json.loads(physical_raw)
    except Exception:
        physical = {"raw": physical_raw[:200]}

doc = {
    "bench": "holy_shit",
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "platform": platform.platform(),
    "sim_micro": micro,
    "killer_loop_ab_v2": killer,
    "physical_micro": physical,
    "claims": {
        "sim_speedup_vs_wda": micro.get("comparisons", {}).get("ligh_vs_wda_speedup"),
        "killer_autopilot_speedup": (killer or {}).get("speedup_wall"),
        "killer_autopilot_tokens_ratio": (killer or {}).get("token_ratio"),
    },
}
json.dump(doc, open(out, "w"), indent=2)
print(json.dumps(doc.get("claims"), indent=2))
print("wrote", out)
PY

echo "✓ holy-shit bench → $OUT"
