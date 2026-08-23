# Developer trial — use LIGH with Cursor on your app

**Goal:** find out if LIGH helps *your coding agent* verify a Debug `.app` — vs what you already do (simctl, screenshots, vision, other MCP). Not our benchmarks.

**Segment we care about:** agents that modify Swift → build → interact → verify → fix on failure. Not human-written Maestro flows (though Maestro A/B is optional).

Time: ~15–30 minutes first run (includes Rust build).

## 1. Install

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator
./scripts/install.sh          # or: unset CARGO_TARGET_DIR && cargo build --release
./scripts/developer-trial.sh  # smoke + MCP snippet
```

## 2. Cursor MCP

Paste output from `./scripts/print-cursor-mcp.sh` into **Cursor → Settings → MCP**.

Or run `./scripts/print-comparison-mcp.sh` for optional **LIGH vs Maestro** (QA footnote).

Primary comparison: **LIGH MCP vs your usual agent stack** (simctl, screenshot + vision, other MCP).

## 3. Agent prompt (copy into Cursor chat)

See [`CURSOR_PROMPT.md`](CURSOR_PROMPT.md). Short version:

> You have LIGH MCP on this Mac. Build my iOS app for Simulator, then verify it with `ligh_cap_app_job`. Use accessibility identifiers in steps. If you get `{ ok: false, fault, detail }`, use the fault to fix the app and retry. Do not guess — fail-closed only.

Replace “my iOS app” with your project path and bundle id.

## 4. Your app-job

Define steps your agent should run (wait/tap/type + final assert):

```json
[
  {"op":"wait","id":"YourScreenId"},
  {"op":"tap","id":"YourButtonId"},
  {"op":"wait","id":"YourDoneId"}
]
```

Run once from terminal:

```bash
ligh cap app-job /path/to/YourApp.app \
  --bundle-id com.you.app \
  --steps '[{"op":"wait","id":"..."}]'
```

## 5. Compare with your baseline

Same agent, same task, same app:

1. **Arm A** — LIGH MCP + [`CURSOR_PROMPT.md`](CURSOR_PROMPT.md)
2. **Arm B** — your usual stack (simctl + screenshot + vision, ios-mcp, etc.)

Record what worked, what failed, and whether you'd keep LIGH installed.

## 6. Optional — Maestro A/B

Same task, two MCP servers (`print-comparison-mcp.sh`). Useful if you write UI tests by hand — not the main wedge proof.

Maestro flow example: `fixtures/third-party/XCUITestDemo/maestro-job.yaml`

## 7. Feedback (please)

```bash
./scripts/developer-feedback.sh
```

Or edit and send `docs/assets/developer-feedback-TEMPLATE.json`.

**We care about:**

- Did the agent understand LIGH faults?
- How does your agent interact with Simulator **today**? (screenshot loop? XCTest? nothing?)
- Would you use LIGH instead of that baseline for agent verify/fix loops?
- Where did it break (install, AX, your app, MCP, agent confusion)?
- One sentence: why / why not

## What we are NOT asking you to do

- Compare latency charts
- Run N=50 gates
- Use our XCUITestDemo unless you have no app handy
