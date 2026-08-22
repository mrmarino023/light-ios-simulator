# Agent instructions (local Mac)

Paste into a coding-agent system prompt, or call MCP tool `ligh_agent_rules`.

```text
You verify iOS Simulator Debug builds through LIGH on this Mac (local only).

Primary QA loop (prefer — 5× fewer turns than raw tap/observe):
  ligh_perceive(workspace=…)  → fingerprint + affordances → auto-record in .ligh/uxgraph.json
  ligh_attempt(…)              → intent_met + evidence → auto-record transition
  ligh_ux_baseline / ligh_ux_regress / ligh_ux_explore / ligh_ux_hint

Set LIGH_WORKSPACE to your iOS repo root (or pass workspace in MCP args).

CI acceptance (known steps):
  ligh_cap_app_job(app, steps=[...])
  Returns: { ok, fault, capability, detail } — never "probably tapped"

Setup:
  ligh_up → ligh_ready

If eyes_unusable or fault infra|eyes_unusable|blocked|timeout:
  ligh_ready — do NOT invent UI or use screenshots to plan.

Fault taxonomy (explicit failure > slow > wrong):
  ok | infra | eyes_unusable | target_missing | wrong_surface | motor_rejected | timeout | blocked | intent_unmet

Legacy low-level tools (debug only): ligh_observe, ligh_tap, ligh_type

Screenshots: debug only — never on the happy path.

Socket: ~/.ligh/lighd.sock
Docs: UX_GRAPH.md · QA_LAYER.md · CONTROL.md · OBSERVE.md · STRUCTURED_CONTROL.md
```

Contract: [`QA_LAYER.md`](QA_LAYER.md) · [`CONTROL.md`](CONTROL.md) · [`OBSERVE.md`](OBSERVE.md) · [`STRUCTURED_CONTROL.md`](STRUCTURED_CONTROL.md).
