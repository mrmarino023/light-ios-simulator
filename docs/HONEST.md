# Honest status (do not oversell)

Last updated: 2026-08-22

## Is this useless?

**Not 100%. Not a product either.**

| Layer | Verdict |
|-------|---------|
| `lighd` motor (headless sim, HID, AX) | Real engineering — keep |
| Fail-closed faults | Real — keep |
| Login N=50 / Maestro bakeoffs | **Misleading** — stop publishing as wins |
| `host_policy` / `agent-cap-loop.py` goal matching | **Cheating** — not agent proof |
| `perceive` / `attempt` + evidence | Right direction — **unvalidated on Mac** |
| UX Graph (`.ligh/uxgraph.json`) | Right direction — **unvalidated on Mac** |
| Cloud agent (Linux) | **Cannot test iOS** — LIGH is Mac-local MCP only |

## What would make it NOT useless

One sentence from a developer who did not build LIGH:

> "On my app, Cursor + LIGH MCP fixed a real bug in fewer steps than screenshot/computer-use, and I know why."

**Zero developers have said that.**

## Kill criteria (unchanged)

After **3 blind OSS apps** + same Cursor prompt:

- LIGH MCP: higher success **or** ≥2× fewer turns **or** unprompted preference
- Otherwise: **stop product thesis**. Keep motor as OSS infra.

## What we will not claim

- "Beats Maestro" (general)
- "5× better" (unmeasured)
- "Autonomous agent works" (1× injected typo)
- "15/15 breadth" (`host_policy` / co-designed caps)
- "Frontier wins" (`if "bluetooth" in goal`)

## What we can claim today

- Host returns structured `intent_met` + evidence (unit-tested)
- UX graph persists nodes/edges/baselines (unit-tested)
- Mac integration: **your job to run** (`gate-autonomous-qa.sh`)

## Run the honest test (Mac)

```bash
export LIGH_WORKSPACE=$PWD
export OPENAI_API_KEY=…
./scripts/gate-third-party-rigor.sh   # narrow — not marketing
./scripts/gate-autonomous-qa.sh       # QA-layer agent (attempt + ux_hint)
./scripts/gate-blind-bakeoff.sh       # protocol doc + turn counter scaffold
```
