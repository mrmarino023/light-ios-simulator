# UX Graph — computational user experience

LIGH turns app UX into a **persistent, diffable graph** stored at `<workspace>/.ligh/uxgraph.json`.

Each `perceive` / `attempt` auto-records nodes (screen fingerprints) and edges (verified transitions).

## Concepts

| Object | Meaning |
|--------|---------|
| **Screen node** | `fingerprint` + affordance labels + optional `source_hints` |
| **Transition edge** | `from_fp` → `to_fp` via `intent` + target, with `intent_met` history |
| **Baseline** | Named snapshot of known-good screens |
| **Regress diff** | Structural diff vs baseline (new/removed/changed screens) |
| **Source hint** | Correlation `fingerprint ↔ Swift file` (confidence rises with edits) |

## Setup

```bash
export LIGH_WORKSPACE=/path/to/your/ios/repo   # or pass workspace in MCP/CLI
ligh daemon start && ligh up
```

Graph path override: `LIGH_UXGRAPH_PATH=/custom/uxgraph.json`

## CLI

```bash
# Auto-record on every perceive/attempt (default when workspace set)
ligh --json cap perceive --workspace "$LIGH_WORKSPACE"
ligh --json cap attempt tap --id loginButton \
  --expect '{"see_id":"homeTitle"}' --workspace "$LIGH_WORKSPACE"

# Graph ops
ligh --json uxgraph status --workspace "$LIGH_WORKSPACE"
ligh --json uxgraph baseline v1.0 --workspace "$LIGH_WORKSPACE"
ligh --json uxgraph regress v1.0 --workspace "$LIGH_WORKSPACE"
ligh --json uxgraph explore --max-steps 6 --workspace "$LIGH_WORKSPACE"
ligh --json uxgraph hint fp_abc123 ContentView.swift --workspace "$LIGH_WORKSPACE"
```

## MCP (Cursor)

| Tool | Purpose |
|------|---------|
| `ligh_perceive` | World model + **records node** |
| `ligh_attempt` | Act + verify + **records edge** |
| `ligh_ux_status` | Graph summary |
| `ligh_ux_baseline` | Save baseline |
| `ligh_ux_regress` | Structural regress diff |
| `ligh_ux_explore` | Safe BFS explore |
| `ligh_ux_hint` | Link fingerprint → source file |

Pass `workspace` in tool args or set `LIGH_WORKSPACE`.

## Agent loop (fix + regress)

```text
1. ligh_ux_baseline("pre-fix")          # optional
2. ligh_attempt(tap login, expect home)  # fails → hypotheses
3. edit Swift (agent)
4. ligh_ux_hint(fp, "ContentView.swift") # after edit
5. xcodebuild → ligh_attempt → pass
6. ligh_ux_regress("pre-fix")           # structural UX diff
```

## Flywheel (why it improves)

Every verify loop adds:

- New screens discovered (`explore`)
- Transition reliability stats (`intent_met` rate per edge)
- Source hints (`fingerprint → file` correlation)
- Baseline diffs on each build

**The graph is the product memory** — not the benchmark JSON.

## Prove it

```bash
cargo test -p ligh-core uxgraph::
./scripts/gate-uxgraph.sh
```

Mac integration: run explore on a booted sim after `ligh up`.

See also: [`QA_LAYER.md`](QA_LAYER.md) · [`AGENT.md`](AGENT.md)
