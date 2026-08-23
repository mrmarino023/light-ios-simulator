#!/usr/bin/env bash
# Autonomous agent matrix: 5 hidden bug scenarios × N LLM runs each.
# Publishes passes AND failures — no cherry-picked pass bar.
#
# Agent sees only: source files, build, ligh_ready, ligh_app_job (via ligh_mcp).
# Scenario id is harness metadata only — never passed to the LLM.
#
# Usage:
#   LIGH_ENV_FILE=/path/.env LIGH_MATRIX_N=5 ./scripts/gate-autonomous-matrix.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_MATRIX_N:-5}"
OUT="${LIGH_MATRIX_OUT:-$ROOT/docs/assets/autonomous-matrix-latest.json}"
TRACES_DIR="${LIGH_MATRIX_TRACES:-$ROOT/docs/assets/autonomous-matrix-traces}"
MANIFEST="$ROOT/fixtures/third-party/XCUITestDemo/scenarios/manifest.json"
GOAL="${LIGH_AUTONOMOUS_GOAL:-The login flow is broken. Find out why and fix it.}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
[[ -f "$MANIFEST" ]] || fail "missing $MANIFEST"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"
# shellcheck source=lib/autonomous-scenario.sh
source "$ROOT/scripts/lib/autonomous-scenario.sh"

autonomous_load_env
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required (export or LIGH_ENV_FILE=)"

autonomous_backup_sources
trap autonomous_restore_sources EXIT

mkdir -p "$TRACES_DIR"
echo "══ autonomous matrix: 5 scenarios × N=$N ══"
echo "  goal: $GOAL"
echo "  traces: $TRACES_DIR"
autonomous_prepare_sim || fail "sim prep failed"

RUNS_FILE=$(mktemp)
trap 'autonomous_restore_sources; rm -f "$RUNS_FILE"' EXIT

python3 - "$MANIFEST" "$N" "$GOAL" "$ROOT" "$RUNS_FILE" "$TRACES_DIR" <<'PY'
import json, os, shutil, subprocess, sys, time
manifest, n, goal, root, runs_path, traces_dir = sys.argv[1:7]
n = int(n)
scenarios = json.load(open(manifest))["scenarios"]
env = os.environ.copy()
env["LIGH_AUTONOMOUS_GOAL"] = goal
# Do NOT pass scenario id or patch hints to the agent process.
env.pop("LIGH_AUTONOMOUS_SCENARIO", None)
ligh_env = os.environ.get("LIGH_ENV_FILE")
if ligh_env:
    env["LIGH_ENV_FILE"] = ligh_env

for sc in scenarios:
    sid = sc["id"]
    patch = sc["patch"]
    for run in range(1, n + 1):
        print(f"\n── {sid} run {run}/{n} ──", flush=True)
        r = subprocess.run(
            ["bash", "-lc", f'''
              set -euo pipefail
              export ROOT="{root}"
              source "{root}/scripts/lib/autonomous-scenario.sh"
              autonomous_apply_scenario "{patch}"
              "{root}/scripts/build-xcuitestdemo.sh" >/tmp/matrix-build.log 2>&1
            '''],
            cwd=root,
            env=env,
        )
        if r.returncode != 0:
            row = {
                "scenario": sid,
                "run": run,
                "verified": False,
                "failure_mode": "patch_or_build_failed",
                "error": "patch_or_build_failed",
            }
            open(runs_path, "a").write(json.dumps(row) + "\n")
            print("  FAIL patch/build", flush=True)
            continue
        out = f"/tmp/autonomous-matrix-{sid}-{run}.json"
        trace_out = os.path.join(traces_dir, f"{sid}-run{run}.json")
        env_run = {**env, "LIGH_AUTONOMOUS_OUT": out}
        t0 = time.time()
        p = subprocess.run(
            [sys.executable, f"{root}/scripts/autonomous-login-agent.py"],
            cwd=root,
            env=env_run,
        )
        ms = int((time.time() - t0) * 1000)
        try:
            doc = json.load(open(out))
            shutil.copy2(out, trace_out)
        except Exception as e:
            doc = {"verified": False, "failure_mode": "harness_error", "error": str(e)}
        row = {
            "scenario": sid,
            "run": run,
            "verified": bool(doc.get("verified")),
            "failure_mode": doc.get("failure_mode"),
            "steps_used": doc.get("steps_used"),
            "tokens": doc.get("tokens"),
            "ms": ms,
            "agent_exit": p.returncode,
            "trace": trace_out,
            "build_recoveries": sum(
                1 for t in (doc.get("trace") or [])
                if (t.get("action") or {}).get("action") == "build_app"
                and not (t.get("result") or {}).get("ok")
            ),
        }
        open(runs_path, "a").write(json.dumps(row) + "\n")
        tag = row["failure_mode"] or "ok"
        print(
            f"  {'PASS' if row['verified'] else 'FAIL'} "
            f"mode={tag} steps={row.get('steps_used')} ms={ms}",
            flush=True,
        )
