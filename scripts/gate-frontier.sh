#!/usr/bin/env bash
# Frontier bakeoff — same hard goals × LIGH(llm) × vision(PNG) × WDA(llm).
# No host shortcuts. Writes docs/assets/frontier-gate-latest.json
#
# Usage:
#   OPENAI_API_KEY=… ./scripts/gate-frontier.sh
#   LIGH_FRONTIER_N=3 OPENAI_MODEL=gpt-5-mini ./scripts/gate-frontier.sh
#   LIGH_FRONTIER_SKIP_VISION=1 LIGH_FRONTIER_SKIP_WDA=1  # faster local
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
N="${LIGH_FRONTIER_N:-5}"
MODEL="${OPENAI_MODEL:-gpt-5-mini}"
OUT="$ROOT/docs/assets/frontier-gate-latest.json"
SKIP_VISION="${LIGH_FRONTIER_SKIP_VISION:-0}"
SKIP_WDA="${LIGH_FRONTIER_SKIP_WDA:-0}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
"$LIGH" daemon status >/dev/null 2>&1 || fail "lighd not running — ligh daemon start && ligh up"
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required"

mkdir -p "$ROOT/docs/assets"

GOALS=(
  "Open Settings (Impostazioni or Settings), use search Cerca/Search, find Bluetooth, leave the Bluetooth row or screen visible, then done"
  "Open Safari, make sure the browser chrome or address field is visible, then done"
  "Open Settings, open Generali or General, then navigate back so Impostazioni/Settings root list is visible again, then done"
  "Open Maps (Mappe or Maps), ensure map chrome or search field is visible, then done"
)

echo "══ Frontier gate N=$N model=$MODEL policy=llm ══"

# Wake eyes
for _ in 1 2 3 4 5; do "$LIGH" home >/dev/null 2>&1 || true; sleep 0.35; done

export PYTHONUNBUFFERED=1
python3 -u - "$ROOT" "$LIGH" "$N" "$MODEL" "$OUT" "$SKIP_VISION" "$SKIP_WDA" "${GOALS[@]}" <<'PY'
import json, os, subprocess, sys, time, urllib.request

root, ligh, n_s, model, out_path, skip_v, skip_w = sys.argv[1:8]
n = int(n_s)
goals = sys.argv[8:]
skip_vision = skip_v == "1"
skip_wda = skip_w == "1"

def run(cmd, timeout=300):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"")
        err = (e.stderr or b"")
        if isinstance(out, bytes):
            out = out.decode(errors="replace")
        if isinstance(err, bytes):
            err = err.decode(errors="replace")
        return 124, out, (err or "") + f"\ntimeout after {timeout}s"

def home():
    subprocess.run([ligh, "home"], capture_output=True, timeout=30)
    time.sleep(0.45)
    subprocess.run([ligh, "home"], capture_output=True, timeout=30)
    time.sleep(0.55)

def arm_ligh(goal):
    t0 = time.time()
    # Control-plane arm (product) — not LLM rediscovery
    code, out, err = run(
        [sys.executable, os.path.join(root, "scripts", "agent-cap-loop.py"), goal],
        timeout=300,
    )
    infra = code == 2
    return {
        "ok": code == 0,
        "seconds": time.time() - t0,
        "infra_skip": infra,
        "driver": "control_plane_capabilities",
        "log_tail": (out or err)[-400:],
    }

def arm_vision(goal):
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "vision_compare", os.path.join(root, "scripts", "vision-compare.py")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    t0 = time.time()
    try:
        r = mod.arm_vision(goal, model, steps=10)
        r.setdefault("seconds", time.time() - t0)
        return r
    except Exception as e:
        return {"ok": False, "seconds": time.time() - t0, "error": str(e)}

def arm_wda(goal):
    t0 = time.time()
    code, out, err = run(
        [sys.executable, os.path.join(root, "scripts", "agent-wda-llm-loop.py"),
         "--model", model, "--steps", "12", "--goal", goal],
        timeout=300,
    )
    return {
        "ok": code == 0,
        "seconds": time.time() - t0,
        "skipped": code == 2,
        "timeout": code == 124,
        "log_tail": (out or err)[-300:],
    }

wda_available = False
if not skip_wda:
    try:
        urllib.request.urlopen(os.environ.get("APPIUM_URL", "http://127.0.0.1:4723").rstrip("/") + "/status", timeout=3)
        wda_available = True
        print("  Appium: up")
    except Exception as e:
        wda_available = False
        print(f"  (WDA arm skipped — Appium not reachable: {e})")

