# LIGH — pitch (2026-08-23)

**Read [`README.md`](README.md) first.** This file is the short strategy summary.

---

## Falsifiable thesis

> A coding agent on macOS gets **better outcomes** verifying a Debug `.app` when it uses structured `perceive` / `attempt` + harness verify instead of screenshot + vision — and can **replay known flows with zero LLM tokens**.

**Not the thesis:** “4× faster than WDA on SpringBoard” · “UX graph is agent memory” · “beats Maestro everywhere”

---

## Who it’s for

- **You** ship an iOS app with accessibility identifiers  
- **Your agent** (Cursor, custom MCP) needs to verify flows after Swift edits  
- **You** want fail-closed `{ ok, fault, detail }`, not PNG cosplay  

**Not for:** human QA authoring (Maestro), apps without AX ids, Linux CI without a Mac host.

---

## Three use cases (in order of proof strength)

1. **`app-job`** — known steps, CI acceptance, fail-closed  
2. **Autonomous UX** — LLM discovers flow once; harness checks success id ([`autonomous-ux-latest.json`](docs/assets/autonomous-ux-latest.json))  
3. **Compiled replay** — same flow, 0 LLM on reruns ([`compiled-replay-latest.json`](docs/assets/compiled-replay-latest.json))

---

## Competitive frame

| Alternative | When LIGH might win | When it doesn’t |
|-------------|---------------------|-----------------|
| simctl + screenshot MCP | Agent needs ids + verdicts, not pixels | You already have reliable vision |
| Maestro | Agent-native structured loop + headless host | Human-written YAML tests |
| Appium/WDA | Persistent host, lower latency on our script | Mature ecosystem, real devices |

Maestro/WDA numbers in repo = **one login job footnotes** — see [`docs/HONEST.md`](docs/HONEST.md).

---

## Kill criteria

1. Agentic baseline A/B on 3 OSS apps: LIGH doesn’t win turns or success → stop product thesis  
2. No external developer says “I’d use this on my repo” → stop product thesis  
3. Keep `lighd` motor as OSS either way  

---

## Demo order (honest)

```bash
./scripts/gate-compiled-replay.sh     # no API key — shows zero-LLM replay
./scripts/gate-autonomous-ux.sh       # needs OPENAI_API_KEY — shows agent loop
ligh cap app-job …                    # shows CI primitive
```

Skip leading with Messages/Settings demos — research only.
