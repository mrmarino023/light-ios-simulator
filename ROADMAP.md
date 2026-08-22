# Roadmap — bet the project on one experiment

**Position today (brutal):** interesting **engineering experiment** + agent **proof-of-concept**. As a **product**, it could still be crap — we have not shown anyone needs it.

| | |
|---|---|
| Engineering | 🟢 interesting |
| Benchmark (login job) | 🟢 narrow but real |
| Agent demo | 🟢 PoC (1× LLM + mechanism gate) |
| Generalization | 🔴 not demonstrated |
| Developer need | 🟠 not demonstrated |
| Moat / business | 🔴 no evidence |
| Worth continuing to **test** | 🟢 yes |

### The question we must destroy

> **Why not Maestro / Appium / WDA + a well-built MCP?**

These answers are **not enough** alone:

- “LIGH is faster on our benchmark” — footnote, not wedge
- “LIGH has structured faults” — any backend can wrap errors in JSON
- “LIGH is Rust” — irrelevant to the buyer

**Only answer that matters:** a coding agent (or developer) gets **materially better** outcomes on **their** app/workflow with LIGH vs the same MCP on Maestro — and can say *why* in one sentence.

### Demonstrated (engineering — do not oversell)

- Third-party OSS app (XCUITestDemo), not designed for LIGH
- Fail-closed matrix **5/5** — injected faults never soft-success
- Dirty-state **50/50** — back-to-back app-jobs, no reboot, 0 AX-empty (LIGH-only session)
- Rigor N=50 isolated arms — LIGH **50/50** p50 **2.2s** vs Maestro **30/50** (60%) p50 **27.5s** p95 **127s** (~**12×** p50 on this job). At N=20 Maestro was **20/20**; reliability breaks under back-to-back load at N=50 (bimodal fast aborts vs ~127s timeouts). Published: [`third-party-rigor-latest.json`](docs/assets/third-party-rigor-latest.json)
- MCP **mechanism** — structured fault → scripted corrective retry → `ok`
- **LLM autonomous (1×)** — `gpt-5-mini` + vague prompt → fault step 5 → Swift fix → verify ([`autonomous-agent-latest.json`](docs/assets/autonomous-agent-latest.json))

### Not demonstrated (product)

- Scroll, modals, keyboard, gestures, WebView, permissions, deep links, crashes, multi-screen, large real apps
- 5 different apps · 5 different workflows (not login-only)
- **5 external developers** — install, Cursor, their app, minimal hand-holding
- **Maestro replacement test** — same MCP, same agent, same task; blind preference
- Autonomous matrix at scale (harness exists; **not** the next priority)

### Stop doing (benchmark trap)

Do **not** chase: N=50 → N=100 → more login bakeoffs → prettier latency charts.

We have enough numbers on the **ideal** job (launch → type → tap → assert). Further benchmark optimization without humans is sunk cost.

### Next (the benchmark is a human)

```text
① 5 external developers — "install, use with Cursor on your Debug .app" (say little)
       ↓
② Same task: LIGH MCP vs Maestro MCP (identical agent prompt)
       ↓
③ 5 apps × 5 workflows (not designed for LIGH; not login-only)
       ↓
④ Kill decision
```

**Kill criteria:** after 5 apps + 5 developers, if nobody prefers LIGH or can't articulate why → **stop the product thesis**. The experiment succeeded by falsifying itself early.

**Win signal:** unprompted *"why would I use anything else for agent iOS verify?"* or measurable agent loop win on **their** repo.

### Legacy gates (published — footnotes, not roadmap center)

**Thesis:** A coding agent can reliably use LIGH to verify an Debug `.app`, with fail-closed structured outcomes — **if** that survives hard workflows and dirty Simulator state.

Not: “faster Simulator host.” Not: Settings LLM demos. Not: “we beat Maestro” as headline.

## Red flags (do not hide)

| Issue | Status |
|-------|--------|
| **Dirty sim → AX empty → LIGH 0/10** | Observed after long **Maestro** sessions (cross-tool). LIGH-only 50× dirty passed 2026-08-22 — reboot still required between tool arms in bakeoffs. |
| **Workflow too easy** | login + 2 fields + assert — not nav, lists, modals, WebView, no-a11y-id apps. |
| **Maestro ≠ agent competitor** | Maestro proves UI automation parity/speed; **product** proof = Cursor → MCP → build → verify → fix loop. |
| **Structured fault for agents** | **Mechanism** proven (scripted MCP loop). **Autonomous** recovery from vague prompt — not proven. |

**Danger ranking for agents:** wrong action ≫ crash ≫ timeout ≫ slow.

## The job

```text
Cursor → build .app → LIGH app-job → launch → resolve → act → settle → verify
                                                              ↓
                                                    verified | fault (explicit)
```

Agent must never get “probably tapped Login.” It gets:

```json
{ "ok": true, "fault": "ok", "capability": "app_job", "detail": { "…" } }
```

or:

```json
{ "ok": false, "fault": "target_missing", "detail": { "step": 2, "op": "tap" } }
```

**Danger ranking for agents:** wrong action ≫ slow action ≫ explicit failure.

## Kill metric (multidimensional)

`claim_pass` requires all of:

