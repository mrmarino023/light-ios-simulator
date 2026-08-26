<p align="center">
  <img src="docs/assets/ligh-messages-demo.gif" alt="LIGH agent opens Messages and types a pitch line" width="320" />
</p>

<h1 align="center">LIGH</h1>

<p align="center">
  <strong>Host-side control plane for coding agents on iOS.</strong><br/>
  Observe · act · verify · repair — on Simulator and physical Expo debug builds.<br/>
  MIT · macOS + Xcode
</p>

<p align="center">
  <a href="#results"><strong>Results</strong></a> ·
  <a href="#apps-under-test"><strong>Apps under test</strong></a> ·
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#connect-cursor"><strong>Cursor</strong></a> ·
  <a href="#expo--physical-iphone"><strong>Expo</strong></a> ·
  <a href="#how-it-works"><strong>How it works</strong></a>
</p>

---

## What it does

Coding agents can edit Swift. They still cannot reliably **use** the app they just changed.

LIGH closes that loop on the host:

```text
write → build → run → interact → verify → fix
```

- **Feel IR** — structured interaction frame (not a screenshot dump)
- **Autopilot** — host reaches UI goals with **0 LLM UI tokens**
- **TRAIL repair** — on failure: trace → localize → ≤2 scoped patches → build → certify

Requires Mac + Xcode Simulator. Works best when the app exposes stable `accessibilityIdentifier`s.

---

## Results

Published gate artifacts under [`docs/assets/`](docs/assets/). Writeup: [`docs/TRAIL_RESULTS.md`](docs/TRAIL_RESULTS.md).

### TRAIL repair

Trace → classify → localize → ≤2 LLM fixes → build → certify. No golden reverse.

