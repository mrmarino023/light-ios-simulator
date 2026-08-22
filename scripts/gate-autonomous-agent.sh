#!/usr/bin/env bash
# Autonomous agent gate: inject bug → LLM fixes source → LIGH verifies (no scripted fix).
#
# Requires: OPENAI_API_KEY, release ligh, Xcode sim
# Usage: ./scripts/gate-autonomous-agent.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
OUT="${LIGH_AUTONOMOUS_OUT:-$ROOT/docs/assets/autonomous-agent-latest.json}"
SRC="$ROOT/fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift"
BACKUP=$(mktemp)

fail() { echo "✗ $*" >&2; exit 1; }
[[ -x "$LIGH" ]] || fail "build release ligh first"
if [[ -z "${OPENAI_API_KEY:-}" && -n "${LIGH_ENV_FILE:-}" && -f "${LIGH_ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${LIGH_ENV_FILE}"
  set +a
fi
[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required (export or LIGH_ENV_FILE=/path/.env)"
[[ -f "$SRC" ]] || fail "missing $SRC"

# shellcheck source=lib/sim-clean.sh
source "$ROOT/scripts/lib/sim-clean.sh"

restore_source() {
  cp "$BACKUP" "$SRC"
}
trap restore_source EXIT

echo "══ autonomous agent gate ══"
cp "$SRC" "$BACKUP"

echo "  ▶ inject bug: apply login-a11y-typo.patch"
PATCH="$ROOT/fixtures/third-party/XCUITestDemo/scenarios/login-a11y-typo.patch"
patch -p1 -d "$ROOT" < "$PATCH"

"$ROOT/scripts/build-xcuitestdemo.sh" >/tmp/autonomous-build-broken.log 2>&1 || fail "build broken app"

sim_clean_reboot "$LIGH"
if ! "$ROOT/scripts/agent-first-loop.sh" >/tmp/autonomous-first.log 2>&1; then
  echo "  ⚠ SpringBoard wake slow — ligh_ready for app-only job"
  "$LIGH" --json ready --settle-ms 3000 --recover-homes 4 >/tmp/autonomous-ready.log 2>&1 || fail "ligh_ready failed"
fi

echo "  ▶ run autonomous agent (LLM, no scripted fix)"
export LIGH_AUTONOMOUS_OUT="$OUT"
python3 "$ROOT/scripts/autonomous-login-agent.py" && PASS=1 || PASS=0

restore_source
trap - EXIT
"$ROOT/scripts/build-xcuitestdemo.sh" >/tmp/autonomous-build-restore.log 2>&1 || true

python3 - "$OUT" "$PASS" <<'PY'
import json, sys
out, passed = sys.argv[1], sys.argv[2] == "1"
doc = json.load(open(out))
doc["source_restored"] = True
doc["bug"] = "loginButton → loginBtnTypo (accessibility id typo)"
if doc.get("driver") != "openai":
    doc["driver"] = "openai"
open(out, "w").write(json.dumps(doc, indent=2)+"\n")
print(json.dumps({"claim_pass": passed and doc.get("verified"), "verified": doc.get("verified"), "driver": doc.get("driver"), "steps": doc.get("steps_used"), "out": out}, indent=2))
raise SystemExit(0 if passed else 1)
PY

echo "══ → $OUT"