PY

python3 - "$OUT" "$N" "$GOAL" "$MANIFEST" "$RUNS_FILE" "$TRACES_DIR" <<'PY'
import json, sys, statistics
out, n, goal, manifest, runs_path, traces_dir = sys.argv[1:7]
n = int(n)
scenarios = json.load(open(manifest))["scenarios"]
rows = [json.loads(l) for l in open(runs_path) if l.strip()]
by_sc = {}
for r in rows:
    by_sc.setdefault(r["scenario"], []).append(r)

failure_taxonomy = {}
for r in rows:
    if r.get("verified"):
        continue
    mode = r.get("failure_mode") or "unknown"
    failure_taxonomy[mode] = failure_taxonomy.get(mode, 0) + 1

scenario_stats = []
for sc in scenarios:
    sid = sc["id"]
    rs = by_sc.get(sid, [])
    passed = [x for x in rs if x.get("verified")]
    fails = [x for x in rs if not x.get("verified")]
    scenario_stats.append({
        "id": sid,
        "patch": sc["patch"],
        "hint_for_humans": sc.get("hint_for_humans"),
        "runs": len(rs),
        "pass": len(passed),
        "pass_rate": round(len(passed) / float(len(rs)), 4) if rs else 0,
        "solved_at_least_once": len(passed) > 0,
        "failure_modes": {
            m: sum(1 for x in fails if (x.get("failure_mode") or "unknown") == m)
            for m in sorted({(x.get("failure_mode") or "unknown") for x in fails})
        },
    })

verified = sum(1 for r in rows if r.get("verified"))
solved = sum(1 for s in scenario_stats if s["solved_at_least_once"])

doc = {
    "gate": "autonomous_matrix",
    "claim": "5 hidden bug scenarios × N LLM runs — vague prompt, failures published",
    "goal": goal,
    "agent_tools": ["read_file", "write_file", "build_app", "ligh_ready", "ligh_app_job", "done"],
    "contamination_check": "scenario id and patch hints are harness-only; not in LLM prompt",
    "n_per_scenario": n,
    "scenarios_total": len(scenarios),
    "scenarios_solved_at_least_once": solved,
    "runs_total": len(rows),
    "runs_pass": verified,
    "runs_fail": len(rows) - verified,
    "pass_rate": round(verified / float(len(rows)), 4) if rows else 0,
    "failure_taxonomy": failure_taxonomy,
    "scenario_stats": scenario_stats,
    "runs": rows,
    "traces_dir": traces_dir,
    "interpretation": "Pass rate shows where the primitive breaks; not a marketing score.",
}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({
    "runs_pass": f"{verified}/{len(rows)}",
    "scenarios_solved_at_least_once": f"{solved}/{len(scenarios)}",
    "failure_taxonomy": failure_taxonomy,
    "out": out,
}, indent=2))
PY

echo "══ → $OUT"
