#!/usr/bin/env bash
# Thin wrapper around ligh_oss_smoke.py — URL-only, no per-app scheme/label maps.
#
# Usage:
#   ./scripts/gate-oss-stranger-smoke.sh
#   ./scripts/gate-oss-stranger-smoke.sh countries
#   ./scripts/gate-oss-stranger-smoke.sh foodtruck
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export LIGH_BIN="${LIGH_BIN:-$ROOT/target/release/ligh}"
export LIGH_DEVICE="${LIGH_DEVICE:-iphone-15-pro}"
export LIGH_OSS_WORK="${LIGH_OSS_WORK:-${RUNNER_TEMP:-/tmp}/ligh-oss-smoke}"
OUT="${LIGH_OSS_OUT:-$ROOT/docs/assets/oss-stranger-smoke-latest.json}"
SELECT="${1:-all}"

COUNTRIES="https://github.com/nalexn/clean-architecture-swiftui.git"
FOODTRUCK="https://github.com/apple/sample-food-truck.git"

specs=()
case "$SELECT" in
  all) specs=("$COUNTRIES" "$FOODTRUCK") ;;
  countries|countrieswiftui) specs=("$COUNTRIES") ;;
  foodtruck|food-truck|food_truck) specs=("$FOODTRUCK") ;;
  *) echo "unknown select: $SELECT (all|countries|foodtruck)" >&2; exit 1 ;;
esac

exec python3 "$ROOT/scripts/ligh_oss_smoke.py" --write "$OUT" "${specs[@]}"