results = {
    "ts": time.time(),
    "model": model,
    "policy": "llm",
    "n_per_goal": n,
    "claim": "frontier_bakeoff_control_plane_vs_vision_vs_wda",
    "ligh_driver": "control_plane_capabilities",
    "goals": [],
    "arms": {"ligh": True, "vision": not skip_vision, "wda": wda_available and not skip_wda},
}

def finalize():
    def rate(arm):
        p = t = 0
        secs = []
        for g in results["goals"]:
            for r in g.get(arm) or []:
                if r.get("skipped") or r.get("infra_skip"):
                    continue
                t += 1
                if r.get("ok"):
                    p += 1
                if "seconds" in r:
                    secs.append(float(r["seconds"]))
        return p, t, (sum(secs) / len(secs) if secs else None)

    summary = {}
    for arm in ("ligh", "vision", "wda"):
        p, t, avg = rate(arm)
        summary[arm] = {"pass": p, "total": t, "pass_rate": (p / t if t else None), "avg_seconds": avg}

    results["summary"] = summary
    wins = 0
    compared = 0
    for g in results["goals"]:
        def pr(arm):
            rows = [r for r in (g.get(arm) or []) if not r.get("skipped") and not r.get("infra_skip")]
            if not rows:
                return None
            return sum(1 for r in rows if r.get("ok")) / len(rows)
        lp, vp, wp = pr("ligh"), pr("vision"), pr("wda")
        others = [x for x in (vp, wp) if x is not None]
        if not others:
            continue
        compared += 1
        if lp is not None and lp >= max(others):
            wins += 1
    results["ligh_wins_goals"] = wins
    results["goals_compared"] = compared
    results["frontier_bar"] = "LIGH pass_rate >= others on >=2/3 compared goals"
    need = max(2, (2 * compared + 2) // 3) if compared else 2
    results["frontier_pass"] = compared > 0 and wins >= need
    open(out_path, "w").write(json.dumps(results, indent=2) + "\n")
    print(json.dumps(summary, indent=2), flush=True)
    print(f"ligh_wins_goals={wins}/{compared} frontier_pass={results['frontier_pass']}", flush=True)
    print(f"wrote {out_path}", flush=True)

try:
    for goal in goals:
        entry = {"goal": goal, "ligh": [], "vision": [], "wda": []}
        short = goal[:52]
        for i in range(1, n + 1):
            home()
            # Control-plane: refuse to score dead eyes as model failure
            ready = subprocess.run(
                [ligh, "--json", "ready", "--settle-ms", "2500", "--recover-homes", "6"],
                capture_output=True, text=True, timeout=90,
            )
            try:
                ready_j = json.loads(ready.stdout or "{}")
            except Exception:
                ready_j = {}
            if not ready_j.get("ok"):
                fault = (ready_j.get("fault") or "infra")
                skip = {"ok": False, "infra_skip": True, "fault": fault, "seconds": 0}
                entry["ligh"].append(dict(skip))
                print(f"  ligh   infra_skip/{fault} {i}/{n} — {short}…", flush=True)
                if not skip_vision:
                    entry["vision"].append(dict(skip))
                    print(f"  vision infra_skip/{fault} {i}/{n} — {short}…", flush=True)
                if wda_available and not skip_wda:
                    entry["wda"].append(dict(skip))
                    print(f"  wda    infra_skip/{fault} {i}/{n} — {short}…", flush=True)
                continue

            r = arm_ligh(goal)
            entry["ligh"].append(r)
            tag = "infra_skip" if r.get("infra_skip") else ("ok" if r["ok"] else "FAIL")
            print(f"  ligh   {tag} {i}/{n} — {short}…", flush=True)

            if not skip_vision:
                home()
                r = arm_vision(goal)
                entry["vision"].append(r)
                print(f"  vision {'ok' if r.get('ok') else 'FAIL'} {i}/{n} — {short}…", flush=True)

            if wda_available and not skip_wda:
                home()
                r = arm_wda(goal)
                entry["wda"].append(r)
                tag = "skip" if r.get("skipped") else ("ok" if r["ok"] else "FAIL")
                print(f"  wda    {tag} {i}/{n} — {short}…", flush=True)
        results["goals"].append(entry)
        finalize()  # checkpoint after each goal
except Exception as e:
    results["error"] = str(e)
    print(f"✗ gate error: {e}", flush=True)
    finalize()
    raise
else:
    finalize()
PY

echo "wrote $OUT"
# Diagnostic — do not fail CI on lose; frontier_pass is in JSON
exit 0
