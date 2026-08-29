# LIGH — agent instructions

You are on a **truth machine for iOS coding-agent changes**: fail-closed certify + optional TRAIL repair.  
Not “tap Simulator for fun” — that is Maestro/XcodeBuildMCP territory.

## Product surfaces (prefer order)

| Surface | When |
|---------|------|
| **Scorepack** | Eval / “did the agent fix it?” — `./scripts/gate-scorepack.sh` |
| **`ligh_test` / agent-loop** | After a Swift change — `.ligh/last-certify.json` with `ok: true` only |
| **TRAIL** | Host prove → localize → ≤2 patches → certify (lab + scorepack) |
| MCP paradise | Local dogfood only — [`docs/AGENT_PARADISE.md`](docs/AGENT_PARADISE.md) |

Competitive map: [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md) · Scorepack: [`docs/SCOREPACK.md`](docs/SCOREPACK.md)

## Primary loop (verify)

```text
ligh_init(path) → once
ligh_up → ligh_viewer (optional)
ligh_test → goal-first verify from .ligh/app-goal.json
         → always writes .ligh/last-certify.json
→ { ok: true } or { fault, detail, trail_allowed }
→ if trail_allowed: fix Swift → rebuild → ligh_test
→ if app_crashed / app_not_running: open process_health.crash_report_path — do NOT TRAIL
```

**Eyes first:** if `eyes_unusable` / `sim_boot_hung`, recover Simulator — do **not** patch the app.  
**Crash first:** if `app_crashed`, read DiagnosticReports — never treat as `discover_no_chrome`.

```bash
LIGH_WORKSPACE=/path/to/app ./scripts/ligh-agent-loop.sh
```

**Never** claim success without `ok: true`. **Never** plan taps from screenshots.

## Repair loop (when motor proves a bug)

```text
prove → classify → localize → structural fix → ≤2 LLM patches → certify
```

See [`docs/TRAIL_BULLETPROOF.md`](docs/TRAIL_BULLETPROOF.md). Scorepack core tasks: login · tab chrome · overlay.

## Fault taxonomy

`ok` · `infra` / `infra_oom` · `eyes_unusable` · `target_missing` · `motor_no_effect` · `wrong_surface` · `app_crashed` · `app_not_running` · `motor_rejected` · `timeout` · `blocked`

## Requirements

- macOS + Xcode Simulator
- Stable `accessibilityIdentifier`s on controls you exercise (labels work, ids better)
- Builds go through **BuildGovernor** (serialize + memory gate)

## Docs

- [`docs/SCOREPACK.md`](docs/SCOREPACK.md) — buyer contract  
- [`docs/AGENT_PARADISE.md`](docs/AGENT_PARADISE.md) — local onboarding  
- [`docs/QA_LAYER.md`](docs/QA_LAYER.md) — MCP contract  
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — host planes  
