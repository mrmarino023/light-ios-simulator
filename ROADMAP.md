# Roadmap

> Product story: [`README.md`](README.md). Brutal status: [`docs/HONEST.md`](docs/HONEST.md).

## The bet (one sentence)

**LIGH is the verification runtime for coding agents building iOS apps** — observe, act, and **prove** the outcome (`VERIFIED` / `FAILED` + evidence), not “I think it worked.”

Not: faster Maestro · UX graph · control plane · another login N=50.

## The only feature (user-facing)

```text
User: "Fix the broken onboarding and verify it works."

Agent:
  → edits Swift
  → builds app
  → opens Simulator
  → explores UI
  → performs actions
  → verifies success condition
  → reports VERIFIED / FAILED with evidence
```

The user must **not** need to know `lighd`, motor, app-job, UX graph, or gates. Those are implementation.

## What wins (competitive)

| We must win on | Not enough alone |
|----------------|------------------|
| Agent completes real verify tasks with fewer false successes | “We use accessibility not pixels” |
| Outcome verification the agent can trust | Raw tap speed |
| Easier than “agent writes XCUITest” or “agent screenshots + vision” on **their** loop | Our internal benchmarks |

**Conceptual competitor:** not Maestro. The question:

> *How does a coding agent know its iOS change actually worked?*

## Stop now (capability trap)

Do **not** add major features until **5 external developers** try LIGH on their own app:

- No new graph thesis · no new gate dimensions · no login×100
- Compiled replay, UX graph, rigor arms → **frozen** (research footnotes only)
- Building AX-only hardening without user signal → **wrong order**

## Known weakness (honest)

Today LIGH works best with **`accessibilityIdentifier`**. That is a competitive ceiling until we ship **best available signal** (label, role, visible text, hierarchy) and **vision fallback when AX is thin** — but only **after** users say “I don’t have ids” in real trials, not from benchmark anxiety.

## This week

- [x] Compress README → product-first (“Agent, test the app for me”)
- [ ] **5-minute onboarding** — target: `install` → MCP → agent verifies (hide researcher steps)
- [ ] **One irresistible demo** — agent edits Swift → agent tests → `VERIFIED` / `FAILED` on video
- [ ] **No new core capabilities**

## Then (the benchmark is human)

Find **5 iOS developers** using Cursor / Claude Code / Copilot.

**Do not ask:** “Do you like LIGH?”

**Ask:** “Show me how you verify an agent’s UI change today. Now try the same task with LIGH.”

Watch where it breaks. Count human interventions.

| If they say… | Then build… |
|--------------|-------------|
| “No accessibility ids” | Label/role/text resolution + vision fallback |
| “Install is a mess” | `brew install` / `ligh init` packaging |
| “I need CI” | CI integration |
| “Agent doesn’t know what to test” | Goal / success inference |
| “I’d just generate XCUITest” | **Kill or pivot product thesis** |

**Win:** 3/5 use it without a 1-hour explanation, or measurable win vs their baseline.

**Kill:** after 5 trials, nobody prefers it and can’t say why → stop product thesis; keep motor as OSS.

## Later (real agent benchmark — not login×50)

When (if) humans signal interest, run **~10 small OSS iOS apps** we did not co-design:

- Agent makes a change · must verify a user journey
- LIGH vs reasonable baseline (XCUITest generation, simctl+vision, their MCP)

| Metric | Why |
|--------|-----|
| Task completion | Does the agent finish? |
| False success | Said “done” when UI wasn’t there? |
| Human interventions | How often you had to rescue? |
| Wall-clock | Loop speed |
| Token cost | Explore/verify cost |

That beats another 4× SpringBoard microbench.

## Onboarding target (not today’s reality)

```text
brew install ligh          # aspirational
ligh init                  # aspirational
“Use LIGH to verify UI changes.”   # in Cursor
```

Today: clone, cargo build, daemon, MCP paste — [`docs/DEVELOPER_TRIAL.md`](docs/DEVELOPER_TRIAL.md). Close the gap when packaging is the blocker users report.

---

## Engineering already done (footnotes — do not lead with these)

Published JSON in [`docs/assets/`](docs/assets/). Motor regression ≠ generalization.

- Fail-closed 5/5 · dirty 50/50 · fixture + XCUITestDemo rigor N=50
- QA perceive/attempt · autonomous UX harness · 1× LLM Swift fix PoC
- MCP mechanism loop · compiled replay (experimental)
- **Disproven:** UX graph helps LLM navigate ([`ux-graph-prove-latest.json`](docs/assets/ux-graph-prove-latest.json))

Legacy harness index: `gate-app-reliability.sh`, `gate-third-party-rigor.sh`, `gate-autonomous-ux.sh`, `gate-compiled-replay.sh`, `gate-agentic-baseline.sh` — run for falsification, not marketing.

Architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · agent API: [`docs/QA_LAYER.md`](docs/QA_LAYER.md)

## Out of scope (for now)

Cloud sim farms · device fleets · OCR-as-product · Maestro parity chase
