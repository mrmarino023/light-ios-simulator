# QA layer — agent substrate (5× architecture)

LIGH’s product wedge for coding agents is not raw `tap`/`observe`. It is **verdict-based control**: one host call returns what changed, whether intent was met, and what to try next.

## Agent API (prefer these)

| MCP tool | CLI | Replaces |
|----------|-----|----------|
| `ligh_perceive` | `ligh cap perceive` | `observe` + manual AX parsing |
| `ligh_attempt` | `ligh cap attempt tap …` | `tap` + `observe` + guess |
| `ligh_find` | `ligh cap find --label …` | `scroll-until` + retries |
| `ligh_dismiss` | `ligh cap dismiss` | ad-hoc keyboard/alert handling |
| `ligh_cap_app_job` | `ligh cap app-job` | CI acceptance (known steps) |

Low-level `ligh_tap` / `ligh_observe` remain for debugging. **Agents should plan with perceive + attempt.**

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

## Agent loop (thin)

```text
ligh_perceive → read feel.salience / feel.suggest
  → prefer host exercise / app-job when steps are known
  → else ligh_attempt(intent=tap, id|label=…, expect={…})
  → if !intent_met: evidence.hypotheses → fix Swift → rebuild
```

Do **not** ask the LLM to “read the UX graph and navigate” — that path was disproven. Feel IR is the live frame; compiled replay / `exercise_app` is the zero-LLM path.

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
