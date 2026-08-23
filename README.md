<p align="center">
  <img src="docs/assets/ligh-messages-demo.gif" alt="LIGH agent opens Messages and types a pitch line" width="320" />
</p>

<h1 align="center">LIGH</h1>

<p align="center">
  <strong>Make coding agents actually use the iOS apps they build.</strong><br/>
  Local iOS Simulator · open source (MIT) · macOS + Xcode
</p>

<p align="center">
  <a href="#try-it"><strong>Try it</strong></a> ·
  <a href="#how-it-works"><strong>How it works</strong></a> ·
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

## What we measured

Same 44-step semantic workflow (Settings → search → assert → screenshot, ×4 cycles):

| | LIGH (`lighd`) | WDA / Appium |
|--|----------------|--------------|
| Wall time | **~10.6 s** | **~50 s** |
| Steps | 44 / 44 | 44 / 44 |
| Failures | 0 | 0 |

~**4.7× faster** than WDA/Appium on the same workflow. Evidence: [`docs/assets/agent-bench-latest.json`](docs/assets/agent-bench-latest.json).

Reproduce: `ligh agent-bench` (WDA baseline needs Appium listening).

That is the **execution layer**. The open question is whether it helps a coding agent **modify, rebuild, and verify** a real app end-to-end — try it on yours.

---

## How it works

```text
Coding agent (Cursor MCP)
        ↓
LIGH — persistent host + perceive/attempt
        ↓
Apple CoreSimulator
        ↓
Your Debug .app
```

Agents use **`ligh_perceive`** (accessibility tree as JSON) and **`ligh_attempt`** (act + check).

More: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · [`docs/QA_LAYER.md`](docs/QA_LAYER.md) · [`docs/STRUCTURED_CONTROL.md`](docs/STRUCTURED_CONTROL.md)

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
