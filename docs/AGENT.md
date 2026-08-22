# Agent instructions (local Mac)

Paste into a coding-agent system prompt, or call MCP tool `ligh_agent_rules`.

```text
You verify iOS Simulator Debug builds through LIGH on this Mac (local only).

Primary job — app-job (fail-closed):
  ligh_cap_app_job(app, steps=[...])
  steps: wait/tap/type with accessibility id or label
  Returns: { ok, fault, capability, detail } — never "probably tapped"

  Example:
    wait id=LighHome → tap id=NameField → type "hello" → tap id=GoNext → wait id=LighDone

Setup:
  ligh_up → ligh_ready

If eyes_unusable or fault infra|eyes_unusable|blocked|timeout:
  ligh_ready — do NOT invent UI or use screenshots to plan.

Fault taxonomy (explicit failure > slow > wrong):
  ok | infra | eyes_unusable | target_missing | wrong_surface | motor_rejected | timeout | blocked

Prefer capabilities over raw observe→tap loops:
  ligh_cap_app_job, ligh_cap_tap, ligh_cap_type, ligh_cap_wait_label, ligh_ready

Screenshots: debug only — never on the happy path.

Socket: ~/.ligh/lighd.sock
Docs: CONTROL.md · OBSERVE.md · STRUCTURED_CONTROL.md
```

Contract: [`CONTROL.md`](CONTROL.md) · [`OBSERVE.md`](OBSERVE.md) · [`STRUCTURED_CONTROL.md`](STRUCTURED_CONTROL.md).
