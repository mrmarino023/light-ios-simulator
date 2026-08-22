#!/usr/bin/env bash
# MCP closed-loop gate: agent uses ligh_cap_app_job fault → fix steps → verify.
# Simulates Cursor MCP path (call_tool) without an LLM — proves structured fault is actionable.
#
# Usage: ./scripts/gate-mcp-loop.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_MCP_LOOP_OUT:-$ROOT/docs/assets/mcp-loop-latest.json}"
APP="${LIGH_APP_PATH:-$ROOT/fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app}"
BUNDLE_ID="${LIGH_APP_BUNDLE_ID:-com.himali.XCUITestDemo}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
[[ -d "$APP" ]] || "$ROOT/scripts/build-xcuitestdemo.sh"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

echo "══ MCP closed-loop gate (XCUITestDemo) ══"
sim_clean_reboot "$LIGH"
"$ROOT/scripts/agent-first-loop.sh" >/tmp/mcp-loop-first.log 2>&1 || fail "first-loop failed"

python3 - "$OUT" "$APP" "$BUNDLE_ID" "$ROOT" <<'PY'
import json, os, sys, time
sys.path.insert(0, os.path.join(sys.argv[4], "scripts"))
from ligh_mcp import call_tool  # noqa: E402

out, app, bid, root = sys.argv[1:5]
t0 = time.time()

STEPS_BAD = [
    {"op": "wait", "id": "usernameTextField"},
    {"op": "tap", "id": "usernameTextField"},
    {"op": "type", "text": "alice"},
    {"op": "tap", "id": "passwordSecureField"},
    {"op": "type", "text": "secret"},
    {"op": "tap", "id": "loginButton"},
    {"op": "wait", "id": "NoSuchHomeTitle"},
]
STEPS_OK = [
    {"op": "wait", "id": "usernameTextField"},
    {"op": "tap", "id": "usernameTextField"},
    {"op": "type", "text": "alice"},
    {"op": "tap", "id": "passwordSecureField"},
    {"op": "type", "text": "secret"},
    {"op": "tap", "id": "loginButton"},
    {"op": "wait", "id": "homeTitle"},
]

def run_round(label, steps, no_install=False, launch_args=None):
    args = {
        "app": app,
        "bundle_id": bid,
        "steps": steps,
        "settle_ms": 3500,
        "timeout_ms": 15000,
    }
    if no_install:
        args["no_install"] = True
    if launch_args:
        args["launch_args"] = launch_args
    t = time.time()
    r = call_tool("ligh_cap_app_job", args)
    return {
        "round": label,
        "ms": int((time.time() - t) * 1000),
        "ok": bool(r.get("ok")),
        "fault": r.get("fault"),
        "detail": r.get("detail"),
        "compact": {"ok": r.get("ok"), "fault": r.get("fault"), "step": (r.get("detail") or {}).get("step")},
    }

rounds = []

print("  ▶ round 1: bad postcondition (agent typo in wait id)")
r1 = run_round("bad_postcondition", STEPS_BAD)
rounds.append(r1)
print("    fault=%s step=%s" % (r1["fault"], (r1.get("detail") or {}).get("step")))

print("  ▶ round 2: agent fixes steps → retry")
r2 = run_round("fixed_happy_path", STEPS_OK, no_install=True)
rounds.append(r2)
print("    ok=%s fault=%s %dms" % (r2["ok"], r2["fault"], r2["ms"]))

print("  ▶ round 3: invalid credentials via launch_args → explicit fail")
r3 = run_round(
    "invalid_credentials",
    [
        {"op": "wait", "id": "usernameTextField"},
        {"op": "tap", "id": "usernameTextField"},
        {"op": "type", "text": "bob"},
        {"op": "tap", "id": "passwordSecureField"},
        {"op": "type", "text": "wrong"},
        {"op": "tap", "id": "loginButton"},
        {"op": "wait", "id": "homeTitle"},
    ],
    no_install=True,
    launch_args=["--ui_test_login_failure"],
)
rounds.append(r3)
print("    fault=%s" % r3["fault"])

print("  ▶ round 4: agent removes failure flag + correct creds")
r4 = run_round("recovery_happy_path", STEPS_OK, no_install=True)
rounds.append(r4)
print("    ok=%s fault=%s %dms" % (r4["ok"], r4["fault"], r4["ms"]))

fault_actionable = (
    not r1["ok"]
    and r1["fault"] not in (None, "ok")
    and isinstance(r1.get("detail"), dict)
    and r1["detail"].get("step") is not None
)
retry_ok = r2["ok"] and r2["fault"] == "ok"
fail_closed = not r3["ok"] and r3["fault"] not in (None, "ok", "infra")
recovery_ok = r4["ok"] and r4["fault"] == "ok"
claim_pass = fault_actionable and retry_ok and fail_closed and recovery_ok

doc = {
    "gate": "mcp_loop",
    "claim": "Proof-of-mechanism: MCP ligh_cap_app_job returns actionable fault; harness scripts corrective retry (not autonomous agent).",
    "app": "XCUITestDemo (OSS third-party)",
    "bundle_id": bid,
    "app_path": app,
    "protocol": ["bad postcondition → fault+step", "fix steps → ok", "launch_args failure → fault", "fix → ok"],
    "rounds": rounds,
    "checks": {
        "fault_actionable": fault_actionable,
        "retry_ok": retry_ok,
        "fail_closed": fail_closed,
        "recovery_ok": recovery_ok,
    },
    "claim_pass": claim_pass,
    "total_ms": int((time.time() - t0) * 1000),
}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps({"claim_pass": claim_pass, "checks": doc["checks"], "out": out}, indent=2))
for k, v in doc["checks"].items():
    print("  %s %s" % ("PASS" if v else "FAIL", k))
raise SystemExit(0 if claim_pass else 1)
PY
STATUS=$?
echo "══ → $OUT"
exit "$STATUS"
