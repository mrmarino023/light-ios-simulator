# LIGH vs competitors — brutal honest map

Last updated: 2026-08-27

## One line

**Maestro proves the UI. LIGH proves your agent's code change** — verify + localize + fix + certify — on **any stranger iOS repo** with one pipeline.

---

## Competitor matrix

| | **Maestro MCP** | **XcodeBuildMCP** | **ios-simulator-mcp** | **Vision baseline** | **LIGH** |
|--|-----------------|-------------------|----------------------|---------------------|----------|
| **Primary job** | Write/run E2E flows | Build + test Xcode | Drive sim | Screenshot → guess | **Verify + repair agent edits** |
| **Agent entry** | `inspect_screen`, `run` | 79 MCP tools | tap/swipe/screenshot | chat + vision | **`ligh_test`** (goal-first) |
| **Stranger repo** | hand-authored YAML | build only | manual | flaky | **URL → HostCapability → discover → certify** |
| **0 AX ids** | text match | n/a | partial | vision | **label-first motor-proven chrome** |
| **Live viewer** | Maestro Viewer ✅ | ❌ | screenshots | screenshots | **`ligh_viewer`** ✅ |
| **Structured faults** | YAML errors | build JSON | raw | none | **`motor_no_effect`, host skips, repair_contract** |
| **Autonomous repair** | ❌ | ❌ | ❌ | retry forever | **TRAIL** ✅ |
| **UI token cost** | inspect + author flows | high MCP schema tax | medium | **very high** | **0 on motor** ✅ |
| **Cloud CI** | Maestro Cloud ✅ | ❌ | ❌ | varies | GitHub Action starter |

---

## OSS stranger architecture (the competitive wedge)

No per-app scheme/label/bundle maps. Same stages for every URL:

```text
HostCapability (Xcode objectVersion, watchOS, disk, iOS runtime)
  → acquire (git|zip, optional #subtree for monorepos)
  → recursive scored find_xcodeproj + pick_scheme
  → gate_project (skip: missing_watchos_runtime | xcode_format_too_new | disk)
  → xcodebuild Debug sim (DerivedData purged after copy)
  → EyesReady (ligh ready — refuse discover on empty AX)
  → label-first live discover (motor wait-label proves chrome)
  → ligh_test goal-first  →  ok:true only
```

| Class | Meaning |
|-------|---------|
| `ok` | motor-proven chrome + goal postconditions |
| `missing_watchos_runtime` | pbx has Watch product; host has no watchOS — **skip**, don't thrash |
| `xcode_format_too_new` | objectVersion > host max — **skip** |
| `app_crashed` | process dead + recent DiagnosticReports — **not** discover_no_chrome; open `.ips` |
| `app_not_running` | expected bundle absent from sim launchctl |
| `discover_no_chrome` | app alive, eyes ok, still no motor-proven chrome — refuse `REPLACE_ME` |
| `eyes_unusable` | recover once, then fail-closed |

**Agent-speed eyes (system surfaces):** hit-test occlusion → classify role (auth/share/permission) → `overlay: system_surface`. Motor policy from role table (auth never auto-dismisses).

**Reproduce:** `./scripts/gate-oss-stranger-batch.sh` · URLs: `scripts/oss-stranger-urls.txt` · API: `scripts/ligh_oss_smoke.py` · contract: [`OSS_PIPELINE.md`](OSS_PIPELINE.md) · artifact: [`oss-stranger-trial-latest.json`](assets/oss-stranger-trial-latest.json)

---

## When agents should pick LIGH

- Agent **edits Swift** and must **prove + fix** in the same session
- You care about **token cost** and **fail-closed** semantics
- You want **stranger OSS** onboarding without writing Maestro YAML first
- Bug class: login gate, missing tab, disabled control, stuck overlay

## When agents should pick Maestro

- Cross-platform E2E suite in repo
- QA team owns YAML flows
- Need Maestro Cloud polish today

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
- 5+ stranger `ligh_test ok` on every host Xcode (host skips are expected)

See [`AGENT_PARADISE.md`](AGENT_PARADISE.md) · [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md)
