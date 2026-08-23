#!/usr/bin/env bash
# Workflow generalization gate: 5 apps × 5 workflows via LIGH app-job.
#
# Usage: ./scripts/gate-workflow-matrix.sh
# Output: docs/assets/workflow-matrix-latest.json
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
MANIFEST="$ROOT/fixtures/workflow-matrix/manifest.json"
OUT="${LIGH_WORKFLOW_OUT:-$ROOT/docs/assets/workflow-matrix-latest.json}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first (cargo build --release)"
[[ -f "$MANIFEST" ]] || fail "missing $MANIFEST"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

echo "══ workflow matrix: 5 apps × 5 workflows ══"
sim_clean_reboot "$LIGH" || fail "sim prep failed"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/workflow-first.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 3000 --recover-homes 4 >/tmp/workflow-ready.log 2>&1 \
  || fail "ligh_ready failed"

python3 - "$ROOT" "$LIGH" "$MANIFEST" "$OUT" <<'PY'
import json, os, subprocess, sys, time

root, ligh, manifest_path, out_path = sys.argv[1:5]
manifest = json.load(open(manifest_path))
results = []
built = {}

def app_path(app):
    env_key = app.get("app_path_env")
    if env_key and os.environ.get(env_key):
        return os.environ[env_key]
    name = app["id"]
    if name == "xcuitestdemo":
        return f"{root}/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
    if name == "lighfixture":
        return f"{root}/fixtures/LighFixture/build/LighFixture.app"
    # LighFeed, LighOnboard, LighModal
    cap = "".join(p.capitalize() for p in name.replace("ligh", "ligh ").split())
    if name.startswith("ligh"):
        cap = "Ligh" + name[4:].capitalize()
    return f"{root}/fixtures/{cap}/build/{cap}.app"

def build_app(app):
    aid = app["id"]
    if aid in built:
        return built[aid]
    cmd = app["build"]
    print(f"  ▶ build {aid}: {cmd}", flush=True)
    r = subprocess.run(cmd, shell=True, cwd=root, capture_output=True, text=True)
    if r.returncode != 0:
        built[aid] = None
        print(r.stderr[-500:] or r.stdout[-500:], flush=True)
        return None
    p = app_path(app)
    if not os.path.isdir(p):
        # parse last line from build script
        for line in reversed((r.stdout or "").splitlines()):
            if line.endswith(".app") and os.path.isdir(line):
                p = line
                break
    built[aid] = p if os.path.isdir(p) else None
    return built[aid]

def run_job(app_path, bundle_id, steps, no_install=False):
    job = json.dumps(steps)
    t0 = time.time()
    cmd = [
        ligh, "--json", "cap", "app-job", app_path, "--bundle-id", bundle_id,
         "--steps", job, "--settle-ms", "3500", "--timeout-ms", "22000",
    ]
    if no_install:
        cmd.append("--no-install")
    r = subprocess.run(cmd, cwd=root, capture_output=True, text=True)
    ms = int((time.time() - t0) * 1000)
    try:
        doc = json.loads(r.stdout)
    except Exception:
        doc = {"ok": False, "fault": "parse_error", "raw": (r.stdout or r.stderr)[-400:]}
    return doc, ms, r.returncode

def launch_app(app_path, bundle_id):
    subprocess.run(
        [ligh, "--json", "cap", "run-app", app_path, "--bundle-id", bundle_id,
         "--settle-ms", "3500", "--timeout-ms", "15000"],
        cwd=root, capture_output=True, text=True,
    )

for app in manifest["apps"]:
    ap = build_app(app)
    if not ap:
        for wf in app["workflows"]:
            results.append({
                "app": app["id"], "workflow": wf["id"], "ok": False,
                "fault": "build_failed", "skill": wf.get("skill"),
            })
        continue
    print(f"\n── {app['label']} ──", flush=True)
    launch_app(ap, app["bundle_id"])
    for i, wf in enumerate(app["workflows"]):
        doc, ms, rc = run_job(ap, app["bundle_id"], wf["steps"], no_install=(i > 0))
        ok = bool(doc.get("ok"))
        row = {
            "app": app["id"],
            "workflow": wf["id"],
            "skill": wf.get("skill"),
            "ok": ok,
            "fault": doc.get("fault"),
            "ms": ms,
            "detail": doc.get("detail"),
        }
        results.append(row)
        print(f"  {'PASS' if ok else 'FAIL'} {wf['id']} fault={doc.get('fault')} {ms}ms", flush=True)

passed = sum(1 for r in results if r.get("ok"))
by_app = {}
for r in results:
    by_app.setdefault(r["app"], {"pass": 0, "total": 0})
    by_app[r["app"]]["total"] += 1
    if r.get("ok"):
        by_app[r["app"]]["pass"] += 1

doc = {
    "gate": "workflow_matrix",
    "claim": manifest.get("claim"),
    "apps": len(manifest["apps"]),
    "workflows_per_app": 5,
    "workflows_total": len(results),
    "workflows_pass": passed,
    "workflows_fail": len(results) - passed,
    "pass_rate": round(passed / float(len(results)), 4) if results else 0,
    "by_app": by_app,
    "results": results,
    "interpretation": "Generalization across apps/workflows. Failures are signal — publish them.",
}
open(out_path, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({"pass": f"{passed}/{len(results)}", "out": out_path}, indent=2))
PY

echo "══ → $OUT"
