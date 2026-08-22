# Structured iOS control for coding agents (local Simulator)

LIGH is a **local Simulator control plane for coding agents** — drive **your Debug `.app`** with settle-honest capabilities. Not computer vision, not cloud.

**Wedge claim:** `ligh cap app-job` (install → launch → `ensure_path` → act → assert) works **every time** on a fixture Debug build. Screenshots are debug only.

**One-liner:** Local Simulator hands for coding agents: accessibility identifiers in, motor taps/types out, fail-closed when overlays or eyes block the path.

Architecture: [`CONTROL.md`](CONTROL.md).

## Prove it

```bash
./scripts/build-fixture.sh
./scripts/gate-app-reliability.sh          # must publish claim_pass
./scripts/gate-app-bakeoff.sh              # same job vs Maestro when installed
```

Published: [`assets/app-reliability-latest.json`](assets/app-reliability-latest.json).

Against your app: set `LIGH_APP_PATH`, `LIGH_APP_BUNDLE_ID`, and `LIGH_APP_*_ID` accessibility identifiers — then the same gate.

## Product surface (MCP)

```bash
./scripts/print-cursor-mcp.sh
```

Tools include `ligh_ready`, `ligh_cap_run_app`, `ligh_cap_tap`, `ligh_cap_type`, `ligh_cap_wait_label`, `ligh_observe` (fail-closed), `ligh_screenshot` (debug).

## Control law

```text
ensure_ready → resolve(id|label) → ensure_path (clear overlay) → fire → settle → assert
```

Typing may raise the keyboard; clearing it is the **next** act’s `ensure_path` — not a type side-effect.

## Competitive stance

| Us | Them |
|----|------|
| Live control plane + `FaultClass` | Maestro: YAML flows + MCP |
| Persistent `lighd` hot path | Appium/WDA: WebDriver weight |
| Identifier-first Debug.app jobs | XCUITest: great tests, not agent MCP |

Fair bakeoff: [`gate-app-bakeoff.sh`](../scripts/gate-app-bakeoff.sh) — same fixture job. Publish losses.

## Secondary evidence (not the wedge)

| Gate | Notes |
|------|-------|
| Scripted vs WDA ~4× | Fixed script, system apps |
| Settings/Messages LLM | Narrow; host settle co-designed |
| Frontier vs vision | Research; Maps can lose |

## Out of scope

Arbitrary unlabeled / game UIs · cloud farms · physical fleets · OCR-as-product

See [`AGENT.md`](AGENT.md) · [`OBSERVE.md`](OBSERVE.md).
