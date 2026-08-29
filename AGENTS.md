# LIGH — agent instructions

You test **iOS Simulator Debug builds** on this Mac via **LIGH MCP**. Fail-closed only.

## Start here

```bash
./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj --build
```

**MCP (3 tools agents need):**

| Tool | When |
|------|------|
| `ligh_init` | Once per project — audit + `.ligh/` bundle |
| `ligh_test` | **After every code change** — goal-first verify |
| `ligh_viewer` | Optional — watch sim in browser |

Then paste MCP config, open `.ligh/AGENT_PROMPT.md` in chat.

Competitive map: [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md)  
OSS stranger contract: [`docs/OSS_PIPELINE.md`](docs/OSS_PIPELINE.md)

## Primary loop (verify)

```text
ligh_init(path)  → once
ligh_up → ligh_viewer (optional)
ligh_test        → goal-first verify from .ligh/app-goal.json
                 → always writes .ligh/last-certify.json
→ { ok: true } or { fault, detail, trail_allowed }
→ if trail_allowed: fix Swift → rebuild → ligh_test
→ if app_crashed / app_not_running: open process_health.crash_report_path — do NOT TRAIL
```

**Eyes first:** if `eyes_unusable` / `sim_boot_hung`, recover Simulator (SpringBoard AX) — do **not** patch the app.
**Crash first:** if `app_crashed`, read DiagnosticReports — never treat as `discover_no_chrome`.

Legacy: `ligh_cap_app_goal` / `ligh_cap_app_job` — prefer **`ligh_test`**.

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

`ok` · `infra` · `eyes_unusable` · `target_missing` · `motor_no_effect` · `wrong_surface` · `app_crashed` · `app_not_running` · `motor_rejected` · `timeout` · `blocked`

Read `detail.step`, `detail.candidates`, `detail.actionable_topk`, `process_health` — fix the app, not the harness. Crash loops are `app_crashed` (open `.ips`), never `discover_no_chrome`.

## Requirements

- macOS + Xcode Simulator
- Stable `accessibilityIdentifier`s on controls you exercise
- Run `./scripts/ligh_audit_accessibility.py <SourceRoot> --suggest-steps` if motor misses targets

## Docs

- [`docs/AGENT_PARADISE.md`](docs/AGENT_PARADISE.md) — onboarding
- [`docs/QA_LAYER.md`](docs/QA_LAYER.md) — MCP contract
- [`docs/TRAIL_BULLETPROOF.md`](docs/TRAIL_BULLETPROOF.md) — repair architecture
