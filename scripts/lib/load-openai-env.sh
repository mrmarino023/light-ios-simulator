#!/usr/bin/env bash
# Load OPENAI_* for agent gates/scripts.
# Order: existing env → LIGH_ENV_FILE → $ROOT/.env
# shellcheck disable=SC1090
load_openai_env() {
  local root="${1:-}"
  if [[ -n "${OPENAI_API_KEY:-}" ]]; then
    return 0
  fi
  if [[ -n "${LIGH_ENV_FILE:-}" && -f "${LIGH_ENV_FILE}" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "${LIGH_ENV_FILE}"
    set +a
    return 0
  fi
  if [[ -n "$root" && -f "$root/.env" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "$root/.env"
    set +a
  fi
}