| Task | App | Wall | Tokens | Outcome |
|------|-----|------|--------|---------|
| Login never navigates | XCUITestDemo (vendored) | **40s** | 1.8k | verified · `LoginViewModel` |
| Onboarding stuck | OnboardingDemo (frozen) | **64s** | 3.8k | verified · `OnboardingView` |
| Notes tab missing after login | [Kix](https://github.com/byKosta/Kix-app) | **126s** | 7.8k | verified · `MainTabView` |

**3/3 verified** (gate ≥2/3 ≤120s) → [`trail-holy-multi-latest.json`](docs/assets/trail-holy-multi-latest.json)

| Baseline (login) | Wall | Tokens |
|------------------|------|--------|
| Vision chat | 622s | 212k |
| Autopilot + chat loop | 61s | 14k |
| **TRAIL** | **40s** | **1.8k** |

| Baseline (Kix Notes tab) | Wall | Tokens |
|--------------------------|------|--------|
| Vision chat | 460s | 128k |
| Autopilot + chat loop | 644s | 148k |
| **TRAIL** | **126s** | **7.8k** |

Full compare: [`trail-holy-compare-latest.json`](docs/assets/trail-holy-compare-latest.json)
Architecture: [`docs/TRAIL_BULLETPROOF.md`](docs/TRAIL_BULLETPROOF.md)

### Autopilot (zero UI tokens)

One generic Feel-IR policy. **6/6 apps**, six flow shapes. Median ~11.5s. No LLM for taps.

| App | Flow | Wall | Steps | LLM UI tokens |
|-----|------|------|-------|---------------|
| LighFixture | form | 11.5s | 2 | 0 |
| LighOnboard | multi-step wizard | 15.2s | 4 | 0 |
| LighModal | sheet overlay | 9.5s | 2 | 0 |
| LighFeed | list drill-down | 8.2s | 1 | 0 |
| XCUITestDemo | login | 11.8s | 3 | 0 |
| Kix | catalog + auth + tabs | 11.5s | 3 | 0 |

→ [`autopilot-generality-latest.json`](docs/assets/autopilot-generality-latest.json)

Reproduce:

```bash
./scripts/gate-trail-holy-multi.sh          # TRAIL multi-task claim
./scripts/gate-autopilot-generality.sh      # 6-app motor claim (if present)
```

---

## Apps under test

### Open-source / third-party (vendored)

| App | Upstream | What we exercise |
|-----|----------|------------------|
| **Kix** | [byKosta/Kix-app](https://github.com/byKosta/Kix-app) | Login, catalog, tab chrome; Notes-tab repair task |
| **XCUITestDemo** | `fixtures/third-party/XCUITestDemo` (`com.himali.XCUITestDemo`) | Login credentials + navigation repair |

Local notes for Kix: [`fixtures/third-party/Kix/UPSTREAM.md`](fixtures/third-party/Kix/UPSTREAM.md)

### In-repo fixtures (flow shapes)

| App | Path | Flow shape |
|-----|------|------------|
| LighFixture | `fixtures/LighFixture` | Form submit |
| LighOnboard | `fixtures/LighOnboard` | Multi-step wizard |
| LighModal | `fixtures/LighModal` | Sheet / overlay |
| LighFeed | `fixtures/LighFeed` | List → detail |
| OnboardingDemo | `fixtures/frozen/OnboardingDemo` | Blocked overlay / home gate |

These are the surfaces used for Autopilot generality and TRAIL repair gates — not App Store binaries.

---

## Install

**Requirements:** macOS, Xcode + iOS Simulator runtime, [Rust](https://rustup.rs).

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator

# Option A — installer (puts ligh / lighd on PATH)
./scripts/install.sh

# Option B — init (release build + doctor + MCP snippet)
./scripts/ligh-init.sh

# Option C — from source only
unset CARGO_TARGET_DIR
cargo build --release -p ligh-cli -p ligh-daemon
```

Smoke:

```bash
./scripts/developer-trial.sh
```

Homebrew HEAD (from a clone): see [`Formula/README.md`](Formula/README.md).

---

## Connect Cursor

1. Build release binaries (above).
2. Print an MCP config with absolute paths:

```bash
./scripts/print-cursor-mcp.sh
```

3. Paste into **Cursor → Settings → MCP** (or `~/.cursor/mcp.json`).

Example shape:

```json
{
  "mcpServers": {
    "ligh": {
      "command": "python3",
      "args": ["/ABS/PATH/light-ios-simulator/scripts/ligh_mcp.py"],
      "env": { "LIGH_BIN": "/ABS/PATH/light-ios-simulator/target/release/ligh" }
    }
  }
}
```

4. In chat, use the prompt in [`docs/CURSOR_PROMPT.md`](docs/CURSOR_PROMPT.md). Short form:

> Build my Debug Simulator `.app`, then verify with `ligh_cap_app_job` using accessibility identifiers. On `{ ok: false, fault }`, fix source, rebuild, retry. Do not claim success without `ok: true`.

Full developer trial: [`docs/DEVELOPER_TRIAL.md`](docs/DEVELOPER_TRIAL.md)

### Useful MCP tools

| Tool | Job |
|------|-----|
| `ligh_perceive` | Settled world + Feel IR |
| `ligh_cap_autopilot` | Goal + params → path → verified (0 UI tokens) |
| `ligh_cap_repair_job` | TRAIL: prove → localize → fix → build → certify (`task_path`) |
| `ligh_attempt` | Act + host verdict |
| `ligh_cap_app_job` | Scripted wait/tap/type + assert |
| `ligh_perceive_routed` | AX first; vision only if eyes fail |

---

## Expo / physical iPhone

Same agent loop. Different motors.

| | Simulator | Physical (your Debug / Expo build) |
|--|-----------|-------------------------------------|
| Eyes | CoreSimulator AX | DevDriver over LAN (`@mm-labs/ligh-expo`) |
| Hands | IndigoHID | DevDriver → WDA cascade |

```bash
# Vendor the Expo config plugin into your app
./scripts/sync-ligh-expo.sh /path/to/YourExpoApp
```

`app.json`:

```json
{
  "expo": {
    "plugins": ["@mm-labs/ligh-expo"]
  }
}
```

Then rebuild native (`npx expo run:ios` or EAS development). JS reload is not enough after driver changes.

Package docs: [`packages/ligh-expo/README.md`](packages/ligh-expo/README.md)  
Runbook: [`docs/PHYSICAL.md`](docs/PHYSICAL.md)

Or during init:

```bash
./scripts/ligh-init.sh /path/to/YourExpoApp
```

---

## How it works

```text
Cursor MCP
    ↓
lighd  — Autopilot over Feel IR
    ↓
Simulator (CoreSimulator)  or  Physical (DevDriver eyes + WDA hands)
    ↓
Your Debug / Expo app
```

**Feel IR** (what the agent sees):

```json
{
  "place": { "surface": "app", "title": "Welcome" },
  "salience": [
    { "rank": 1, "kind": "primary_button", "label": "Get Started" }
  ],
  "feel": { "phase": "settled", "ready": true },
  "suggest": { "intent": "tap", "label": "Get Started" }
}
```

**TRAIL** (when a goal fails):

```text
TraceFailure → hybrid localize → constrained fix → build → certify
```

Architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

---

## License

[MIT](LICENSE)