| Dimension | Bar |
|-----------|-----|
| Reliability | `pass_rate == 1.0` at publish N (currently N=50 on fixture) |
| No silent wrong-target | Every success includes postcondition wait on Done chrome |
| Explicit faults only | Failures emit `FaultClass` — never soft-success |
| Latency | workflow `p95_ms` (iters after first install) under budget |
| Recovery | Mid-flight AX/overlay recovery is host-owned (`ensure_path` / relaunch), scored in results |

Published: [`docs/assets/app-reliability-latest.json`](docs/assets/app-reliability-latest.json).

## Sequence (frozen — no more login benchmark expansion)

```text
✅ Engineering baseline (fixture + XCUITestDemo + fail-closed + dirty 50 + rigor N=50 + MCP + 1× LLM autonomous)
       ↓
⏭️  STOP adding N=50 / matrix runs until developers speak
       ↓
→ 5 developers + Maestro A/B + harder apps/workflows
       ↓
→ Kill or double down
```

Harnesses:
- `./scripts/gate-fail-closed.sh` → `docs/assets/fail-closed-latest.json` (**5/5**)
- `./scripts/gate-dirty-state.sh` → `docs/assets/dirty-state-latest.json` (**50/50**, `LIGH_DIRTY_N=50`)
- `./scripts/gate-third-party-rigor.sh` → `docs/assets/third-party-rigor-latest.json` (clean arms; published **N=50** — LIGH 50/50, Maestro 30/50)
- `./scripts/gate-mcp-loop.sh` → `docs/assets/mcp-loop-latest.json` (proof-of-mechanism; harness scripts the fix)
- `./scripts/gate-app-reliability.sh` (fixture motor)

If ① returns silent wrong-target or ok:true on bad asserts → **stop shipping claims**.  
If ② fails → AX/recovery is the product bug, not latency.  
If ⑥ gets nobody → kill the product thesis.

### What we already ran (baseline — narrow claims only)

| Experiment | Claim allowed | Claim forbidden |
|------------|---------------|-----------------|
| LighFixture N=50 | Motor + multidimensional gate on **our** fixture | “any app” |
| XCUITestDemo N=10 clean | ~9× p50 **on this 6-step login job** | “LIGH beats Maestro” |
| Maestro bakeoff | Automation speed on same YAML job | “better for coding agents” |

## Competitive bakeoff (vs Maestro)

Same app, same workflow, same Mac. User-level job first — not “our primitive vs their YAML.”

Example semantic job (fixture today; real app tomorrow):

```text
launch → Home → type → GoNext → verify Done
```

Table to publish: workflow success · explicit fail · wrong-target · AX-empty/blocked · p50/p95 action · total workflow · recovery · cold start.

Harness: `./scripts/gate-app-bakeoff.sh`

## Debug `.app` narrowing (honest)

“Simulator Debug `.app`” is a real market shrink. That is OK **only if** third-party dogfood works.

- Fixture proves the motor.
- **Third-party app proves the wedge.** Prefer an app we did not design around LIGH.
- If only identifier-rich apps work → claim must say that out loud.

## Agent recovery (wedge, not patch)

Deterministic test frameworks stop. Agents need:

```text
tap → AX empty → wait → re-observe → resolve → tap → verify
```

Host owns recovery (`ensure_ready`, overlay `ensure_path`, relaunch). Agent sees structured fault or verified — not raw CLI noise.

## MCP (not optional)

```text
Cursor → MCP → ligh app-job → Simulator → { ok, fault, detail }
```

`ligh_cap_app_job` must be first-class. Without this bridge, LIGH is another automation CLI.

## Cold Mac < 5 min

Promote over any “× vs WDA” number:

```text
git clone → ./scripts/install.sh → app-job green
```

Measurable. Developer tools die at install.

Harness: `./scripts/gate-cold-start.sh` → [`docs/assets/cold-start-latest.json`](docs/assets/cold-start-latest.json)

**Note:** build with workspace binaries (`unset CARGO_TARGET_DIR && cargo build --release`) — sandboxed builds may land in a cache dir, not `target/release/`.

## Done

- [x] Control plane: phase + overlay + `FaultClass`
- [x] Motor: `ensure_ready → resolve → ensure_path → fire → settle`
- [x] `ligh cap app-job` + fixture
- [x] Reliability gate + N=50 fixture publish (multidimensional `claim_pass`)
- [x] Maestro bakeoff — LIGH wins reliability tie + **~11× faster p50** on fixture (publish either way)
- [x] Demote SpringBoard / vision / “~4× WDA” from marketing
- [x] MCP `ligh_cap_app_job` + compact verified/fault payload
- [x] Cold Mac proof — daemon bounce → app-job **10.6s** (budget 5 min)

## Next (in order)

- [ ] **Developer pack** — install, MCP config, minimal prompt (no benchmark deck)
- [ ] **5 external developers** — their app; observe where it breaks
- [ ] **Maestro MCP A/B** — same agent, same task, blind preference
- [ ] **5 apps × 5 workflows** — not login-only
- [ ] **Kill decision** — no pull after above → stop product thesis
- [x] Engineering baseline + rigor N=50 publish + 1× LLM autonomous (published JSON footnotes)
- [x] ~~N=50 rigor~~ — published; do not re-run without developer signal
- [ ] ~~Autonomous matrix at scale~~ — **deprioritized** (benchmark trap)

## Demote (research only — never marketing)

- SpringBoard / Settings LLM breadth
- Vision PNG bakeoffs
- Scripted “~4× WDA” microbench (footnote at most)

## Out of scope

Cloud farms · remote TCP product · physical device fleets · OCR-as-product
