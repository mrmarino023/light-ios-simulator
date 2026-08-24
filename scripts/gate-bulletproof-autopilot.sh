#!/usr/bin/env bash
# Bulletproof-beta gate. It reports scope deficits honestly and runs the same
# append-only paired protocol used for published validation artifacts.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CMD="${1:-check}"
MATRIX="$ROOT/fixtures/validation-week/matrix.json"
REPEATS="${LIGH_BULLETPROOF_REPEATS:-20}"

check_matrix() {
  python3 - "$MATRIX" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
third = {a["id"] for a in m["apps"] if a.get("third_party")}
runnable = [t for t in m["tasks"] if t.get("status") == "runnable"]
priority = [t for t in runnable if t.get("repeat_priority")]
errors = []
if len(third) < 3:
    errors.append(f"third-party apps: {len(third)}/3")
if len(runnable) < 15:
    errors.append(f"runnable heterogeneous tasks: {len(runnable)}/15")
if len(priority) < 3:
    errors.append(f"priority repeated tasks: {len(priority)}/3")
print(json.dumps({
    "third_party_apps": len(third),
    "runnable_tasks": len(runnable),
    "priority_tasks": len(priority),
    "ready": not errors,
    "deficits": errors,
}, indent=2))
if errors:
    raise SystemExit(1)
PY
}

case "$CMD" in
  check)
    cargo test --manifest-path "$ROOT/Cargo.toml" -p ligh-core --lib
    cargo test --manifest-path "$ROOT/Cargo.toml" -p ligh-daemon
    check_matrix
    ;;
  faults)
    cargo test --manifest-path "$ROOT/Cargo.toml" -p ligh-daemon fault_injection
    cargo test --manifest-path "$ROOT/Cargo.toml" -p ligh-core target_epoch
    cargo test --manifest-path "$ROOT/Cargo.toml" -p ligh-core failed_type
    ;;
  repeat)
    check_matrix
    export LIGH_VW_REPEAT="$REPEATS"
    export LIGH_VW_TASKS="${LIGH_VW_TASKS:-login-never-navigates kix-login-never-authenticates onboarding-home-broken}"
    export LIGH_VW_ARMS="${LIGH_VW_ARMS:-autopilot baseline}"
    exec "$ROOT/scripts/validation-week.sh" paired
    ;;
  *)
    echo "usage: $0 [check|faults|repeat]" >&2
    exit 2
    ;;
esac
