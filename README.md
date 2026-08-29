<p align="center">
  <img src="docs/assets/ligh-messages-demo.gif" alt="LIGH agent opens Messages and types a pitch line" width="320" />
</p>

<h1 align="center">LIGH</h1>

<p align="center">
  <strong>Truth machine for iOS coding agents.</strong><br/>
  Fail-closed certify · structured repair (TRAIL) · scored eval/CI — not another tap MCP.<br/>
  MIT · macOS + Xcode
</p>

<p align="center">
  <a href="#the-bet"><strong>The bet</strong></a> ·
  <a href="#results"><strong>Results</strong></a> ·
  <a href="#agent-scorepack"><strong>Scorepack</strong></a> ·
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#how-it-works"><strong>How it works</strong></a>
</p>

---

## The bet

**Maestro proves durable UI flows. XcodeBuildMCP builds. LIGH proves the agent's Swift change** — with `ok: true` only, structured faults, and optional TRAIL repair.

Sold to people who **cannot vibes-merge**: agent eval harnesses, platforms, and CI on agent-authored PRs.  
Compose with Maestro (E2E partner). Do not compete for “every Cursor user installs another MCP.”

| Buyer | Job |
|-------|-----|
| **Eval / agent platforms** | Frozen scorepack → inject bug → agent or TRAIL → scoreboard |
| **CI (agent PRs)** | Goal certify on critical paths → block merge on `ok: false` |
| Local Mac dogfood | Secondary — [`docs/AGENT_PARADISE.md`](docs/AGENT_PARADISE.md) |

→ Strategy + distance: [`docs/SCOREPACK.md`](docs/SCOREPACK.md) · vs stack: [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md)

---

## What it does

```text
agent edits Swift → build → sim → prove → fault taxonomy → localize/repair → certify
```

- **`ligh_test`** — goal-first verify; always writes `.ligh/last-certify.json`
- **TRAIL** — prove → effect-class localize → ≤2 patches → certify (lab + scorepack)
- **BuildGovernor** — serialize builds, memory backpressure, `infra_oom` (host plane)
- **0 LLM UI tokens** on motor (Autopilot)

Requires Mac + Xcode Simulator.

### Agent Scorepack (start here if you buy the bet)

```bash
./scripts/gate-scorepack.sh --dry-run    # contract + scoreboard schema
./scripts/gate-scorepack.sh              # full TRAIL core pack (OPENAI_API_KEY + Mac)
```

Pack: [`scorepack/v1/manifest.json`](scorepack/v1/manifest.json) · CI: `.github/workflows/ligh-scorepack.yml`

### Local certify (secondary)

```bash
./scripts/ligh-paradise.sh /path/to/MyApp.xcodeproj --build
LIGH_WORKSPACE=/path/to/app ./scripts/ligh-agent-loop.sh
```

**MCP:** `ligh_init` → `ligh_test` → `ligh_viewer` — dogfood, not the wedge.

→ [AGENTS.md](AGENTS.md)

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

**Architecture v5 — two products, same motor** ([`docs/OSS_PIPELINE.md`](docs/OSS_PIPELINE.md)):

| Product | Input | KPI |
|---------|-------|-----|
| **Agent loop** (primary) | workspace / `ligh_init` | time-to-ok per patch |
| **Stranger proof** | Tier B `--app` / prebuilt `.app` | `tier_b_verify_pass` |
| Cold git build | Tier C URL | honest skip/benchmark — build fail ≠ LIGH broken |

```text
HostCapability → preflight_v2(SPM) → [Tier B: no build | Tier C: xcodebuild]
  → EyesReady → process_health → label-first discover → ligh_test
```

| Class | Meaning |
|-------|---------|
| ✓ pass | motor-proven chrome + `ligh_test ok` |
| ⊘ host-skip | `missing_watchos_runtime`, `xcode_format_too_new`, `swift_tools_too_new`, … |
| ✗ host | `sim_boot_hung` / `eyes_unusable` — fix Simulator, not Swift |
| ✗ app | `app_crashed` / `app_not_running` / `discover_no_chrome` / goal fail |
| crash ≠ chrome | recent DiagnosticReports → `app_crashed` (never `discover_no_chrome`) |

**System surfaces (login / ASWebAuth / share / permission):** hit-test occlusion → classify role → `overlay: system_surface`. Motor policy from role table (auth never auto-dismisses). → [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

Proven (label-first, 0 AX ids):

| App | Mode | Chrome | Result |
|-----|------|--------|--------|
| CountriesSwiftUI | stranger | motor label | ✓ |
| Food Truck | stranger | motor label | ✓ |
| **Mastodon** | Tier B `--app` | `Unisciti a mastodon.social` | ✓ `ligh_test` · 10.5s · [artifact](docs/assets/oss-stranger-mastodon-tierb.json) |

```bash
./scripts/gate-oss-stranger-batch.sh    # scripts/oss-stranger-urls.txt
./scripts/gate-oss-stranger-smoke.sh    # Countries + Food Truck only
# Tier B (primary stranger verify — no cold xcodebuild):
python3 scripts/ligh_oss_smoke.py --app /path/to/App.app --bundle-id bid --source-root /path/to/src
```

→ artifact [`oss-stranger-trial-latest.json`](docs/assets/oss-stranger-trial-latest.json) · Mastodon Tier B [`oss-stranger-mastodon-tierb.json`](docs/assets/oss-stranger-mastodon-tierb.json) · contract [`docs/OSS_PIPELINE.md`](docs/OSS_PIPELINE.md) · vs Maestro [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md)


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
