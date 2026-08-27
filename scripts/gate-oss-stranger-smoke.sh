#!/usr/bin/env bash
# OSS stranger smoke — label-first discover + ligh_test on public iOS apps (0 AX ids).
#
# Usage:
#   ./scripts/gate-oss-stranger-smoke.sh              # countries + foodtruck
#   ./scripts/gate-oss-stranger-smoke.sh countries
#   ./scripts/gate-oss-stranger-smoke.sh foodtruck
#   LIGH_OSS_OUT=/tmp/oss.json ./scripts/gate-oss-stranger-smoke.sh
#
# Env:
#   LIGH_BIN, LIGH_DEVICE (default iphone-15-pro), LIGH_OSS_WORK (scratch dir)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIGH="${LIGH_BIN:-$ROOT/target/release/ligh}"
LIGHD="${LIGH%d/ligh}/lighd"
DEVICE="${LIGH_DEVICE:-iphone-15-pro}"
WORK="${LIGH_OSS_WORK:-${RUNNER_TEMP:-/tmp}/ligh-oss-smoke}"
OUT="${LIGH_OSS_OUT:-$ROOT/docs/assets/oss-stranger-smoke-latest.json}"
SELECT="${1:-all}"

fail() { echo "✗ $*" >&2; exit 1; }
[[ "$(uname -s)" == "Darwin" ]] || fail "macOS + Xcode required"
[[ -x "$LIGH" ]] || fail "missing $LIGH — cargo build --release -p ligh-cli -p ligh-daemon"
[[ -x "$LIGHD" ]] || fail "missing $LIGHD"

export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=core.hooksPath
export GIT_CONFIG_VALUE_0=/dev/null
export PYTHONPATH="$ROOT/scripts"
export LIGH_BIN="$LIGH"

mkdir -p "$WORK"
RESULTS_JSONL="$WORK/results.jsonl"
: >"$RESULTS_JSONL"

ensure_daemon() {
  "$LIGH" daemon stop >/dev/null 2>&1 || true
  pkill -x lighd 2>/dev/null || true
  sleep 0.5
  nohup "$LIGHD" >>"$WORK/lighd.log" 2>&1 &
  sleep 1.2
  "$LIGH" up --device "$DEVICE" >/dev/null 2>&1 \
    || "$LIGH" up --device "$DEVICE" \
    || fail "ligh up failed — see $WORK/lighd.log"
}

