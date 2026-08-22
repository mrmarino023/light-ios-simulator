# Control plane — agent infrastructure for Debug `.app`

**Job:** coding agent verifies your Simulator Debug build with fail-closed structured control.

```text
Cursor → MCP → app-job → launch → resolve → ensure_path → act → settle → verify
```

## Outcomes (never “probably”)

```json
{ "ok": true, "fault": "ok", "capability": "app_job", "detail": { "steps": 5 } }
```

```json
{ "ok": false, "fault": "target_missing", "detail": { "step": 2, "op": "tap" } }
```

Danger for agents: **wrong action ≫ slow ≫ explicit fail**.

## Motor

```text
ensure_ready → resolve(id|label) → ensure_path (clear overlay) → fire → settle
```

Typing may raise keyboard; clearing is the **next** act’s `ensure_path`. Host owns recovery (relaunch / overlay), not the gate script.

## Multidimensional claim

```bash
./scripts/gate-app-reliability.sh          # LIGH_APP_N=50 for publish
# claim_pass = 100% AND Done postcondition AND warm p95 ≤ LIGH_APP_P95_MS
```

Published: [`assets/app-reliability-latest.json`](assets/app-reliability-latest.json).

Third-party dogfood (required before marketing the wedge):

```bash
LIGH_APP_PATH=…/MyApp.app LIGH_APP_BUNDLE_ID=… \
LIGH_APP_HOME_ID=… LIGH_APP_FIELD_ID=… LIGH_APP_GO_ID=… LIGH_APP_DONE_ID=… \
LIGH_APP_N=20 ./scripts/gate-app-reliability.sh
```

## MCP

`ligh_cap_app_job` — first-class product tool. See [`STRUCTURED_CONTROL.md`](STRUCTURED_CONTROL.md).

## Competitors

Maestro bakeoff (same semantic job): `./scripts/gate-app-bakeoff.sh`

Sequence / kill rules: [`../ROADMAP.md`](../ROADMAP.md).
