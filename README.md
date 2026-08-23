<p align="center">
  <img src="docs/assets/ligh-messages-demo.gif" alt="LIGH agent opens Messages and types a pitch line" width="320" />
</p>

<h1 align="center">LIGH</h1>

<p align="center">
  <strong>Agent, test the app for me.</strong><br/>
  Local iOS Simulator · open source (MIT) · macOS + Xcode · experimental
</p>

<p align="center">
  <a href="#try-it"><strong>Try it</strong></a> ·
  <a href="#how-it-works"><strong>How it works</strong></a> ·
  <a href="docs/DEVELOPER_TRIAL.md"><strong>Your app</strong></a> ·
  <a href="docs/HONEST.md"><strong>Honest status</strong></a>
</p>

---

## What you wanted

You have a coding agent that edits your Swift. You want it to **open the app, try the flow, and tell you if it worked** — without screenshot roulette and without the model faking success.

Tell Cursor:

> *"Add validation to the signup form and verify it works in the Simulator."*

LIGH gives the agent a local way to do that: launch your Debug `.app`, read the **accessibility tree** as JSON, tap/type, and return **verified or a clear failure**.

**Works best** if your views have `accessibilityIdentifier`. **Requires** a Mac with Xcode Simulator. **Not proven yet** that developers prefer this over XCUITest or vision — we're looking for people to try it.

---

## Try it

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator
./scripts/install.sh
unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon
./scripts/developer-trial.sh
```

Paste MCP config from `./scripts/print-cursor-mcp.sh` into **Cursor → Settings → MCP**. Then ask your agent to build and verify your app.

Full guide: [`docs/DEVELOPER_TRIAL.md`](docs/DEVELOPER_TRIAL.md)

---

## How it works

```text
Agent edits Swift → builds .app → LIGH launches Simulator
→ agent reads UI (accessibility JSON, not pixels)
→ agent taps/types → LIGH checks the result → fix or fail
```

Persistent local daemon (`lighd`) on top of Apple's CoreSimulator. Agents mainly use **`ligh_perceive`** (what's on screen) and **`ligh_attempt`** (act + verify). MCP: [`scripts/ligh_mcp.py`](scripts/ligh_mcp.py).

**How does LIGH read the screen?** Accessibility tree, primarily — structured JSON from the Simulator. Screenshots are optional debug, not the core loop.

More: [`docs/QA_LAYER.md`](docs/QA_LAYER.md) · [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

---

## What we know (short)

| | |
|---|---|
| Agent can test a fixture app with verification | Yes — [evidence](docs/assets/autonomous-ux-latest.json) |
| Works on one OSS app we didn't build | One login flow — [evidence](docs/assets/third-party-rigor-latest.json) |
| External developers want this | **Don't know** — [we need you to try](docs/DEVELOPER_TRIAL.md) |

Benchmarks, gates, Maestro comparisons, dirty-state N=50 → [`docs/HONEST.md`](docs/HONEST.md) (engineering validation, not the pitch).

Experimental: compiled zero-LLM replay, UX graph — [`docs/UX_GRAPH.md`](docs/UX_GRAPH.md). Graph-as-LLM-memory was **disproven**; we don't sell it.

---

## License

[MIT](LICENSE)
