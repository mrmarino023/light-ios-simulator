# Agent paradise — test any iOS app in one command

**Goal:** an agent (or human) goes from zero → working MCP → smoke verify → structured retry loop in ~5 minutes.

## One command

```bash
./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj --build
```

Or if you already have a Simulator `.app`:

```bash
./scripts/ligh-paradise.sh /path/to/build/MyApp.app
```

Or bootstrap from an existing frozen task:

```bash
./scripts/ligh-paradise.sh fixtures/frozen/tasks/login-never-navigates/task.json
```

## What you get

Written to **`<your-project>/.ligh/`**:

| File | Purpose |
|------|---------|
| `project.json` | bundle id, paths, audit, suggested steps |
| `app-job.json` | ready-to-run `ligh_cap_app_job` steps |
| `task.skeleton.json` | TRAIL-style verification template |
| `AGENT_PROMPT.md` | paste into Cursor chat |

Plus: Cursor MCP snippet, smoke run, artifact at `docs/assets/agent-paradise-latest.json`.

## Agent loop (copy to chat)

```text
1. ligh_up → ligh_ready if eyes_unusable
2. ligh_cap_app_job with .ligh/app-job.json steps (edit ids from audit)
3. ok:false → read fault + detail → fix Swift → xcodebuild → retry
4. ok:true only = verified
```

Re-test anytime:

```bash
LIGH_WORKSPACE=/path/to/your/app ./scripts/ligh-test.sh
```

## Accessibility audit

Agents need stable ids — not labels (locale-dependent).

```bash
PYTHONPATH=scripts python3 scripts/ligh_audit_accessibility.py /path/to/Sources --suggest-steps --json
```

Grades: **A** ≥80% interactive controls identified · **B** ≥50% · below = add `.accessibilityIdentifier("…")` before expecting reliable motor.

## Competitive edge (why not vision / Maestro)

| Stack | Agent experience |
|-------|------------------|
| Screenshot + vision | Slow, expensive, flaky postconditions |
| Maestro / XCTest | Human-authored flows; agent must maintain YAML |
| **LIGH** | Structured faults, 0 LLM UI tokens on motor, declarative goals, TRAIL repair |

Headline numbers (repair): login **33s / 1.3k tokens** vs vision **622s / 212k / fail** · L3 held-out **3/3 ≤120s**.

## Gates (honest measurement)

```bash
./scripts/gate-agent-environment.sh   # MCP + motor usable on this Mac
./scripts/gate-trail-holy-multi.sh      # TRAIL L2* regression
./scripts/gate-trail-l3.sh              # TRAIL L3 sealed held-out
./scripts/gate-autopilot-generality.sh  # 0 LLM UI tokens motor
```

## Next (roadmap)

1. **`cap_repair_job` in Rust** — drop Python harness tax on repair hot path
2. **L4** — agent-introduced bugs, no `bug.patch`
3. **GitHub Action** — `ligh test` on every PR
4. **Hosted Mac runner** — agents without local Xcode

## See also

- [`AGENTS.md`](../AGENTS.md) — short agent rules
- [`DEVELOPER_TRIAL.md`](DEVELOPER_TRIAL.md) — manual trial + feedback
- [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md) — repair architecture
