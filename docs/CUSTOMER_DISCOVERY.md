# Customer discovery — who has this problem?

**Do not invent the market.** We have interesting engineering. We do not yet know if anyone will change tools for it.

## The right question

Not: *“How do I get LIGH from 25/25 to 30/30?”*

Not: *“Who uses Maestro?”* (Maestro is one competitor among many — and mostly serves **human-written UI tests**, not agent loops.)

**Ask:**

> **How does your coding agent currently interact with an iOS Simulator?**

Watch what they do today. Do not ask if they like LIGH.

## Segment that might care

```text
AI coding agent (Cursor / Claude Code / Codex / …)
        ↓
modifies Swift
        ↓
build
        ↓
launch app
        ↓
"tap Login" · "verify Home"
        ↓
on failure → structured reason → fix code → retry
```

This is **not** normal QA. It is a workflow that only exists because agents exist.

## Pitch (locked)

| Wrong | Right |
|-------|-------|
| “Faster Maestro” | **Execution runtime for coding agents** that must autonomously verify and repair iOS Debug `.app` |
| “12× on login” | **Structured `{ ok, fault, evidence }`** so the agent can fix code without guessing |
| “10,000 developers need this” | **We don't know yet** — discovery first |

## Real competitor: agentic baseline

The product benchmark is **not** LIGH vs Maestro.

Same agent · same task · same app · two arms:

| Arm | Stack |
|-----|--------|
| **A** | Agent + **LIGH MCP** (`ligh_cap_app_job`, faults, reach) |
| **B** | Agent + **baseline** (simctl + screenshot + vision / coordinate guess / retry) |

Measure (publish per run):

- Time to green test
- Tool calls until success
- Failure recovery steps (agent-initiated)
- Human interventions
- Tokens (if available)
- Task completion rate

Maestro A/B is optional footnote — useful for QA-minded devs, not the wedge proof.

## Signals

| They say / do | Signal |
|---------------|--------|
| screenshot → vision → simctl → guess coords → retry | **Strong** — pain exists |
| “I've been trying to make Cursor do exactly this and it's painful” | **Strong** |
| XCTest → CI, works fine | Weak for LIGH wedge |
| “We don't test on Simulator in the agent loop” | Out of segment (for now) |

## Outreach (not a startup pitch)

Send: **repo link + 30s screen recording** of agent loop (build → app-job → fault → fix → green).

One question:

> **How does your agent currently interact with an iOS Simulator?**

No deck. No TAM slide. No “we're raising.”

## Target list (~20) — research worksheet

Fill names as you find real contacts. Categories only; do not treat as validated leads.

| Category | Examples to research |
|----------|-------------------|
| Coding agents / AI IDE | Cursor, Claude Code, Codex, Devin-class tools, Continue, Aider mobile workflows |
| iOS Simulator MCP / tooling | SimPilot, ios-mcp, simulator-mcp, idb, Appium/WDA maintainers |
| Mobile + AI testing | Teams publishing “agent tests mobile” demos or blog posts |
| iOS CI / dev infra | Fastlane, Bitrise, Emerge, teams shipping sim-based agent demos |

Track in a spreadsheet: name · company · how they test iOS today · quote · follow-up.

## Win / kill (after ~5 conversations + agentic A/B)

**Win:** unprompted *“why would I use anything else for agent iOS verify?”* or measurable win on arm A vs B (time, calls, recovery, completion).

**Kill:** everyone already happy with XCTest/CI, or baseline simctl+vision is good enough, or nobody in segment is trying this workflow.

## Artifacts

- Developer install: [`DEVELOPER_TRIAL.md`](DEVELOPER_TRIAL.md)
- Agentic A/B protocol: [`AGENTIC_BASELINE.md`](AGENTIC_BASELINE.md)
- Feedback template: [`assets/developer-feedback-TEMPLATE.json`](assets/developer-feedback-TEMPLATE.json)
- Architecture (not marketing): [`ARCHITECTURE.md`](ARCHITECTURE.md)
