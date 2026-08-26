#!/usr/bin/env bash
# Holy-shit bench — competitive product proof across apps + tasks.
#
# Layers (publish all, pass or fail):
#   1. Workflow matrix   — 5 apps × 5 motor workflows (login, scroll, modal, wizard, nav)
#   2. Autopilot generality — one host policy, 6 flow shapes, zero LLM tokens
#   3. Sim microbench    — observe/tap p50 vs WDA baseline
#   4. Killer A/B v2     — autopilot vs vision on N frozen bug tasks (needs OPENAI_API_KEY)
#   5. Physical micro    — optional DevDriver tab taps
#
# Publishes docs/assets/holy-shit-bench-latest.json
#
# Usage:
#   ./scripts/holy-shit-bench.sh
#   LIGH_HOLY_SKIP_MATRIX=1 ./scripts/holy-shit-bench.sh
#   LIGH_HOLY_SKIP_PILOT=1 ./scripts/holy-shit-bench.sh
#   LIGH_HOLY_KILLER_TASKS="login-never-navigates kix-notes-tab-missing" ./scripts/holy-shit-bench.sh
#   LIGH_HOLY_PHYSICAL=1 ./scripts/holy-shit-bench.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_HOLY_OUT:-$ROOT/docs/assets/holy-shit-bench-latest.json}"
ITER="${LIGH_HOLY_ITER:-12}"
SKIP_KILLER="${LIGH_HOLY_SKIP_KILLER:-0}"
SKIP_MATRIX="${LIGH_HOLY_SKIP_MATRIX:-0}"
SKIP_PILOT="${LIGH_HOLY_SKIP_PILOT:-0}"
PHYSICAL="${LIGH_HOLY_PHYSICAL:-0}"
PILOT_APPS="${LIGH_HOLY_PILOT_APPS:-lighfixture lighonboard lighmodal lighfeed xcuitestdemo kix}"
KILLER_TASKS="${LIGH_HOLY_KILLER_TASKS:-login-never-navigates kix-notes-tab-missing onboarding-home-broken}"

fail() { echo "✗ $*" >&2; exit 1; }

[[ -x "$LIGH" ]] || { echo "building…"; (cd "$ROOT" && cargo build --release -p ligh-cli -p ligh-daemon); }
[[ -x "$LIGH" ]] || fail "missing $LIGH"

echo "══ Holy-shit bench (multi-app · multi-task · competitive) ══"
# Simulator product proof — never let Mae DevDriver / ~/.ligh/wda.env own AX.
export LIGH_UI="${LIGH_UI:-sim}"
if [[ "$LIGH_UI" == "sim" || "$LIGH_UI" == "simulator" ]]; then
  unset LIGH_WDA_UDID LIGH_WDA_BUNDLE LIGH_WDA_URL LIGH_WDA_SESSION || true
fi
"$LIGH" daemon stop --json 2>/dev/null || true
pkill -x lighd 2>/dev/null || true
sleep 1
nohup env -u LIGH_WDA_UDID -u LIGH_WDA_BUNDLE -u LIGH_WDA_URL -u LIGH_WDA_SESSION \
  LIGH_UI=sim "$ROOT/target/release/lighd" >> /tmp/lighd-holy.log 2>&1 &
sleep 1

echo "▶ sim up"
"$LIGH" up --device iphone-15-pro >/dev/null 2>&1 || "$LIGH" up --device iphone-15-pro

MATRIX_JSON=""
if [[ "$SKIP_MATRIX" != "1" ]]; then
  echo "▶ workflow matrix (5 apps × 5 tasks — motor generalization)"
  if "$ROOT/scripts/gate-workflow-matrix.sh" >/tmp/ligh-holy-matrix.log 2>&1; then
    MATRIX_JSON="$ROOT/docs/assets/workflow-matrix-latest.json"
  else
    echo "  ⚠ workflow matrix failed — see /tmp/ligh-holy-matrix.log"
    MATRIX_JSON="$ROOT/docs/assets/workflow-matrix-latest.json"
  fi
else
  echo "▶ workflow matrix skipped (LIGH_HOLY_SKIP_MATRIX=1)"
fi

PILOT_JSON=""
if [[ "$SKIP_PILOT" != "1" ]]; then
  echo "▶ autopilot generality ($PILOT_APPS — zero LLM, one policy)"
  if LIGH_PILOT_APPS="$PILOT_APPS" "$ROOT/scripts/gate-autopilot-generality.sh" >/tmp/ligh-holy-pilot.log 2>&1; then
    PILOT_JSON="$ROOT/docs/assets/autopilot-generality-latest.json"
  else
    echo "  ⚠ autopilot generality failed — see /tmp/ligh-holy-pilot.log"
    PILOT_JSON="$ROOT/docs/assets/autopilot-generality-latest.json"
  fi
else
  echo "▶ autopilot generality skipped (LIGH_HOLY_SKIP_PILOT=1)"
fi

echo "▶ sim agent microbench (iter=$ITER)"
MICRO_JSON="/tmp/ligh-holy-micro.json"
"$LIGH" --json bench agent --iterations "$ITER" --with-micro --micro-only --no-wda >"$MICRO_JSON" 2>/tmp/ligh-holy-micro.log || true

KILLER_DIR="/tmp/ligh-holy-killer-tasks"
mkdir -p "$KILLER_DIR"
KILLER_INDEX="$KILLER_DIR/index.json"
KILLER_JSON=""

