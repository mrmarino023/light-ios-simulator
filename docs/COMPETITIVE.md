# LIGH vs competitors — brutal honest map

Last updated: 2026-08-27

## One line

**Maestro proves the UI. LIGH proves your agent's code change** — verify + localize + fix + certify.

---

## Competitor matrix

| | **Maestro MCP** | **XcodeBuildMCP** | **ios-simulator-mcp** | **Vision baseline** | **LIGH** |
|--|-----------------|-------------------|----------------------|---------------------|----------|
| **Primary job** | Write/run E2E flows | Build + test Xcode | Drive sim | Screenshot → guess | **Verify + repair agent edits** |
| **Agent entry** | `inspect_screen`, `run` | 79 MCP tools | tap/swipe/screenshot | chat + vision | **`ligh_test`** (goal-first) |
| **Live viewer** | Maestro Viewer ✅ | ❌ | screenshots | screenshots | **`ligh_viewer`** ✅ (new) |
| **Cross-platform** | iOS + Android + web | Apple only | iOS sim | any | **iOS sim** (+ partial Expo) |
| **Saved flows** | YAML in repo ✅ | ❌ | ❌ | ❌ | **`.ligh/app-goal.json`** ✅ |
| **Structured faults** | YAML errors | build JSON | raw | none | **`motor_no_effect`, repair_contract** ✅ |
| **Autonomous repair** | ❌ | ❌ | ❌ | retry forever | **TRAIL** ✅ (unique) |
| **UI token cost** | inspect + author flows | high MCP schema tax | medium | **very high** | **0 on motor** ✅ |
| **Needs AX ids** | flexible (text/labels) | n/a | partial | no | **no — label-first discover** ✅ |
| **Cloud CI** | Maestro Cloud ✅ | ❌ | ❌ | varies | **GitHub Action** (starter) |

---

## When agents should pick LIGH

- Agent **edits Swift** and must **prove + fix** in the same session
- You care about **token cost** and **fail-closed** semantics
- Bug class: login gate, missing tab, disabled control, stuck overlay

## When agents should pick Maestro

- Cross-platform E2E suite in repo
- QA team owns YAML flows
- Need Maestro Cloud + Viewer polish today

## When agents should pick XcodeBuildMCP

- Build/test/debug loop only — no verify semantics needed

---

## LIGH product surface (agent paradise)

```text
ligh_init(path)     → .ligh/ audit + app-goal.json
ligh_test()         → verify (PRIMARY)
ligh_viewer()       → browser sim (debug)
ligh_cap_repair_job → TRAIL when test proves bug
```

Shell: `./scripts/ligh-paradise.sh` · `./scripts/ligh-test.sh`

---

## Still missing (honest)

- Hosted Mac runner (monetization path)
- Android
- Repair fully in Rust (Python harness on hot path)
- 10+ stranger-repo trials published

**OSS stranger proof (2026-08-27):** 2/2 iOS apps with **0 accessibility ids** — CountriesSwiftUI + Apple Food Truck — `ligh_test ok:true` via label-first discover (`docs/assets/oss-stranger-trial-latest.json`).

**CI:** every push/PR runs [`.github/workflows/oss-stranger-smoke.yml`](../.github/workflows/oss-stranger-smoke.yml) → `./scripts/gate-oss-stranger-smoke.sh` (clone → build → discover → `ligh_test`).

See [`AGENT_PARADISE.md`](AGENT_PARADISE.md) roadmap.
