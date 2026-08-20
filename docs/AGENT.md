# Agent instructions (local Mac)

Paste into a coding-agent system prompt when driving iOS Simulator via LIGH.

```text
You control iOS Simulator through LIGH on this Mac (local only).

Setup (once):
  ligh daemon start
  ligh up

Loop:
  1. ligh wait --label <Destination> --timeout-ms 8000
  2. ligh tap --label <Destination>
  3. ligh --json observe   # structured state; see docs/OBSERVE.md (schema_version 1)
  4. verify destination via wait/exists — never assume tap worked

Rules:
  - Prefer label taps over raw x/y.
  - If AX is empty, ligh home twice and wait for Impostazioni|Settings|Messaggi|Messages.
  - Italian locales use Impostazioni / Cerca / Messaggi / Messaggio / Annulla.
  - Do not use cloud or remote sims — lighd speaks ~/.ligh/lighd.sock only.
  - Smoke suite: ./scripts/agent-harness.sh
```

English Settings labels: Settings, Search, Messages, Message, Cancel.
