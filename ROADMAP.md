# Roadmap

> Product story: [`README.md`](README.md). Brutal status: [`docs/HONEST.md`](docs/HONEST.md).

## One line

Not: more motor → more faults → more benchmarks → more fixtures.

**Real app → real agent → real task → real verification → real developer.**

## The bet

**LIGH is the verification runtime for coding agents building iOS apps** — reliable interaction + **proved outcome** (`VERIFIED` / `FAILED` + evidence).

Claim we want to earn:

> LIGH gives agents reliable interaction and verification, regardless of how the UI exposes itself.

AX-first when available. Not AX-only.

---

## Top 5 — in order of impact

### 1. Full loop: code → build → test → fix → verify

**The biggest leap.** Demo target:

> *"Fix the broken onboarding and make sure it works."*

Agent autonomously: **edit Swift → build → launch `.app` → explore → interact → verify → if fail, fix → retry.**

- Not a LIGH-co-designed fixture
- **Frozen real / open-source app**
- Publish video + JSON trace

Status: **not done** on a non-co-designed app with fix+retry loop.

---

### 2. Vision fallback (keep AX primary)

Do **not** sell “AX instead of pixels.”

```text
AX available?
  yes → semantic interaction (perceive / attempt)
  no  → screenshot/vision → locate target
        → action
        → AX/state verification
```

AX becomes an **advantage**, not a requirement that excludes half of apps.

Status: **not built** as integrated fallback path.

---

### 3. Benchmark: 10 real OSS apps × 5 tasks

Not more of our fixtures.

- **10** open-source iOS apps we did **not** modify for LIGH
- **5** task types each: onboarding · create item · edit · navigate · multi-step flow

Compare: **LIGH** vs **screenshot+vision baseline** vs **XCUITest/Maestro** where fair.

| Metric only |
|-------------|
| Completion rate |
| False-success rate |
| Human interventions |
| Wall-clock |
| Tokens |

Win example: LIGH 90% vs baseline 60% → you have a story. Another login×50 does not.

Status: **not run.**

---

### 4. `brew install ligh` + 2 minutes

Today is too “research project.” Target:

```text
brew install ligh
ligh init
```

Then: Cursor → LIGH MCP → `MyApp.app` → agent says *"I can test your app."*

No lecture on `lighd`, gates, fixtures, Rust workspace. **If install takes longer than the demo, you lost.**

Status: clone + `cargo build` + daemon + MCP paste — see [`docs/DEVELOPER_TRIAL.md`](docs/DEVELOPER_TRIAL.md).

---

### 5. Five developer trials (the decision)

**Most important validation** — run as soon as #4 is “good enough”, don’t wait for #3 to be perfect.

Find **5** people who: build iOS · use Cursor / Claude Code / Copilot · have a **real app**.

Tell them:

> *"Use your coding agent to change something in your app and verify it works. Don't ask me how."*

**No assisted onboarding.** Measure: *"Would you keep this installed?"*

| Result | Action |
|--------|--------|
| **3+ yes** | Invest in #1–#3 properly |
| **0/5 yes** | Stop adding capabilities; pivot or kill product thesis |

---

## Stop (capability trap)

Until #5 gives a signal:

- No new UX graph thesis · no login×100 · no new gate dimensions
- Compiled replay / graph → frozen research footnotes
- Don’t build #2–#3 in depth if #5 fails on a thin slice of #1+#4

---

## User-facing feature (what we’re building toward)

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

User never needs to know motor, app-job, UX graph, or `lighd`.

---

## Engineering already done (footnotes)

Published: [`docs/assets/`](docs/assets/). Do not lead with these.

- Fail-closed · dirty 50/50 · fixture + XCUITestDemo rigor (narrow jobs)
- QA perceive/attempt · autonomous UX on **our** fixture · 1× LLM Swift fix PoC
- **Disproven:** UX graph helps LLM navigate

Harnesses = falsification, not marketing.

---

## Out of scope (for now)

Cloud sim · device fleets · Maestro parity chase · more internal fixtures
