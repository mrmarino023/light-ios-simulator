#!/usr/bin/env bash
# Thin wrapper — all intelligence lives in ligh_oss_smoke.py (no per-app maps).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export LIGH_BIN="${LIGH_BIN:-$ROOT/target/release/ligh}"
export LIGH_DEVICE="${LIGH_DEVICE:-iphone-15-pro}"
export LIGH_OSS_WORK="${LIGH_OSS_WORK:-$ROOT/.oss-trial}"
OUT="${LIGH_OSS_OUT:-$ROOT/docs/assets/oss-stranger-trial-latest.json}"
URLS="${1:-$ROOT/scripts/oss-stranger-urls.txt}"
exec python3 "$ROOT/scripts/ligh_oss_smoke.py" --urls-file "$URLS" --write "$OUT"
