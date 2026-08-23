# UX Graph — optional compile path (not LLM memory)

LIGH can persist screen fingerprints and transitions to `<workspace>/.ligh/uxgraph.json` while you run `perceive` / `attempt` with a workspace set.

**Honest positioning:** the graph is **telemetry + a compiler input**. It does **not** make LLM agents navigate better (disproven — see [`ux-graph-prove-latest.json`](assets/ux-graph-prove-latest.json)).

The one validated use case: **compile `intent_met` edges → motor steps → zero-LLM replay**.

Evidence: [`compiled-replay-latest.json`](assets/compiled-replay-latest.json) · gate: `./scripts/gate-compiled-replay.sh`

---

## Concepts

| Object | Meaning |
|--------|---------|
| **Screen node** | `fingerprint` + affordance labels |
| **Transition edge** | `from_fp` → `to_fp` via tap/type, with `intent_met` history |
| **Compiled flow** | Motor step list written to `.ligh/compiled/{goal_id}.json` |
| **Baseline / regress** | Structural diff vs saved screens (experimental) |

---

## Workflow that works

```text
1. Run a successful flow (motor seed OR autonomous UX discover arm)
2. ligh uxgraph compile-flow HomeReady --workspace …
3. ligh uxgraph execute-compiled HomeReady MyApp.app --bundle-id … --workspace …
   → llm_tokens = 0, harness verifies goal id
```

Motor-only seed (no LLM):

```bash
./scripts/gate-compiled-replay.sh
```

---

## CLI

```bash
export LIGH_WORKSPACE=/path/to/ios/repo

ligh --json uxgraph status --workspace "$LIGH_WORKSPACE"
ligh --json uxgraph compile-flow HomeReady --workspace "$LIGH_WORKSPACE"
ligh --json uxgraph execute-compiled HomeReady build/MyApp.app \
  --bundle-id com.you.app --workspace "$LIGH_WORKSPACE"
```

Auto-record happens on `ligh cap perceive` / `ligh cap attempt` when `--workspace` is set.

---

## MCP tools

| Tool | Purpose |
|------|---------|
| `ligh_ux_status` | Graph summary (nodes, edges) |
| `ligh_ux_baseline` / `ligh_ux_regress` | Structural diff (experimental) |
| `ligh_ux_explore` | Safe BFS explore (research) |
| `ligh_ux_hint` | Fingerprint → source file hint |

**Do not** instruct agents to “read the graph instead of perceiving” — replay-arm A/B showed they ignore it or do worse.

---

## Experimental / research

```bash
./scripts/gate-ux-graph-prove.sh              # discover vs seed → compile → execute
LIGH_PROVE_PHASE1=discover ./scripts/gate-ux-graph-prove.sh   # includes flaky LLM discover
cargo test -p ligh-core uxgraph::
```

---

## See also

- [`QA_LAYER.md`](QA_LAYER.md) — primary agent API  
- [`../README.md`](../README.md) — use cases and canonical gates
