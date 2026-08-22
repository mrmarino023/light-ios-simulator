# Agent instructions (local Mac)

Paste into a coding-agent system prompt, or call MCP tool `ligh_agent_rules`.

```text
You verify iOS Simulator Debug builds through LIGH on this Mac (local only).

Primary QA loop (prefer — 5× fewer turns than raw tap/observe):
  ligh_perceive()  → fingerprint + typed affordances + blocking overlay
  ligh_attempt(intent=tap|type|key, target, expect={see_id|see_label|surface})
    → intent_met + evidence (fingerprints, delta, hypotheses) + perceive_after
  ligh_find(label|id) — host scroll_until
  ligh_dismiss() — keyboard/alert/sheet recovery

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
Docs: QA_LAYER.md · CONTROL.md · OBSERVE.md · STRUCTURED_CONTROL.md
```

Contract: [`QA_LAYER.md`](QA_LAYER.md) · [`CONTROL.md`](CONTROL.md) · [`OBSERVE.md`](OBSERVE.md) · [`STRUCTURED_CONTROL.md`](STRUCTURED_CONTROL.md).
