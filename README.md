<h1 align="center">LIGH</h1>

<p align="center">
  <strong>Local iOS Simulator control plane for coding agents</strong><br/>
  Open source · MIT · macOS + Xcode Simulator only
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT" /></a>
  <a href="https://github.com/mrmarino023/light-ios-simulator"><img src="https://img.shields.io/badge/github-open%20source-black.svg" alt="GitHub" /></a>
</p>

---

## What this is

LIGH is a **persistent Rust host** (`lighd`) around Apple’s real CoreSimulator. Coding agents drive **your Debug `.app`** through structured tools — accessibility JSON, verified taps/types, explicit faults — instead of screenshot + vision guessing.

**One sentence:** give Cursor (or any MCP client) `perceive` / `attempt` on real accessibility ids, get **verified or fail-closed** — then optionally **replay the same flow with zero LLM tokens**.

Not a new simulator. Not “Playwright for iOS” as marketing fluff. A **Mac-local agent substrate** for apps that expose accessibility identifiers.

---

## Use cases (why bother)

### 1. Agent verifies a build while you iterate Swift

You change onboarding. The agent should answer: *did the flow still reach the success screen?*

```text
Agent → ligh_perceive (read ids) → ligh_attempt (tap/type + expect) → harness checks success id
```

No scripted coordinates. No “I think it worked” from the model — the harness looks for `HomeReady` / `homeTitle` in the tree.

**Gate:** `./scripts/gate-autonomous-ux.sh` · evidence: [`autonomous-ux-latest.json`](docs/assets/autonomous-ux-latest.json)

### 2. Cheap CI replay after the agent found the path once

First run: LLM discovers the flow (~84s, ~42k tokens on LighOnboard in our fixture).  
Second run onward: **compiled motor replay, 0 LLM tokens** (~31s on the same fixture).

```text
seed (QA) → uxgraph compile-flow → uxgraph execute-compiled
```

**Gate:** `./scripts/gate-compiled-replay.sh` · evidence: [`compiled-replay-latest.json`](docs/assets/compiled-replay-latest.json)

Use this when the flow is stable and you want **regression without paying the model every time**.

### 3. Known-step jobs (CI / acceptance)

When steps are already known (login matrix, smoke test), skip the LLM entirely:

```bash
ligh cap app-job /path/to/MyApp.app --bundle-id com.you.app --steps '[
  {"op":"wait","id":"usernameTextField"},
  {"op":"type","text":"alice","id":"usernameTextField"},
  {"op":"tap","id":"loginButton","until_id":"homeTitle"}
]'
```

**Gates:** `./scripts/gate-app-reliability.sh` · OSS: [`third-party-rigor-latest.json`](docs/assets/third-party-rigor-latest.json)

---

## Why agents might prefer this over simctl + screenshots

| DIY stack | LIGH |
|-----------|------|
| Screenshot → vision model → guessed tap | `ligh_perceive` → read `id` / `label` from AX JSON |
| Silent failures, retries burn tokens | `ligh_attempt` returns `intent_met`, faults, hypotheses |
| Agent declares “done” | Harness checks success id independently |
| New MCP spawn per action | Persistent `lighd` RPC + [`ligh_mcp.py`](scripts/ligh_mcp.py) |

**Requirements on your app:** stable `accessibilityIdentifier` (or labels) on the elements you care about. Without that, LIGH has nothing reliable to grab — same as any AX automation.

Deep dive: [`docs/QA_LAYER.md`](docs/QA_LAYER.md)

---

## Proven today (published JSON)

| What | Result | Evidence |
|------|--------|----------|
| **QA layer** — perceive + attempt + evidence | Demonstrated on fixture | [`qa-layer-latest.json`](docs/assets/qa-layer-latest.json) |
| **Autonomous UX** — LLM, no scripted nav, harness verify | Pass on LighOnboard | [`autonomous-ux-latest.json`](docs/assets/autonomous-ux-latest.json) |
| **Compiled replay** — seed → compile → execute, 0 LLM | Pass on LighOnboard | [`compiled-replay-latest.json`](docs/assets/compiled-replay-latest.json) |
| **Fail-closed** — injected faults never soft-success | 5/5 | [`fail-closed-latest.json`](docs/assets/fail-closed-latest.json) |
| **Dirty state** — 50 back-to-back app-jobs, no reboot | 50/50 | [`dirty-state-latest.json`](docs/assets/dirty-state-latest.json) |
| **OSS login job** (XCUITestDemo) — app-job reliability | LIGH 50/50 rigor arm | [`third-party-rigor-latest.json`](docs/assets/third-party-rigor-latest.json) |
| **MCP mechanism** — fault → retry → ok | Scripted proof | [`mcp-loop-latest.json`](docs/assets/mcp-loop-latest.json) |
| **LLM fix loop (1×)** — vague prompt → Swift fix | 1/1 injected bug | [`autonomous-agent-latest.json`](docs/assets/autonomous-agent-latest.json) |