if [[ "$SKIP_KILLER" != "1" && -n "${OPENAI_API_KEY:-}" ]]; then
  echo "▶ killer-loop A/B v2 — tasks: $KILLER_TASKS"
  killer_rows=()
  for task_id in $KILLER_TASKS; do
    TASK="$ROOT/fixtures/frozen/tasks/$task_id/task.json"
    if [[ ! -f "$TASK" ]]; then
      echo "  ⚠ missing task $task_id — skip"
      continue
    fi
    TASK_OUT="$KILLER_DIR/${task_id}.json"
    echo "  ── task=$task_id"
    if LIGH_KILLER_TASK="$TASK" LIGH_AB2_OUT="$TASK_OUT" LIGH_AB2_REPEAT=1 \
      "$ROOT/scripts/gate-killer-loop-ab-v2.sh" >/tmp/ligh-holy-killer-"$task_id".log 2>&1; then
      killer_rows+=("$task_id")
    else
      echo "    ⚠ failed — /tmp/ligh-holy-killer-${task_id}.log"
      [[ -f "$TASK_OUT" ]] && killer_rows+=("$task_id")
    fi
  done
  python3 - "$KILLER_INDEX" "$KILLER_DIR" "${killer_rows[@]+"${killer_rows[@]}"}" <<'PY'
import json, os, sys
out, kdir = sys.argv[1], sys.argv[2]
ids = sys.argv[3:]
rows = []
for tid in ids:
    p = os.path.join(kdir, f"{tid}.json")
    if os.path.isfile(p):
        d = json.load(open(p))
        rows.append({
            "task": tid,
            "speedup_vs_vision": d.get("speedup_vs_vision"),
            "token_ratio_vs_vision": d.get("token_ratio_vs_vision"),
            "claim_pass": d.get("claim_pass"),
            "autopilot_passed": (d.get("arms") or {}).get("autopilot", {}).get("passes"),
        })
json.dump({"tasks": rows, "n": len(rows)}, open(out, "w"), indent=2)
PY
  KILLER_JSON="$KILLER_INDEX"
elif [[ "$SKIP_KILLER" == "1" ]]; then
  echo "▶ killer-loop skipped (LIGH_HOLY_SKIP_KILLER=1)"
else
  echo "▶ killer-loop skipped (set OPENAI_API_KEY for multi-task A/B)"
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

python3 - "$OUT" "$MICRO_JSON" "$MATRIX_JSON" "$PILOT_JSON" "$KILLER_JSON" "$PHYSICAL_JSON" <<'PY'
import json, sys, time, platform, os, statistics

out, micro_path, matrix_path, pilot_path, killer_path, physical_raw = sys.argv[1:7]

def load(path):
    if path and os.path.isfile(path):
        try:
            return json.load(open(path))
        except Exception as e:
            return {"error": str(e), "path": path}
    return None

micro = load(micro_path) or {"error": "missing micro", "log": "/tmp/ligh-holy-micro.log"}
matrix = load(matrix_path)
pilot = load(pilot_path)
killer = load(killer_path) if killer_path else None

physical = None
if physical_raw.strip() not in ("", "null"):
    try:
        physical = json.loads(physical_raw)
    except Exception:
        physical = {"raw": physical_raw[:200]}

killer_speedups = [
    t.get("speedup_vs_vision") for t in (killer or {}).get("tasks", [])
    if isinstance(t.get("speedup_vs_vision"), (int, float))
]
killer_pass = sum(1 for t in (killer or {}).get("tasks", []) if t.get("claim_pass"))

doc = {
    "bench": "holy_shit",
    "version": 2,
    "claim": "Competitive product: motor + autopilot generalize across apps/tasks; agent loop beats vision when keyed",
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "platform": platform.platform(),
    "workflow_matrix": matrix,
    "autopilot_generality": pilot,
    "sim_micro": micro,
    "killer_loop_ab_v2_multi": killer,
    "physical_micro": physical,
    "claims": {
        "workflow_pass_rate": matrix.get("pass_rate") if matrix else None,
        "workflow_pass": matrix.get("workflows_pass") if matrix else None,
        "workflow_total": matrix.get("workflows_total") if matrix else None,
        "autopilot_apps_reached": pilot.get("apps_reached") if pilot else None,
        "autopilot_apps_total": pilot.get("apps_total") if pilot else None,
        "autopilot_generality_pass": pilot.get("claim_pass") if pilot else None,
        "flow_shapes_covered": pilot.get("flow_shapes_covered") if pilot else None,
        "sim_speedup_vs_wda": micro.get("comparisons", {}).get("ligh_vs_wda_speedup"),
        "killer_tasks_run": (killer or {}).get("n"),
        "killer_tasks_pass": killer_pass,
        "killer_median_speedup": round(statistics.median(killer_speedups), 2) if killer_speedups else None,
    },
    "product_verdict": {
        "motor_generalizes": bool(matrix and matrix.get("pass_rate", 0) >= 0.8),
        "autopilot_generalizes": bool(pilot and pilot.get("claim_pass")),
        "agent_beats_vision": killer_pass > 0 and (statistics.median(killer_speedups) >= 3.0 if killer_speedups else False),
    },
}
json.dump(doc, open(out, "w"), indent=2)
print("")
print("══ Holy-shit claims ══")
print(json.dumps(doc.get("claims"), indent=2))
print(json.dumps(doc.get("product_verdict"), indent=2))
print("wrote", out)
PY

echo "✓ holy-shit bench → $OUT"
