# QA layer — agent substrate (5× architecture)

LIGH’s product wedge for coding agents is not raw `tap`/`observe`. It is **verdict-based control**: one host call returns what changed, whether intent was met, and what to try next.

## Agent API (prefer these)

| MCP tool | CLI | Replaces |
|----------|-----|----------|
| `ligh_perceive` | `ligh cap perceive` | `observe` + manual AX parsing |
| `ligh_attempt` | `ligh cap attempt tap …` | `tap` + `observe` + guess |
| `ligh_find` | `ligh cap find --label …` | `scroll-until` + retries |
| `ligh_dismiss` | `ligh cap dismiss` | ad-hoc keyboard/alert handling |
| `ligh_cap_autopilot` | `ligh cap autopilot` | the complete LLM-driven UI loop |
| `ligh_cap_app_job` | `ligh cap app-job` | CI acceptance (known steps) |

Low-level `ligh_tap` / `ligh_observe` remain for debugging. Coding agents should
prefer one `autopilot` call; `perceive + attempt` remains the interactive fallback.

## Perceive (world model + Feel IR)

```bash
ligh --json cap perceive --settle-ms 2500
```

Returns (inside `detail`):

- `perceive` — fingerprint, affordances, blocking (full QA view)
- `feel` — **Feel IR** (preferred for planning): place + ranked salience + block + delta + `suggest`

Feel IR is a **live frame**, not UX-graph memory. Host planners use it; do not treat it as LLM long-term memory.

Also: `ready` / `eyes_unusable`, `location.fingerprint`, typed `affordances[]`, `since_last`.

## Attempt (act + verify)

```bash
ligh --json cap attempt tap --id loginButton \
  --expect '{"see_id":"homeTitle"}' --settle-ms 2500
```

Returns `detail.verdict`:

- `intent_met` — motor **and** expectation
- `evidence.pre_fingerprint` / `post_fingerprint` / `fingerprint_changed`
- `evidence.delta_events` — focus/value/keyboard/navigated
- `evidence.missing` — which expect clauses failed
- `evidence.hypotheses` — e.g. `a11y_id_mismatch`, `silent_tap`, `backend_rejection`
- `perceive_after` — fresh world model

Intents: `tap`, `type`, `key`.

Expect JSON keys: `see_id`, `see_label`, `surface`, `fingerprint_changed`.

## Autopilot (goal → host-discovered path)

```bash
ligh --json cap autopilot \
  --app build/MyApp.app --bundle-id com.example.MyApp \
  --goal-id homeTitle \
  --param alice --param secure:secret
```

The input is the acceptance target plus typed data. There is deliberately no
step-list argument. Rust repeatedly builds Feel IR, fills fields by kind, ranks
safe controls, handles overlays, waits for async CTA transitions, and verifies
the target. The response has `reached`, a compact trace and `llm_tokens: 0`.
Failure returns a semantic `diagnosis` and an optional `source_hint`.

## Coding-agent loop (thin)

```text
read Swift → write minimal patch → build → ligh_cap_autopilot
  → reached: strict harness accepts immediately; stop
  → not reached: diagnosis/source_hint → fix Swift; retry
```

Do **not** ask the LLM to “read the UX graph and navigate” — that path was
disproven. Feel IR is the live frame; Autopilot consumes it inside the host.

One `attempt` replaces ~4–6 observe/tap/observe/guess turns.

## Prove it

```bash
cargo test -p ligh-core qa::
./scripts/gate-qa-layer.sh              # unit gate (any platform)

# Mac integration (booted sim + fixtures):
./scripts/gate-autonomous-ux.sh         # canonical — LLM + perceive/attempt + harness
./scripts/gate-compiled-replay.sh       # zero-LLM replay
```

## Design rule

> Every host call answers: what I did, what changed, whether your intent worked, and what to try next.

See also: [`UX_GRAPH.md`](UX_GRAPH.md) (compile/replay only) · [`OBSERVE.md`](OBSERVE.md) · [`AGENT.md`](AGENT.md)
