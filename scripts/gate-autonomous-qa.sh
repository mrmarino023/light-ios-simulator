#!/usr/bin/env bash
# Autonomous QA-layer agent gate — Mac + OPENAI_API_KEY + booted sim required.
# Honest: fails if sim/API missing. Does not fake pass.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export LIGH_WORKSPACE="${LIGH_WORKSPACE:-$ROOT}"
OUT="${LIGH_GATE_OUT:-$ROOT/docs/assets}/autonomous-agent-qa-latest.json"

fail() { echo "✗ $*" >&2; exit 1; }
ok() { echo "✓ $*"; }

[[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required (real LLM run)"
[[ -x "${LIGH_BIN:-$ROOT/target/release/ligh}" ]] || fail "build release ligh first"

LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
"$LIGH" daemon status >/dev/null 2>&1 || fail "lighd not running — ligh daemon start && ligh up"

echo "══ Autonomous QA agent gate ══"
echo "  workspace=$LIGH_WORKSPACE"
echo "  inject bug patch if LIGH_INJECT_BUG=1"

if [[ "${LIGH_INJECT_BUG:-0}" == "1" ]]; then
  PATCH="$ROOT/fixtures/third-party/XCUITestDemo/scenarios/login-a11y-typo.patch"
  git -C "$ROOT" checkout -- fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift 2>/dev/null || true
  patch -p1 -d "$ROOT" < "$PATCH"
  ok "injected login-a11y-typo"
fi

PASS=0
if python3 "$ROOT/scripts/autonomous-login-agent-qa.py"; then
  PASS=1
  ok "autonomous QA agent verified login"
else
  echo "  FAIL — see autonomous-agent-qa-latest.json"
fi

if [[ "${LIGH_INJECT_BUG:-0}" == "1" ]]; then
  git -C "$ROOT" checkout -- fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift 2>/dev/null || true
fi

[[ -f "$OUT" ]] || OUT="$ROOT/docs/assets/autonomous-agent-qa-latest.json"
python3 - <<PY
import json, os
p = "$OUT"
doc = json.load(open(p)) if os.path.isfile(p) else {"verified": False}
doc["gate_env"] = "mac_integration"
doc["claim_pass"] = bool(int("$PASS"))
open(p, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps(doc, indent=2))
PY

[[ "$PASS" == "1" ]] || exit 1
ok "wrote $OUT"