# clone_url name scheme project_rel source_rel bundle_hint
smoke_one() {
  local url="$1" name="$2" scheme="$3" project_rel="$4" source_rel="$5" bundle_hint="$6"
  local dir="$WORK/$name"
  local log="$WORK/$name-build.log"
  echo "══ OSS smoke: $name ══"

  if [[ ! -d "$dir/.git" ]]; then
    echo "▶ clone $url"
    git clone --depth 1 "$url" "$dir" >/dev/null
  else
    echo "▶ reuse clone $dir"
  fi

  local xcode="$dir/$project_rel"
  [[ -d "$xcode" ]] || fail "missing project $xcode"

  local derived="$dir/build/ligh/DerivedData"
  echo "▶ xcodebuild -scheme $scheme"
  (
    cd "$dir"
    xcodebuild \
      -project "$project_rel" \
      -scheme "$scheme" \
      -configuration Debug \
      -sdk iphonesimulator \
      -derivedDataPath "$derived" \
      -destination 'generic/platform=iOS Simulator' \
      ONLY_ACTIVE_ARCH=YES ARCHS=arm64 \
      CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= \
      build
  ) >"$log" 2>&1 || {
    tail -40 "$log" >&2
    fail "xcodebuild failed for $name — $log"
  }

  local app_src
  app_src=$(find "$derived/Build/Products" -type d -name '*.app' | head -1)
  [[ -n "$app_src" && -d "$app_src" ]] || fail "no .app for $name"
  local app_dest="$dir/build/ligh/$(basename "$app_src")"
  rm -rf "$app_dest"
  mkdir -p "$(dirname "$app_dest")"
  cp -R "$app_src" "$app_dest"

  local bid
  bid=$(python3 -c "import plistlib; print(plistlib.load(open('$app_dest/Info.plist','rb')).get('CFBundleIdentifier') or '')")
  [[ -n "$bid" ]] || bid="$bundle_hint"
  local src="$dir/$source_rel"
  [[ -d "$src" ]] || src="$dir"

  local ws="$dir"
  mkdir -p "$ws/.ligh"
  python3 - <<PY
import json, sys
sys.path.insert(0, "$ROOT/scripts")
from ligh_project import write_agent_bundle
from ligh_audit_accessibility import (
    audit_source_root, suggest_app_job_steps, suggest_app_goal, suggest_verification,
)
audit = audit_source_root("$src")
steps = suggest_app_job_steps(audit)
goal = suggest_app_goal(audit, steps)
doc = {
    "schema": 1,
    "kind": "app",
    "app_path": "$app_dest",
    "bundle_id": "$bid",
    "source_root": "$src",
    "workspace": "$ws",
    "audit": audit,
    "suggested_app_job": steps,
    "suggested_app_goal": goal,
    "suggested_verification": suggest_verification(audit, steps),
}
write_agent_bundle(doc, "$ws/.ligh")
print("audit", audit.get("readiness_grade"), "ids", audit.get("identity_count"),
      "hints", (audit.get("label_hints") or [])[:5])
PY

  echo "▶ live discover"
  local disc="$WORK/$name-discover.json"
  python3 "$ROOT/scripts/ligh_discover.py" \
    --app "$app_dest" --bundle-id "$bid" --source-root "$src" \
    --write "$ws/.ligh" --project-json "$ws/.ligh/project.json" \
    >"$disc" 2>"$WORK/$name-discover.err" || true
  python3 -c "import json; d=json.load(open('$disc')); print('discover', d.get('proven_chrome'), d.get('agent_ready'), d.get('bootstrap_ok'))"

  echo "▶ ligh-test"
  local test_out="$WORK/$name-test.json"
  LIGH_WORKSPACE="$ws" LIGH_TEST_OUT="$test_out" "$ROOT/scripts/ligh-test.sh" \
    >"$WORK/$name-test.stdout" 2>"$WORK/$name-test.stderr" || true
  python3 - <<PY
import json, time
d = json.load(open("$test_out"))
disc = json.load(open("$disc"))
row = {
    "name": "$name",
    "repo": "$url",
    "bundle_id": "$bid",
    "app_path": "$app_dest",
    "proven_chrome": disc.get("proven_chrome") or disc.get("wait_hint"),
    "discover_ready": bool(disc.get("agent_ready")),
    "bootstrap_ok": bool(disc.get("bootstrap_ok")),
    "ligh_test": {"ok": bool(d.get("ok")), "fault": d.get("fault"), "mode": d.get("mode")},
    "ts": int(time.time()),
}
open("$RESULTS_JSONL", "a").write(json.dumps(row) + "\n")
print("result", row["name"], "ok=", row["ligh_test"]["ok"], "fault=", row["ligh_test"]["fault"],
      "chrome=", row["proven_chrome"])
raise SystemExit(0 if row["ligh_test"]["ok"] else 1)
PY
}

ensure_daemon

PASS=0
FAIL=0
run_countries=0
run_foodtruck=0
case "$SELECT" in
  all) run_countries=1; run_foodtruck=1 ;;
  countries|countrieswiftui) run_countries=1 ;;
  foodtruck|food-truck|food_truck) run_foodtruck=1 ;;
  *) fail "unknown select: $SELECT (use all|countries|foodtruck)" ;;
esac

set +e
if [[ "$run_countries" -eq 1 ]]; then
  if smoke_one \
    "https://github.com/nalexn/clean-architecture-swiftui.git" \
    "CountriesSwiftUI" \
    "CountriesSwiftUI" \
    "CountriesSwiftUI.xcodeproj" \
    "CountriesSwiftUI" \
    "com.swiftui.CountriesSwiftUI"
  then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
fi
if [[ "$run_foodtruck" -eq 1 ]]; then
  if smoke_one \
    "https://github.com/apple/sample-food-truck.git" \
    "FoodTruck" \
    "Food Truck" \
    "Food Truck.xcodeproj" \
    "App" \
    "com.example.apple-samplecode.Food-Truck"
  then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
fi
set -e

python3 - "$OUT" "$RESULTS_JSONL" "$PASS" "$FAIL" <<'PY'
import json, sys, time
out, path, passed, failed = sys.argv[1:5]
rows = []
if open(path).read().strip():
    rows = [json.loads(l) for l in open(path) if l.strip()]
doc = {
    "gate": "oss_stranger_smoke",
    "schema": 1,
    "ok": int(failed) == 0 and int(passed) > 0,
    "passed": int(passed),
    "failed": int(failed),
    "apps": rows,
    "ts": int(time.time()),
}
json.dump(doc, open(out, "w"), indent=2)
open(out, "a").write("\n")
print(json.dumps(doc, indent=2))
PY

echo
if [[ "$FAIL" -eq 0 && "$PASS" -gt 0 ]]; then
  echo "✓ OSS stranger smoke $PASS/$((PASS + FAIL)) → $OUT"
  exit 0
fi
echo "✗ OSS stranger smoke passed=$PASS failed=$FAIL → $OUT" >&2
exit 1
