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

## Perceive (world model)

```bash
ligh --json cap perceive --settle-ms 2500
```

Returns (inside `detail.perceive`):

- `ready` / `eyes_unusable`
- `location.fingerprint` — stable screen hash (no coordinates)
- `location.surface` / `title` / `bundle_id`
- `blocking` — keyboard | alert | sheet | transition
- `affordances[]` — typed: `text_field`, `primary_button`, `nav_back`, …
- `since_last` — sensation event kinds

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
ligh_perceive
  → pick affordance
ligh_attempt(intent=tap, id=…, expect={see_id:…})
  → if !intent_met: read evidence.hypotheses → fix Swift → rebuild
  → repeat
```

One `attempt` replaces ~4–6 observe/tap/observe/guess turns.

## Prove it

```bash
cargo test -p ligh-core qa::
./scripts/gate-qa-layer.sh          # unit gate (any platform)
# Mac integration (requires booted sim):
# OPENAI_API_KEY=… ./scripts/gate-qa-agent.sh
```

## Design rule

> Every host call answers: what I did, what changed, whether your intent worked, and what to try next.

See also: [`OBSERVE.md`](OBSERVE.md) · [`AGENT.md`](AGENT.md) · [`CONTROL.md`](CONTROL.md).