Footnotes (real but narrow — **one login-style job**, not general superiority):

- vs Maestro on that job: ~12× p50 at N=50 — see rigor JSON; Maestro was 20/20 at N=20, broke at N=50 under load
- vs WDA on a 44-step SpringBoard script: ~4× wall-clock — research microbench, not the product claim

---

## What we do **not** claim

- “Beats Maestro / Appium in general”
- “UX graph makes agents smarter” — **disproven** on OSS A/B ([`ux-graph-prove-latest.json`](docs/assets/ux-graph-prove-latest.json))
- “Any app, any flow, zero setup” — needs accessibility ids on Debug Simulator builds
- “Developers already prefer this” — zero external validation ([`docs/CUSTOMER_DISCOVERY.md`](docs/CUSTOMER_DISCOVERY.md))
- “Autonomous debugging at scale” — 1× proof + small matrix only

Full honesty doc: [`docs/HONEST.md`](docs/HONEST.md) · plan: [`ROADMAP.md`](ROADMAP.md)

---

## Install

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator
./scripts/install.sh
unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon
./scripts/developer-trial.sh
ligh doctor
```

- Xcode + iOS Simulator runtime · Rust 1.82+
- LLM gates: copy [`.env.example`](.env.example) → `.env` with `OPENAI_API_KEY`

Guide: [`docs/DEVELOPER_TRIAL.md`](docs/DEVELOPER_TRIAL.md) · Cursor MCP: [`docs/AGENT_ENV.md`](docs/AGENT_ENV.md)

---

## Run the gates (recommended order)

```bash
unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon

# 1. Canonical — agent + QA layer (needs OPENAI_API_KEY)
./scripts/gate-autonomous-ux.sh

# 2. Zero-LLM replay on same fixture (no API key)
./scripts/gate-compiled-replay.sh

# 3. Motor / CI acceptance
./scripts/build-fixture.sh
LIGH_APP_N=50 ./scripts/gate-app-reliability.sh

# 4. OSS stretch (optional)
./scripts/gate-xcuitestdemo-bakeoff.sh
LIGH_APP_N=50 ./scripts/gate-third-party-rigor.sh
```

Experimental (research only, do not block merges on these):

```bash
./scripts/gate-ux-graph-prove.sh          # graph A/B — often claim_pass: false
LIGH_UX_ARM=discover ./scripts/gate-autonomous-ux.sh   # also record ux graph
```

---

## Agent tools (MCP)

```bash
./scripts/print-cursor-mcp.sh   # paste into Cursor → Settings → MCP
python3 scripts/ligh_mcp.py     # stdio MCP server
```

Prefer for agents:

| Tool | Purpose |
|------|---------|
| `ligh_perceive` | Read affordances + screen fingerprint |
| `ligh_attempt` | Tap/type + `expect` → `intent_met` + evidence |
| `ligh_find` / `ligh_dismiss` | Scroll target / keyboard-sheet |
| `ligh_cap_app_job` | Known steps end-to-end |

Prompt template: [`docs/CURSOR_PROMPT.md`](docs/CURSOR_PROMPT.md) · [`docs/AGENT.md`](docs/AGENT.md)

---

## Optional: UX graph → compiled replay

Recording screens/transitions during `perceive`/`attempt` is a **side effect**. The one validated use is **compile intent_met edges into motor steps** for zero-LLM replay — not LLM “graph memory.”

See [`docs/UX_GRAPH.md`](docs/UX_GRAPH.md)

---

## Architecture

```text
Coding agent (Cursor + MCP)
        ↓
   ligh / ligh_mcp.py
        ↓
   lighd  — motor, AX, HID, fail-closed capabilities
        ↓
   CoreSimulator → your Debug .app
```

Details: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · motor design: [`docs/HUMAN_MOTOR.md`](docs/HUMAN_MOTOR.md)

---

## Research demos (not the product wedge)

SpringBoard / Messages / Settings loops and WDA microbenches are host experiments — useful for engineering, not the headline:

```bash
./scripts/demo-type-agent.sh      # Messages typing demo
ligh bench agent --steps 40       # vs WDA wall-clock script
```

---

## License

[MIT](LICENSE) — pin your Xcode version; private Apple frameworks apply.

Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md)
