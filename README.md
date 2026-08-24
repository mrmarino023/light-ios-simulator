<p align="center">
  <img src="docs/assets/ligh-messages-demo.gif" alt="LIGH agent opens Messages and types a pitch line" width="320" />
</p>

<h1 align="center">LIGH</h1>

<p align="center">
  <strong>Make coding agents actually use the iOS apps they build.</strong><br/>
  Local iOS Simulator + physical Expo/Debug · open source (MIT) · macOS + Xcode
</p>

<p align="center">
  <a href="#try-it"><strong>Try it</strong></a> ·
  <a href="#how-it-works"><strong>How it works</strong></a> ·
  <a href="#physical-iphone--expo"><strong>Physical / Expo</strong></a> ·
  <a href="#what-we-measured"><strong>Evidence</strong></a> ·
  <a href="docs/DEVELOPER_TRIAL.md"><strong>Your app</strong></a>
</p>

---

## The problem

AI coding agents can write Swift. **Getting them to actually run and verify what they built on the Simulator is still painfully slow.**

The goal is not another iOS simulator. It is to make Apple's existing Simulator a **much better execution environment for coding agents**:

```text
write → build → run → interact → verify → fix
```

Tell Cursor:

> *"Add validation to the signup form and verify it works in the Simulator."*

LIGH gives the agent a local control plane: persistent `lighd` on CoreSimulator, accessibility JSON for observe/act, and structured pass/fail results.

**Requires** a Mac with Xcode Simulator. **Works best** with `accessibilityIdentifier` on your views.

---

## Positioning

### One sentence

**LIGH is a local iOS execution substrate for coding agents:** the model fixes
Swift, and the host autonomously runs, uses and verifies the app on Simulator.

### What LIGH is

- A **persistent local control plane** on top of CoreSimulator (`lighd`)
- A **host Autopilot** that takes a goal plus typed data, discovers the UI path
  at runtime, and spends **zero LLM UI tokens**
- A **strict verifier** that fail-closes and accepts a working patch
- A system optimized for the real coding-agent loop:

```text
read → edit → build → run → use → verify → fix
```

### What LIGH is not

- Not a generic WebDriver replacement for every mobile automation job
- Not automation of unmodified App Store apps (physical path needs **your**
  Debug / Expo development build)
- Not a recorder or YAML-flow authoring tool
- Not an LLM memory layer over app screens
- Not a cloud device farm

### Why this exists

The hard part is not making a model write Swift. The hard part is making it
**use the app it just built quickly enough to stay inside a debugging loop**.

That is the wedge:

- traditional mobile automation optimizes for **test authoring + CI**
- LIGH optimizes for **local fix → run → verify for coding agents**

### Honest competitive position

- **vs Appium / WDA:** LIGH is narrower, but much better aligned with local
  coding-agent execution. Appium wins on breadth, language ecosystem and general
  automation; LIGH wins when the job is “fix the app and prove the fix on the
  simulator now.” Our execution-layer benchmark is ~4.7× faster on the same
  semantic workflow.
- **vs Maestro / Maestro MCP:** Maestro is currently the strongest adjacent
  competitor for agentic mobile QA. It is excellent when the deliverable is a
  **repeatable YAML test flow** you keep in CI. LIGH is stronger when the
  deliverable is a **working code fix** and the main bottleneck is the agent's
  local execution loop. The host Autopilot removes UI micro-decisions from the
  model instead of generating a persistent scripted flow.
- **vs XCUITest / Detox / Espresso:** those are test frameworks, not agent
  control planes. They are great when humans write and maintain tests. LIGH is
  for the different job where an agent must inspect the app, change source, run
  it, and verify the result autonomously.

### The sharp claim

LIGH should be read as:

> the fastest honest way to let a coding agent locally use and verify the iOS
> app it is actively changing

not as:

> the best general-purpose mobile test framework

That broader claim would be false.

Milestone note: [`docs/MILESTONE_HOST_AUTOPILOT.md`](docs/MILESTONE_HOST_AUTOPILOT.md).
Validation week (do not change the architecture): [`docs/VALIDATION_WEEK.md`](docs/VALIDATION_WEEK.md) —
run `./scripts/validation-week.sh`.

---

## What we measured

### Execution layer — observe → act → verify

Same 44-step semantic workflow (Settings → search → assert → screenshot, ×4 cycles):

| | LIGH (`lighd`) | WDA / Appium |
|--|----------------|--------------|
| Wall time | **~10.6 s** | **~50 s** |
| Steps | 44 / 44 | 44 / 44 |
| Failures | 0 | 0 |

~**4.7× faster** than WDA/Appium on the same workflow. Evidence: [`docs/assets/agent-bench-latest.json`](docs/assets/agent-bench-latest.json).

Reproduce: `ligh agent-bench` (WDA baseline needs Appium listening).

### Coding-agent loop — two protocols

