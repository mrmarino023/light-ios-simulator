# Agent environment — what actually works today

**Honest status.** We built the **primitives** for a coding-agent loop; we did **not** yet ship a turnkey “agent does everything on any app” product.

## The loop we promised

```text
Cursor + LIGH MCP
  → ligh_up / ligh_ready
  → ligh_observe (structured)
  → ligh_cap_app_goal | app_job | reach
  → { ok, fault, evidence } → agent fixes Swift → retry
```

That loop **exists** when AX is settled and the app has reachable labels/ids.

## Capability matrix (2026-08-22)

| Capability | MCP tool | Host motor | Notes |
|------------|----------|------------|-------|
| Session boot | `ligh_up`, `ligh_ready` | ✅ | Cold path needs first-loop wake |
| Observe (no PNG) | `ligh_observe`, `ligh_sense` | ✅ | Fail-closed on transition/empty |
| Launch system app | `ligh_launch` | ✅ `launch` op in app_goal setup | No `.app` path needed |
| Launch Debug `.app` | `ligh_cap_run_app`, `ligh_run` | ✅ | Product path |
| Tap / type / wait | `ligh_tap`, `ligh_type`, `ligh_wait` | ✅ fire_verified | Prefer cap_* or app_goal |
| Reach / scroll | `ligh_cap_reach`, `ligh_scroll_until` | ✅ | Host-owned |
| Swipe | `ligh_swipe` | ✅ CLI/RPC | Gesture explore |
| Key (return, …) | `ligh_key` | ✅ | In app_goal `key` op |
| Declarative job | `ligh_cap_app_job` | ✅ | id/label steps + dismiss_overlay |
| Declarative goal | `ligh_cap_app_goal` | ✅ | setup + postconditions |
| Structured faults | all cap_* | ✅ | target_missing, motor_no_effect, … |
| Autonomous LLM + fix | `autonomous-login-agent.py` | ✅ | XCUITestDemo bug inject |
| Unified agent loop | `agent-unified-loop.py` | ✅ scripted + optional LLM |
| Human motor (probe, settle) | `ligh_cap_explore`, cognition | ✅ P1 — probe planner in host |
| Agentic baseline gate | — | ❌ not wired | compare vs simctl+vision manually |
| Legacy demo caps | `open_settings`, `settings_search` | ⚠️ still in MCP | **Do not use** — use app_goal |

## Install → Cursor (5 min)

```bash
git clone … && cd light-ios-simulator
./scripts/install.sh
./scripts/gate-agent-environment.sh   # must pass before you trust MCP
./scripts/print-cursor-mcp.sh           # paste into Cursor Settings → MCP
```

Agent prompt: [`CURSOR_PROMPT.md`](CURSOR_PROMPT.md) · rules: [`AGENT.md`](AGENT.md)

## What “everything” means (scope)

**In scope for agents today**

- Debug `.app` with accessibility identifiers
- System apps via `launch` + label steps (Safari, Settings, …)
- Fail-closed faults with candidates for recovery
- One-shot autonomous fix demo (XCUITestDemo)

**Not in scope yet**

- Arbitrary unlabeled UIs (vision product)
- Host probe planner / explore budget ([`HUMAN_MOTOR.md`](HUMAN_MOTOR.md) P2–P4)
- Guaranteed cold-install green without `ligh_ready` (trial still flaky on AX wake)

## If something fails

| Symptom | Fix |
|---------|-----|
| `eyes_unusable` at step 0 | `ligh_ready` or `./scripts/agent-first-loop.sh` first |
| `motor_no_effect` | `ligh_cap_reach` or fix affordance in app |
| `target_missing` | read `evidence.candidates`, fix a11y id/label |
| MCP tools missing | rebuild + restart Cursor MCP |
| Stale `lighd` | `./scripts/lib/sim-clean.sh` + `ligh daemon start` |

## Validate

```bash
./scripts/gate-agent-environment.sh
# → docs/assets/agent-environment-latest.json
```

Pass = agent environment is **honestly** ready for Cursor trials. Fail = fix infra before pitching.
