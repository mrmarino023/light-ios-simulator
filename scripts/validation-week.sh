#!/usr/bin/env bash
# Validation week driver — freeze architecture, grow the matrix, publish distributions.
#
# Commands:
#   ./scripts/validation-week.sh              # coverage vs minimum bar
#   ./scripts/validation-week.sh ingest       # import already-published artifacts
#   ./scripts/validation-week.sh smoke        # generality (zero LLM tokens)
#   ./scripts/validation-week.sh paired       # honest A/B repeats (needs OPENAI_API_KEY)
#   ./scripts/validation-week.sh summarize    # medians, p90, pass rate, taxonomy
#
# Env:
#   LIGH_VW_REPEAT=5
#   LIGH_VW_TASKS="login-never-navigates login-button-disabled"
#   LIGH_VW_ARMS="autopilot baseline"
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CMD="${1:-status}"
PY="$ROOT/scripts/validation_week.py"
REPEAT="${LIGH_VW_REPEAT:-5}"
TASKS="${LIGH_VW_TASKS:-login-never-navigates kix-login-never-authenticates}"
ARMS="${LIGH_VW_ARMS:-autopilot baseline}"
RUNS="$ROOT/docs/assets/validation-week-runs"

fail() { echo "✗ $*" >&2; exit 1; }

# shellcheck source=lib/load-openai-env.sh
source "$ROOT/scripts/lib/load-openai-env.sh"

task_file() {
  local id="$1"
  echo "$ROOT/fixtures/frozen/tasks/$id/task.json"
}

case "$CMD" in
  status)
    python3 "$PY" status
    ;;
  ingest)
    python3 "$PY" ingest
    python3 "$PY" status || true
    ;;
  summarize)
    python3 "$PY" summarize
    python3 "$PY" status || true
    ;;
  smoke)
    echo "══ Validation week · smoke (generality, 0 LLM tokens) ══"
    "$ROOT/scripts/gate-autopilot-generality.sh" || true
    python3 "$PY" ingest
    python3 "$PY" status || true
    ;;
  paired)
    load_openai_env "$ROOT"
    [[ -n "${OPENAI_API_KEY:-}" ]] || fail "OPENAI_API_KEY required for paired runs"
    mkdir -p "$RUNS"
    echo "══ Validation week · paired A/B ══"
    echo "  tasks=$TASKS  repeats=$REPEAT  arms=$ARMS"
    echo "  protocol=scored  (no exercise_app/coaching, same verifier both arms)"
    echo ""
    export LIGH_KILLER_HONEST=1
    export LIGH_KILLER_SCORED=1
    for task in $TASKS; do
      tf="$(task_file "$task")"
      [[ -f "$tf" ]] || fail "missing task $tf"
      for i in $(seq 1 "$REPEAT"); do
        for arm in $ARMS; do
          echo "── $task  run $i/$REPEAT  arm=$arm"
          ARM_OUT="$RUNS/${arm}-${task}-r${i}-raw.json"
          LEDGER_OUT="$RUNS/${arm}-${task}-r${i}.json"
          [[ ! -e "$ARM_OUT" && ! -e "$LEDGER_OUT" ]] || \
            fail "append-only run $task/$i/$arm already exists; use a new repeat index"
          set +e
          LIGH_KILLER_HONEST=1 LIGH_KILLER_SCORED=1 LIGH_KILLER_ARM="$arm" LIGH_KILLER_OUT="$ARM_OUT" \
            LIGH_KILLER_TASK="$tf" \
            "$ROOT/scripts/gate-killer-loop.sh" >"/tmp/vw-${arm}-${task}-${i}.log" 2>&1
          set -e
          if [[ -f "$ARM_OUT" ]]; then
            python3 "$PY" record "$ARM_OUT" --app "$(python3 -c "import json; print(json.load(open('$tf'))['app_id'])")" \
              --task "$task" --arm "$arm" --repeat "$i" --kind bugfix \
              --dest "$LEDGER_OUT"
          else
            echo "   ✗ no artifact (see /tmp/vw-${arm}-${task}-${i}.log)"
            python3 - "$ARM_OUT" "$tf" "$arm" "$task" "$i" <<'PY'
import hashlib, json, subprocess, sys
raw, task_file, arm, task, i = sys.argv[1:6]
spec = json.load(open(task_file))
prompt = spec["agent_prompt"]
try:
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
except Exception:
    sha = "unknown"
open(raw, "w").write(json.dumps({
    "artifact_schema_version": 2,
    "protocol": "scored",
    "protocol_version": spec["protocol_version"],
    "arm": arm,
    "task": task,
    "task_prompt": prompt,
    "prompt_hash": hashlib.sha256(prompt.encode()).hexdigest(),
    "app_id": spec["app_id"],
    "model": __import__("os").environ.get("OPENAI_MODEL", "gpt-5-mini"),
    "git_sha": sha,
    "verified": False,
    "claim_pass": False,
    "wall_time_ms": None,
    "llm_tokens": 0,
    "failure_phase": "infra",
    "failure_class": "infra_flake",
    "verification_reason": "no_artifact",
    "protocol_violations": [],
    "agent_actions": []
}, indent=2) + "\n")
PY
            python3 "$PY" record "$ARM_OUT" \
              --app "$(python3 -c "import json; print(json.load(open('$tf'))['app_id'])")" \
              --task "$task" --arm "$arm" --repeat "$i" --kind bugfix \
              --dest "$LEDGER_OUT"
          fi
        done
      done
    done
    python3 "$PY" summarize
    python3 "$PY" status || true
    echo "══ → $ROOT/docs/assets/validation-week-summary.json"
    echo "══ → $ROOT/docs/VALIDATION_WEEK_RESULTS.md"
    ;;
  *)
    echo "usage: $0 [status|ingest|smoke|paired|summarize]" >&2
    exit 2
    ;;
esac
