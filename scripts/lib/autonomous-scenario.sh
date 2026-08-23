#!/usr/bin/env bash
# Apply / restore XCUITestDemo scenario patches for autonomous gates.
# shellcheck shell=bash
AUTONOMOUS_DEMO_DIR="${AUTONOMOUS_DEMO_DIR:-$ROOT/fixtures/third-party/XCUITestDemo/XCUITestDemo}"
AUTONOMOUS_SCENARIOS_DIR="${AUTONOMOUS_SCENARIOS_DIR:-$ROOT/fixtures/third-party/XCUITestDemo/scenarios}"

autonomous_backup_sources() {
  AUTONOMOUS_BAK=$(mktemp -d)
  cp "$AUTONOMOUS_DEMO_DIR"/ContentView.swift "$AUTONOMOUS_BAK/"
  cp "$AUTONOMOUS_DEMO_DIR"/LoginViewModel.swift "$AUTONOMOUS_BAK/"
}

autonomous_restore_sources() {
  if [[ -n "${AUTONOMOUS_BAK:-}" && -d "$AUTONOMOUS_BAK" ]]; then
    cp "$AUTONOMOUS_BAK/ContentView.swift" "$AUTONOMOUS_DEMO_DIR/"
    cp "$AUTONOMOUS_BAK/LoginViewModel.swift" "$AUTONOMOUS_DEMO_DIR/"
  else
    git -C "$ROOT" checkout -- \
      fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift \
      fixtures/third-party/XCUITestDemo/XCUITestDemo/LoginViewModel.swift 2>/dev/null || true
  fi
}

autonomous_apply_scenario() {
  local patch_name="$1"
  local patch="$AUTONOMOUS_SCENARIOS_DIR/$patch_name"
  [[ -f "$patch" ]] || { echo "missing patch: $patch" >&2; return 1; }
  git -C "$ROOT" checkout -- \
    fixtures/third-party/XCUITestDemo/XCUITestDemo/ContentView.swift \
    fixtures/third-party/XCUITestDemo/XCUITestDemo/LoginViewModel.swift 2>/dev/null \
    || autonomous_restore_sources
  patch -p1 -d "$ROOT" < "$patch"
}

autonomous_load_env() {
  # shellcheck source=load-openai-env.sh
  source "${AUTONOMOUS_ROOT:-$ROOT}/scripts/lib/load-openai-env.sh"
  load_openai_env "${AUTONOMOUS_ROOT:-$ROOT}"
}

autonomous_prepare_sim() {
  sim_clean_reboot "$LIGH"
  if ! "$ROOT/scripts/agent-first-loop.sh" >/tmp/autonomous-first.log 2>&1; then
    echo "  ⚠ SpringBoard wake slow — ligh_ready for app-only job"
    "$LIGH" --json ready --settle-ms 3000 --recover-homes 4 >/tmp/autonomous-ready.log 2>&1 \
      || return 1
  fi
}