**Product path** (host `exercise_app`): OnboardingDemo. Agent fixes Swift; host runs known taps. This measures *edit + host exercise*, not AX-vs-vision.

| Arm | Pass | Wall | LLM tokens |
|-----|------|------|------------|
| **LIGH (AX + host exercise)** | yes | **~86 s** | **~27k** |
| Vision baseline | yes | ~204 s | ~73k |
| Hybrid (AX→vision) | no | ~334 s | ~402k |

Evidence: [`docs/assets/killer-loop-ab-latest.json`](docs/assets/killer-loop-ab-latest.json). Reproduce: `./scripts/gate-killer-loop.sh`.

**Historical honest A/B v1** (no host exercise): XCUITestDemo
`login-never-navigates` — same prompt, agent must type/tap via AX **or** vision.
`exercise_app` disabled. Both arms **failed** this run (postcondition not met);
neither modality won.

| Arm | Pass | Wall | LLM tokens | Used exercise_app |
|-----|------|------|------------|-------------------|
| LIGH (AX) | no | ~369 s | ~445k | no |
| Vision baseline | no | ~333 s | ~306k | no |

Evidence: [`docs/assets/killer-loop-ab-honest-latest.json`](docs/assets/killer-loop-ab-honest-latest.json). Reproduce: `LIGH_KILLER_HONEST=1 ./scripts/gate-killer-loop-ab.sh` (needs `OPENAI_API_KEY`).

This v1 result identified the architectural bug: the LLM was still the UI
executor. It is retained as the negative baseline, not presented as a win.

**Honest A/B v2 — Host Autopilot.** Same XCUITestDemo task, injected bug,
model, acceptance target and strict harness. Neither arm receives a step list.
The Autopilot arm restricts the LLM to code (`read/write/build/run_goal`);
Rust discovers and drives the UI path from live Feel IR. Vision still drives
every tap through the LLM.

| Arm | Pass | Wall | LLM tokens | Patches / builds |
|-----|------|------|------------|------------------|
| **LIGH Host Autopilot** | yes | **41.9 s** | **9,034** | 1 / 1 |
| Vision baseline | yes | 152.4 s | 67,040 | 1 / 1 |

That paired run is **3.64× faster wall-clock** and uses **7.42× fewer LLM
tokens**. The UI executor itself used zero LLM tokens. Evidence:
[`docs/assets/killer-loop-ab-v2-latest.json`](docs/assets/killer-loop-ab-v2-latest.json).
Reproduce with `./scripts/gate-killer-loop-ab-v2.sh` (needs
`OPENAI_API_KEY`). The artifact is published regardless of outcome and only
passes when Autopilot both verifies and reaches the 3× threshold.

The same policy also passes the no-special-cases generality gate on **6/6
apps**, covering six different flow shapes:

- **LighFixture** — form: type + submit, 2 actions, 11.5 s
- **LighOnboard** — multi-screen wizard, 4 actions, 14.2 s
- **LighModal** — sheet presentation + confirmation, 2 actions, 10.0 s
- **LighFeed** — list → detail navigation, 1 action, 9.4 s
- **XCUITestDemo** — third-party OSS login with credentials, 3 actions, 11.2 s
- **Kix** — third-party catalog + auth + tabs, 3 actions, 12.7 s

Kix was the hole: login worked, then Autopilot wandered catalog cards because
SwiftUI tab bars walk as a childless `AXGroup` and XCTest ids like `tab_home`
show up in AXP as the SF Symbol plus the visible label (`house.fill` / `Home`).
The host now hit-tests childless tab/nav/tool bars and binds `tab_*` goal ids
to tab-chrome labels only. Reproduce Kix with
`LIGH_PILOT_APPS=kix ./scripts/gate-autopilot-generality.sh`.

There are no per-app branches or recorded flows in Autopilot. Every run receives
only an acceptance goal plus typed data; Rust discovers the path at runtime and
uses **zero LLM UI tokens**. Evidence:
[`docs/assets/autopilot-generality-latest.json`](docs/assets/autopilot-generality-latest.json).
Reproduce with `./scripts/gate-autopilot-generality.sh`.

---

## Physical iPhone + Expo

Same agent loop as Simulator. Different motors.

```text
lighd
 ├─ eyes  → @mm-labs/ligh-expo DevDriver (in-app AX over LAN)
 └─ hands → WDA / Appium XCUITest (system taps/swipes)
              fail-closed on screen_sig (ACK without ΔUI = lie)
```

| | Simulator | Physical (owned Debug / Expo dev client) |
|--|-----------|------------------------------------------|
| Eyes | CoreSimulator AX | DevDriver AX dump |
| Hands | IndigoHID | **WDA** (in-app fake UITouch is lab-only) |
| Proof law | motor effect checks | `effect: ok` requires `screen_sig` change |

**Proven on device (Mae Expo app):** tap Profile → Home with
`motor: physical` + `effect: ok`, plus WDA swipe. Full runbook:
[`docs/PHYSICAL.md`](docs/PHYSICAL.md).

