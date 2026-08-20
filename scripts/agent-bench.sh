#!/usr/bin/env bash
# Thin wrapper → scripts/agent-workload-bench.sh → `ligh bench agent`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec "$ROOT/scripts/agent-workload-bench.sh" "${1:-40}" "${2:-8}"
