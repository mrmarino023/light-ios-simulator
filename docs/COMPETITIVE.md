# LIGH vs competitors — brutal honest map

Last updated: 2026-08-30

## One line

**Maestro proves durable UI flows. LIGH proves the agent's code change** — fail-closed certify + TRAIL repair — sold to **eval / CI / agent platforms** that cannot vibes-merge.

Not competing for “every Cursor user installs a tap MCP.” That market is owned.

---

## Competitor matrix

| | **Maestro MCP** | **XcodeBuildMCP** | **ios-simulator-mcp** | **Vision baseline** | **LIGH** |
|--|-----------------|-------------------|----------------------|---------------------|----------|
| **Primary job** | Write/run E2E flows | Build + test Xcode | Drive sim | Screenshot → guess | **Certify + repair agent edits** |
| **Buyer** | QA + agents | Agents building | Agents tapping | Chat default | **Eval harnesses · agent PR CI · platforms** |
| **Agent entry** | `inspect_screen`, `run` | 79 MCP tools | tap/swipe/screenshot | chat + vision | **`ligh_test` / scorepack** |
| **Scored repair** | ❌ | ❌ | ❌ | ❌ | **TRAIL + scorepack** ✅ |
| **Structured faults** | YAML errors | build JSON | raw | none | **`ok` / `app_crashed` / repair_contract** |
| **UI token cost** | inspect + flows | schema tax | medium | **very high** | **0 on motor** ✅ |
| **Cloud** | Maestro Cloud ✅ | ❌ | ❌ | varies | Scorepack/certify Actions (hosted Mac = later) |

**Compose:** Maestro = durable E2E. XcodeBuildMCP = build. LIGH = truth of **this** Swift change.

---

## Where we shine (ranked)

1. **Agent scorepack** — frozen inject → fix → `ok:true` → wall/tokens/faults ([`docs/SCOREPACK.md`](SCOREPACK.md))
2. **CI certify** — agent PR goal postconditions; emit fault / `trail_allowed`
3. **Not** solo-dev paradise, stranger cold-build theater, or Android parity with Maestro

---

## OSS stranger architecture (supporting, not the wedge)

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

## When buyers should pick LIGH

- Scoring **coding agents** on iOS UI repair (eval harness / platform)
- **Agent PR CI**: goal certify + structured fault; optional TRAIL
- Fail-closed `ok:true` only — cannot vibes-merge

## When buyers should pick Maestro

- Cross-platform durable E2E suite + Cloud
- QA-owned YAML flows

## When buyers should pick XcodeBuildMCP

- Build/test/debug loop only — no certify/repair semantics

---

## LIGH product surface

```text
gate-scorepack.sh     → scored TRAIL pack (PRIMARY for platforms)
ligh_test / agent-loop → .ligh/last-certify.json (CI + dogfood)
ligh_cap_repair_job   → TRAIL when certify proves bug
ligh_viewer           → debug only
```

---

## Still missing (honest)

- Hosted Mac multi-tenant (commodity without unique job — add after scorepack demand)
- External agent plug-in API (today: TRAIL host arm; agent-under-test harness next)
- Android
- Repair fully in Rust (Python on hot path)
- Broad production effect classes beyond core pack

See [`SCOREPACK.md`](SCOREPACK.md) · [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md)