Wire any Expo app:

```bash
./scripts/sync-ligh-expo.sh /path/to/YourExpoApp
# app.json plugins: ["@mm-labs/ligh-expo"]
# then: EAS / expo run:ios development build
```

```bash
cp scripts/wda.env.example ~/.ligh/wda.env   # UDID, bundle, team
./scripts/start-appium-wda.sh                # keep running
./target/release/lighd &
./target/release/ligh device wait
./target/release/ligh tap --json --label 'TabProfile'
```

Package docs: [`packages/ligh-expo/README.md`](packages/ligh-expo/README.md).

Host Autopilot ×3 evidence below is **Simulator-scoped** until Autopilot is
re-gated on the physical WDA motor.

---

## How it works

```text
Coding agent (Cursor MCP)
        ↓
LIGH host — Autopilot over Feel IR (perceive → plan → act → verify)
        ↓
  ┌─────┴─────┐
  │           │
CoreSimulator  Physical HybridPhysical
(IndigoHID)    (DevDriver eyes + WDA hands)
  │           │
Your Debug .app / Expo development build
```

### Three representations (only one is the product wedge)

| | What it is | Who uses it | Role |
|--|------------|-------------|------|
| **Screenshot** | Pixels | Vision LLMs | Fallback when AX is unusable |
| **AX tree** | Raw accessibility dump | Debug / motor | Too big and noisy for planning |
| **Feel IR** | Live interaction frame | Host + thin agent | Default world model |

**Feel IR** is not a screenshot and not a dump of the tree. After every settle, Rust builds a small JSON frame:

```text
place     → where you are (fingerprint, surface, title)
salience  → what weighs (ranked CTAs / fields, top-N)
block     → what blocks (keyboard, alert, sheet)
delta     → what just changed (fp changed? events?)
feel      → phase: settled | transition | blocked | eyes_unusable
suggest   → optional next host act (tap label/id or dismiss)
```

Example shape (agent sees this from `ligh_perceive`):

```json
{
  "place": { "fingerprint": "fp_ab08…", "surface": "app", "title": "Welcome" },
  "salience": [
    { "rank": 1, "kind": "primary_button", "label": "Get Started" },
    { "rank": 2, "kind": "button", "label": "Skip" }
  ],
  "block": null,
  "feel": { "phase": "settled", "keyboard": false, "ready": true },
  "suggest": { "intent": "tap", "label": "Get Started" }
}
```

### Who decides what

```text
LLM (slow, expensive)     →  read/edit Swift, decide what code to fix
Rust Autopilot            →  discover and drive the UI path, verify the goal
Feel IR                   →  the host's live interaction state (~ms update)
strict harness            →  accept/reject the patch without another LLM turn
```

Canonical coding-agent loop on LIGH:

```text
read_file → write_file → build_app → run_goal → host_accept
```

- **`run_goal`** receives an acceptance target plus typed data, never a step list. Rust discovers the path from live Feel IR and drives it with zero LLM UI tokens.
- **`host_accept`** immediately runs the strict harness when the target appears. A passing patch ends the loop before the model can rewrite working code.
- The planner is app-agnostic: field kinds, CTA salience, overlays, deltas and bounded recovery. Per-app flows are forbidden.
- Screenshots are **debug / escalation only** (`ligh_perceive_routed`: AX → ready retry → vision if still `eyes_unusable`).

### What we tried and rejected

A persistent **UX graph** (screens + transitions as LLM memory) was measured and **did not help** agents navigate — they ignored it or used more tokens ([`docs/UX_GRAPH.md`](docs/UX_GRAPH.md)). Feel IR is the opposite design: a **live frame for the computer**, not a history document for the model. The graph remains useful as telemetry / compile-to-replay input (`llm_tokens = 0`), not as agent memory.

### Agent tools (MCP)

| Tool | Job |
|------|-----|
| `ligh_perceive` | Settled world model + **Feel IR** |
| `ligh_attempt` | Act + host verdict (`intent_met`, evidence) |
| `ligh_perceive_routed` | AX-first; vision only on escalation |
| `ligh_cap_autopilot` | Goal + typed data → host-discovered path → verified result (0 UI tokens) |
| `ligh_cap_app_job` | Known multi-step job (CI / fixtures) |

More detail: [`docs/QA_LAYER.md`](docs/QA_LAYER.md) · [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · [`docs/STRUCTURED_CONTROL.md`](docs/STRUCTURED_CONTROL.md)

---

## Try it

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator
./scripts/install.sh
unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon
./scripts/developer-trial.sh
```

Paste MCP config from `./scripts/print-cursor-mcp.sh` into **Cursor → Settings → MCP**.

Full guide: [`docs/DEVELOPER_TRIAL.md`](docs/DEVELOPER_TRIAL.md)

---

## License

[MIT](LICENSE)
