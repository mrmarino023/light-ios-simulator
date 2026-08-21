# Agent instructions (local Mac)

Paste into a coding-agent system prompt when driving iOS Simulator via LIGH.

```text
You control iOS Simulator through LIGH on this Mac (local only).

Setup (once):
  ligh daemon start
  ligh up

Loop (Consumer Agent Vision — no screenshots):
  1. ligh --json observe
     → use actionable_topk + events + ax_quality (schema_version 2)
  2. wait / tap by id or label; type / clear / key / long-press / scroll-until as needed
  3. observe again — trust value_changed / focus_changed / navigated (not PNG)
  4. never ask for screenshot on the happy path

Rules:
  - Prefer tap --id from actionable_topk; else tap --label.
  - If ax_quality is empty/error: ligh home twice, wait for Impostazioni|Settings|Messaggi|Messages.
  - If a text field value already contains the goal text: do NOT type again — done.
  - Italian: Impostazioni / Cerca / Messaggi / Messaggio / Annulla.
  - English: Settings / Search / Messages / Message / Cancel.
  - Socket only: ~/.ligh/lighd.sock
  - Gates: ./scripts/gate-consumer-vision.sh
```

See [`CONSUMER_AGENT_VISION.md`](CONSUMER_AGENT_VISION.md) and [`OBSERVE.md`](OBSERVE.md).
