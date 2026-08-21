# Roadmap — local de facto (this Mac)

LIGH is a **local** execution layer: your Mac, CoreSimulator, `lighd` on a Unix socket.  
No cloud farm. No remote sims. If it isn’t useful on a developer/CI Mac, it isn’t done.

## Done

- [x] MIT + `./scripts/install.sh` + Homebrew HEAD
- [x] [`docs/OBSERVE.md`](docs/OBSERVE.md) + `schema_version: 1` → **v2** (Consumer Agent Vision)
- [x] Workloads: Settings, Messages, SpringBoard smoke
- [x] `scripts/agent-reliability.sh` — published **100/100 · 0%** (50×50 both)
- [x] `scripts/time-to-first-loop.sh`
- [x] `scripts/agent-harness.sh` — one local gate
- [x] [`docs/AGENT.md`](docs/AGENT.md) — paste into agent system prompts
- [x] [`docs/CONSUMER_AGENT_VISION.md`](docs/CONSUMER_AGENT_VISION.md) — see + feel + motor (no PNG to LLM)

## Consumer Agent Vision (local)

Design: dense scene graph + sensation bus + human motor. Screenshot = debug only.

- [x] Observe schema **v2** (`actionable_topk`, `scene`, `events`, `ax_quality`)
- [x] AX richness (`id`, focus/hittable/tree fields)
- [x] Motor: `long-press`, `scroll-until`, `clear`, `key`, `tap --id`
- [x] Agent loop uses observe v2 only (`scripts/agent-llm-loop.py`)
- [x] Substrate gate: observe v2 + motor (`./scripts/gate-consumer-vision.sh`)
- [x] Gate **40/40** LLM Messages+Settings **without images** (`gpt-5-mini`, settle→surface→act) → `docs/assets/consumer-vision-gate-latest.json`
- [ ] Vision-only baseline comparison (`LIGH_VISION_COMPARE=1`) when API budget allows
- [ ] Broader goals beyond Settings/Messages composer (still not generic computer-use)

Cloud / remote farms remain **out of scope** (see below).

## Local next (still this machine)

- [ ] Cold Mac proof: clone → install → first loop &lt; 5 min (stopwatch once, paste numbers)
- [ ] Same Settings/Messages smoke **vs WDA** at N≥20 (Appium in Terminal)
- [ ] Document Xcode major pin when something breaks ([`docs/XCODE.md`](docs/XCODE.md))
- [ ] Optional: thin MCP that only wraps local `lighd` (community)

## Out of scope

- Cloud / multi-tenant Simulator farms  
- Remote TCP product  
- Physical device fleets  

## Local gates

```bash
./scripts/install.sh
ligh doctor
ligh daemon start && ligh up
./scripts/agent-harness.sh
```
