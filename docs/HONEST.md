# Honest status

Last updated: 2026-08-23

## Bottom line

**Real tool. Unproven product.**

You have a coding agent that edits Swift. You want it to **open the app, try the flow, and say if it worked**. LIGH is a local Mac experiment for that — accessibility JSON + verified actions, not screenshots.

We do **not** know if developers prefer this over XCUITest or vision. **That is the only question that matters now.**

**Next milestone:** 5 iOS developers using Cursor (or similar) try LIGH on **their** app without a 1-hour walkthrough. If 3/5 get value, explore further. If most say "I'd just generate XCUITest," stop the product thesis and keep the motor as OSS.

---

## What actually works (with published JSON)

| Layer | Verdict |
|-------|---------|
| `lighd` motor + headless sim | ✅ Keep — real infra |
| Fail-closed `app-job` faults | ✅ 5/5 injected faults never lie |
| QA `perceive` / `attempt` + evidence | ✅ Demonstrated — [`qa-layer-latest.json`](assets/qa-layer-latest.json) |
| Autonomous agent + **harness verify** (no scripted nav) | ✅ LighOnboard pass — [`autonomous-ux-latest.json`](assets/autonomous-ux-latest.json) |
| **Compiled replay** (seed → compile → execute, 0 LLM) | ✅ LighOnboard pass — [`compiled-replay-latest.json`](assets/compiled-replay-latest.json) |
| OSS login job reliability (app-job) | ✅ LIGH 50/50 at rigor N=50 — narrow job only |
| MCP fault → retry mechanism | ✅ Scripted proof, not a reliability stat |
| LLM finds one injected Swift bug (1×) | ✅ PoC only — [`autonomous-agent-latest.json`](assets/autonomous-agent-latest.json) |

---

## What we tried and should stop selling

| Claim | Verdict |
|-------|---------|
| **UX graph helps LLM navigate faster** | ❌ Disproven — replay arm used *more* tokens than control — [`ux-graph-prove-latest.json`](assets/ux-graph-prove-latest.json) |
| **“Beats Maestro” (general)** | ❌ One login job, one OSS app — footnote at best |
| **“Graph is the product memory”** | ❌ Recording works; LLM navigation aid does not |
| **`host_policy` / co-designed caps** | ❌ Not agent proof |
| **15/15 breadth / frontier vision wins** | ❌ Research harnesses — not wedge |
| **Cloud agent tests iOS** | ❌ LIGH is Mac-local only |

---

## The only product question that matters

> On **your** app, does Cursor + LIGH MCP fix or verify faster than simctl + screenshot/vision — and can you say **why**?

**Zero external developers have answered that yet.**

Until then: publish gates, publish failures, run [`docs/AGENTIC_BASELINE.md`](AGENTIC_BASELINE.md) fairly.

---

## Kill criteria

After **3 blind OSS apps** + same agent prompt vs agentic baseline:

- LIGH wins on success rate **or** ≥2× fewer agent turns **or** unprompted preference  
- Otherwise: **stop the product thesis**. Keep motor as OSS infra.

---

## Run the honest tests (Mac)

```bash
unset CARGO_TARGET_DIR && cargo build --release -p ligh-cli -p ligh-daemon

# Canonical product gates
./scripts/gate-autonomous-ux.sh       # LLM + perceive/attempt + harness
./scripts/gate-compiled-replay.sh     # zero-LLM replay

# Motor acceptance
LIGH_APP_N=50 ./scripts/gate-app-reliability.sh
LIGH_APP_N=50 ./scripts/gate-third-party-rigor.sh

# Experimental — publish even when claim_pass: 0
./scripts/gate-ux-graph-prove.sh
./scripts/gate-agentic-baseline.sh    # LIGH vs vision (needs OPENAI_API_KEY)
```
