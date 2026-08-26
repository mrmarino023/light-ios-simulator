# LIGH — agent instructions

You test **iOS Simulator Debug builds** on this Mac via **LIGH MCP**. Fail-closed only.

## Start here

```bash
./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj --build
# → .ligh/project.json · app-job.json · AGENT_PROMPT.md · MCP snippet
```

Then paste MCP config, open `.ligh/AGENT_PROMPT.md` in chat, run `LIGH_WORKSPACE=/path/to/app ./scripts/ligh-test.sh`.

Full guide: [`docs/AGENT_PARADISE.md`](docs/AGENT_PARADISE.md)

## Primary loop (verify)

```text
ligh_up → ligh_ready (if eyes_unusable)
→ ligh_cap_app_job(app, steps) OR ligh_cap_app_goal(setup, postconditions)
→ { ok: true } or { fault, detail } → fix Swift → rebuild → retry
```

**Never** claim success without `ok: true`. **Never** plan taps from screenshots.

## Repair loop (when motor proves a bug)

Host path (same for every OSS app):

```text
prove → classify → localize → structural fix → ≤2 LLM patches → certify
```

See [`docs/TRAIL_BULLETPROOF.md`](docs/TRAIL_BULLETPROOF.md).

## Tools (prefer order)

| Tool | Use |
|------|-----|
| `ligh_cap_app_goal` | Declarative postconditions — **best for agents** |
| `ligh_cap_app_job` | Explicit steps (wait/tap/type) |
| `ligh_perceive` / `ligh_attempt` | Low-level when caps miss |
| `ligh_ready` | Recover from SpringBoard / eyes_unusable |

## Fault taxonomy

`ok` · `infra` · `eyes_unusable` · `target_missing` · `motor_no_effect` · `wrong_surface` · `motor_rejected` · `timeout` · `blocked`

Read `detail.step`, `detail.candidates`, `detail.actionable_topk` — fix the app, not the harness.

## Requirements

- macOS + Xcode Simulator
- Stable `accessibilityIdentifier`s on controls you exercise
- Run `./scripts/ligh_audit_accessibility.py <SourceRoot> --suggest-steps` if motor misses targets

## Docs

- [`docs/AGENT_PARADISE.md`](docs/AGENT_PARADISE.md) — onboarding
- [`docs/QA_LAYER.md`](docs/QA_LAYER.md) — MCP contract
- [`docs/TRAIL_BULLETPROOF.md`](docs/TRAIL_BULLETPROOF.md) — repair architecture
