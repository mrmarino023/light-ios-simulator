#!/usr/bin/env bash
# External apps gate — apps we do NOT modify. This is the generalization experiment.
#
# Usage: ./scripts/gate-external-apps.sh
# Output: docs/assets/external-apps-latest.json
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
MANIFEST="$ROOT/fixtures/external-apps/manifest.json"
OUT="${LIGH_EXTERNAL_OUT:-$ROOT/docs/assets/external-apps-latest.json}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first (cargo build --release)"
[[ -f "$MANIFEST" ]] || fail "missing $MANIFEST"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

echo "══ external apps (frozen — no source edits) ══"
sim_clean_reboot "$LIGH" || fail "sim prep failed"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/external-first.log 2>&1 \
  || "$LIGH" --json ready --settle-ms 3000 --recover-homes 4 >/tmp/external-ready.log 2>&1 \
  || fail "ligh_ready failed"

python3 - "$ROOT" "$LIGH" "$MANIFEST" "$OUT" <<'PY'
import json, os, subprocess, sys, time

root, ligh, manifest_path, out_path = sys.argv[1:5]
manifest = json.load(open(manifest_path))
results = []

def app_path(app):
    env_key = app.get("app_path_env")
    if env_key and os.environ.get(env_key):
        return os.environ[env_key]
    aid = app["id"]
    if aid == "xcuitestdemo":
        return f"{root}/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"
    return app.get("app_path")

def build_app(app):
    cmd = app.get("build")
    if not cmd:
        return app_path(app)
    print(f"  ▶ build {app['id']}: {cmd}", flush=True)
    r = subprocess.run(cmd, shell=True, cwd=root, capture_output=True, text=True)
    if r.returncode != 0:
        print((r.stderr or r.stdout)[-400:], flush=True)
        return None
    p = app_path(app)
    return p if p and os.path.isdir(p) else None

def launch_app(ap, bundle_id):
    subprocess.run(
        [ligh, "--json", "cap", "run-app", ap, "--bundle-id", bundle_id,
         "--settle-ms", "3500", "--timeout-ms", "15000"],
        cwd=root, capture_output=True, text=True,
    )

def run_job(ap, bundle_id, steps, no_install=False):
    job = json.dumps(steps)
    cmd = [
        ligh, "--json", "cap", "app-job", ap, "--bundle-id", bundle_id,
        "--steps", job, "--settle-ms", "3500", "--timeout-ms", "22000",
    ]
    if no_install:
        cmd.append("--no-install")
    t0 = time.time()
    r = subprocess.run(cmd, cwd=root, capture_output=True, text=True)
    ms = int((time.time() - t0) * 1000)
    try:
        doc = json.loads(r.stdout)
    except Exception:
        doc = {"ok": False, "fault": "parse_error", "raw": (r.stdout or r.stderr)[-400:]}
    return doc, ms

for app in manifest["apps"]:
    ap = build_app(app)
    if not ap:
        for wf in app["workflows"]:
            results.append({
                "app": app["id"], "workflow": wf["id"], "ok": False,
                "fault": "build_failed", "frozen": app.get("frozen", False),
            })
        continue
    print(f"\n── {app['label']} (frozen={app.get('frozen')}) ──", flush=True)
    launch_app(ap, app["bundle_id"])
    for i, wf in enumerate(app["workflows"]):
        doc, ms = run_job(ap, app["bundle_id"], wf["steps"], no_install=(i > 0))
        ok = bool(doc.get("ok"))
        row = {
            "app": app["id"],
            "workflow": wf["id"],
            "skill": wf.get("skill"),
            "ok": ok,
            "fault": doc.get("fault"),
            "ms": ms,
            "detail": doc.get("detail"),
            "frozen": app.get("frozen", False),
        }
        results.append(row)
        print(f"  {'PASS' if ok else 'FAIL'} {wf['id']} fault={doc.get('fault')} {ms}ms", flush=True)

passed = sum(1 for r in results if r.get("ok"))
doc = {
    "gate": manifest.get("gate"),
    "claim": manifest.get("claim"),
    "rules": manifest.get("rules"),
    "apps": len(manifest["apps"]),
    "workflows_total": len(results),
    "workflows_pass": passed,
    "workflows_fail": len(results) - passed,
    "pass_rate": round(passed / float(len(results)), 4) if results else 0,
    "interpretation": "Generalization signal only. Do not tune app source to improve this gate.",
    "results": results,
}
open(out_path, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({"pass": f"{passed}/{len(results)}", "out": out_path}, indent=2))
PY

echo "══ → $OUT"
