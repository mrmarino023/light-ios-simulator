# Agentic baseline — LIGH vs what agents do today

**Purpose:** falsify the product thesis against the **actual** alternative — not Maestro.

## Setup

1. One Mac, one Simulator, one Debug `.app` (yours or XCUITestDemo).
2. Same Cursor agent, same model, same task description.
3. Two runs — order randomized if possible.

### Arm A — LIGH

- MCP: `./scripts/print-cursor-mcp.sh`
- Prompt: [`CURSOR_PROMPT.md`](CURSOR_PROMPT.md)
- Success = `ligh_cap_app_job` returns `{ "ok": true }`

### Arm B — Baseline (pick what they use today)

Use whatever the interviewee already runs. Common patterns:

```text
xcrun simctl boot / install / launch
xcrun simctl io booted screenshot
→ vision model describes UI
→ tap by coordinates or guessed element
→ screenshot again → retry
```

If they have **ios-mcp**, **SimPilot**, or **simctl-only** scripts — use that. The baseline must be **their** stack, not a strawman.

**Rules for arm B:** no LIGH tools; agent may use screenshots for planning (that's the point).

## Task (keep narrow)

Example (XCUITestDemo):

```text
Build Debug app for Simulator.
Log in with test@example.com / password.
Verify home screen is visible.
If anything fails, fix the app and retry until green or blocked.
```

For your app: same shape — 1 flow, 1 assert, agent may edit Swift.

## Record

Copy [`assets/agentic-baseline-TEMPLATE.json`](assets/agentic-baseline-TEMPLATE.json) and fill after each arm.

| Field | Why |
|-------|-----|
| `time_to_green_s` | Wall clock to verified success |
| `tool_calls` | Agent turns / MCP invocations |
| `human_interventions` | You had to steer |
| `recovery_attempts` | Fix-code loops after failure |
| `completed` | Task done without giving up |
| `failure_mode` | Where it died (vision wrong tap, no fault signal, install, …) |
| `one_sentence` | Agent or you: why A or B was easier to act on |

## Publish

```bash
# After both arms — merge into one JSON under docs/assets/
cp docs/assets/agentic-baseline-TEMPLATE.json docs/assets/agentic-baseline-latest.json
# edit latest with both arms + notes
```

Do **not** publish as “LIGH wins” from a single run. Need pattern across apps and/or developers.

## What this proves

| Outcome | Interpretation |
|---------|----------------|
| A faster, fewer calls, better recovery | Wedge might be real |
| B good enough | No business — OSS niche at best |
| Both fail | Task too hard or app problem — separate from tool thesis |
| Only A fails install | Fix cold start, not motor |

## Related (footnotes only)

- Maestro A/B: `./scripts/print-comparison-mcp.sh` — QA-minded comparison, not primary thesis
- Engineering rigor N=50: [`third-party-rigor-latest.json`](assets/third-party-rigor-latest.json) — login job only
