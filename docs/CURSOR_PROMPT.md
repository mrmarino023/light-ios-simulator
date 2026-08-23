# Cursor prompt — developer trial

Copy into Cursor chat (adjust paths):

---

You are on a Mac with **LIGH MCP** connected. Your job: verify my **Debug iOS Simulator `.app`**.

1. Build the app for Simulator (`xcodebuild` or I’ll tell you the scheme).
2. Call `ligh_up` then `ligh_cap_app_job` with steps that match my UI (use `accessibilityIdentifier` values from the Swift source when possible).
3. If the result is `{ "ok": false, "fault": "...", "detail": { "step", "op", ... } }`, read the fault, fix the **source**, rebuild, and retry until `ok: true` or you hit a real blocker.
4. Do **not** claim success without `ok: true` from LIGH. Do **not** use screenshots to plan taps.

My app: `PATH_TO_APP.app` · bundle id: `BUNDLE_ID`

First acceptance job (edit steps to match my UI):

```json
[
  {"op":"wait","id":"REPLACE_ME"},
  {"op":"tap","id":"REPLACE_ME"},
  {"op":"wait","id":"REPLACE_DONE"}
]
```

If LIGH returns `eyes_unusable`, call `ligh_ready` and retry once.

---

**Optional A/B (if Maestro MCP is also configured):** run the same workflow with `maestro_run_flow` — QA footnote only.

**Primary comparison:** run the same task with **simctl + screenshot + vision** (no LIGH) and compare recovery, tool calls, and time to green. See [`AGENTIC_BASELINE.md`](AGENTIC_BASELINE.md).
