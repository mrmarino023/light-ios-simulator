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
- **TRAIL repair** — [`repair_engine.py`](scripts/repair_engine.py): classify → KB localize → structural ops → ≤2 LLM patches → certify (same path for every OSS app)

Requires Mac + Xcode Simulator. Works with **label-first discover** when apps have no `accessibilityIdentifier`s (and better still when they do).

### Agent paradise (start here)

```bash
./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj --build
LIGH_WORKSPACE=/path/to/app ./scripts/ligh-test.sh      # goal-first verify
```

**MCP:** `ligh_init` → `ligh_test` → `ligh_viewer` · vs Maestro: [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md)

→ [Agent paradise guide](docs/AGENT_PARADISE.md) · [AGENTS.md](AGENTS.md)

---

## Results

Same job for every column: **a bug is injected into a real iOS app → the agent must fix the Swift and prove the fix in Simulator.**  
Wall = time to verified fix. Tokens = LLM tokens burned. ✓/✗ = postconditions passed.

### What we compare

| Stack | Plain English |
|-------|----------------|
| **Vision LLM agent** | What people do today: screenshots → LLM decides taps → LLM edits code in a long chat. No structured host repair. |
| **Chat agent + LIGH taps** | Coding agent still repairs in unconstrained chat, but uses LIGH Autopilot for UI instead of screenshots. Shows that **better taps alone are not enough**. |
| **LIGH (TRAIL)** | Full LIGH repair path: host proves the failure → finds the file → ≤2 scoped LLM patches → rebuild → certify on the same flow. |

Artifact: [`docs/TRAIL_RESULTS.md`](docs/TRAIL_RESULTS.md) · [`trail-holy-compare-latest.json`](docs/assets/trail-holy-compare-latest.json)

### Head-to-head (repair)

| Bug | Vision LLM agent | Chat agent + LIGH taps | **LIGH (TRAIL)** |
|-----|------------------|------------------------|------------------|
| Login never navigates (XCUITestDemo) | 622s · 212k tokens · ✗ failed | 61s · 14k · ✓ | **33s · 1.3k · ✓** |
| Notes tab missing ([Kix](https://github.com/byKosta/Kix-app)) | 460s · 128k · ✓ | 644s · 148k · ✓ | **78s · 0 · ✓** |
| Onboarding stuck (OnboardingDemo) | *(no vision A/B yet)* | *(no A/B yet)* | **64s · 4.4k · ✓** |

**Read the Kix row:** vision burned **~8 minutes / 128k tokens**. Chat+taps was worse. **LIGH** restored the omitted tab from the broken tree (View type still present) and verified in **~78s / 0 LLM tokens**.

| | LIGH vs Vision LLM | LIGH vs Chat+taps |
|--|--------------------|-------------------|
| Login wall | **~19× faster** (vision never verified) | **~1.9× faster** |
| Kix wall | **~5.9× faster** | **~8× faster** |
| Login tokens | **~160× fewer** | **~10× fewer** |

### LIGH repair runs (absolute)

| Bug | App | Wall | Tokens | File localized |
|-----|-----|------|--------|----------------|
| Login never navigates | XCUITestDemo | **33s** | 1.3k | `LoginViewModel.swift` |
| Onboarding stuck | OnboardingDemo | **64s** | 4.4k | `OnboardingView.swift` |
| Notes tab missing | Kix | **78s** | 0 | `MainTabView.swift` |

**3/3 verified ≤120s (L2\* suite)** → [`trail-holy-multi-latest.json`](docs/assets/trail-holy-multi-latest.json)  
How it works: [`docs/TRAIL_BULLETPROOF.md`](docs/TRAIL_BULLETPROOF.md) (architecture for every OSS app — not per-task patches)

### Architecture (all OSS apps)

Same [`repair_engine.py`](scripts/repair_engine.py) for every vendored app — no task-id modes, no filename priors:

```text
broken tree → StructuralKB → classify → causal localize → operators → LLM (if miss) → certify
```

Refuse unknown effects rather than edit the wrong file.

### Stranger OSS apps (0 accessibility ids)

Label-first paradise on **unrelated** public repos — no AX ids, no per-app patches. Discover proves chrome with motor `wait-label`, then `ligh_test` goal-first.

| App | Repo | Ids | Proven chrome | `ligh_test` |
|-----|------|-----|---------------|-------------|
| CountriesSwiftUI | [nalexn/clean-architecture-swiftui](https://github.com/nalexn/clean-architecture-swiftui) | 0 | `Countries` | ✓ |
| Food Truck | [apple/sample-food-truck](https://github.com/apple/sample-food-truck) | 0 | `Donuts` | ✓ |

**2/2** → [`oss-stranger-trial-latest.json`](docs/assets/oss-stranger-trial-latest.json) · CI: [`.github/workflows/oss-stranger-smoke.yml`](.github/workflows/oss-stranger-smoke.yml) (`./scripts/gate-oss-stranger-smoke.sh`) · competitive map: [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md)

### LIGH Autopilot only (UI goals — not repair)

Separate claim: reach a UI goal with **0 LLM tokens for taps** (host policy on accessibility). This is *not* the repair table above.

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
./scripts/gate-trail-holy-multi.sh          # L2* regression suite
./scripts/gate-autopilot-generality.sh      # UI-goal motor claim
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
