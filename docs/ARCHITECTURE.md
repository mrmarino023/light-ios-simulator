# LIGH architecture — universal motor, agent-first

This doc is the **design contract**. Motor regression fixtures (LighFixture, LighFeed, …) exercise the pipeline; they are **not** generalization evidence. See [`gate-external-apps.sh`](../scripts/gate-external-apps.sh).

## Product wedge

> A coding agent gives LIGH an **app-level goal** and receives **structured, verifiable** outcomes — not opaque simulator manipulation.

## Four layers

| Layer | Responsibility | Agent-facing |
|-------|----------------|--------------|
| **L1 Session** | `lighd`, sim boot, HID, app install/swap | `ligh_up`, `ligh_ready` |
| **L2 Perception** | AX dump, scene, overlay, `actionable_topk`, `rank_candidates` | `ligh_observe` |
| **L3 Motor** | ready → resolve → clear_path → **fire+verify** → settle | `reach`, `dismiss_overlay`, taps via `app_goal` |
| **L4 Goal** | Declarative setup + postconditions | `ligh_cap_app_goal`, `ligh_cap_app_job` |

**Human motor (target):** cognition layer (settle judge, probe planner, universal search) + full gesture vocabulary — see [`HUMAN_MOTOR.md`](HUMAN_MOTOR.md).

## Motor pipeline (invariant for every app)

```
ready → perceive → ensure_path → fire_verified → settle
```

### clear_path (overlay FSM)

- **Keyboard** — dismiss before tapping chrome under keys
- **Sheet / alert** — do **not** auto-dismiss if target lives on the overlay; fire AX-first
- **Transition** — wait, do not tap

### fire_verified (no fake ok)

Strategies tried in order until **observable UI change** or exhausted:

1. AX press (sheet/alert or fallback)
2. HID tap
3. HID hold
4. AX press fallback

If all fire but UI unchanged → fault **`motor_no_effect`** (not `ok: true`).

Verification signals: overlay change, screen title change, sheet dismissed, target id left viewport, AX identifier set changed, new sense events.

### reach (host-owned)

`reach(id|label)` = dismiss + scroll + wait until target is on a clear path. Agents should prefer this over manual swipe loops.

### scroll_until

Success when target is **on-screen** (`find_onscreen_id_in_dump`), not merely present in a virtualized AX tree.

## Fault taxonomy (fail-closed)

| fault | Meaning |
|-------|---------|
| `ok` | Postcondition satisfied |
| `target_missing` | Not reachable — read `evidence.candidates` |
| `motor_no_effect` | Fire ack'd but UI unchanged — try `reach`, AX, or fix app |
| `blocked` | Overlay could not be cleared |
| `wrong_surface` | Wrong app in foreground |
| `infra` / `eyes_unusable` | Call `ligh_ready` |
| `timeout` | Budget exhausted |

## What counts as evidence

| Tier | Gate | Claim allowed |
|------|------|----------------|
| **Motor regression** | `gate-workflow-matrix.sh` | Motor ops work on known fixtures |
| **Third-party frozen** | `gate-external-apps.sh`, rigor N=50 | Generalization (per app, publish failures) |
| **Agent loop** | `gate-autonomous-agent.sh` | Agent can close the loop on structured faults |
| **Developer trial** | Human + agentic baseline A/B | Product wedge |
| **Customer discovery** | "How does your agent use Simulator?" | Segment exists or not |

**Do not** conflate motor regression 25/25 with “works on any app.”

## Agent loop

```
observe → app_goal | reach | (edit + build) → app_goal → done
```

Rules:

- No screenshots on the happy path
- Never claim success without LIGH `ok: true`
- On `target_missing` / `motor_no_effect`: read `evidence`, then `reach` or fix source

## Performance targets (agent session)

- Hot path via `lighd` only
- Adaptive settle (stop when `settled && actionable`)
- Per-app relaunch, not full sim reboot between jobs
- Slim evidence to LLM (topk ≤ 8, candidates ≤ 5)

## Next experiments (priority order)

1. Developer trials on real apps — [`DEVELOPER_TRIAL.md`](DEVELOPER_TRIAL.md)
2. LIGH vs simctl+vision on the same agent/task (same prompt, two arms)
3. Add external apps to `fixtures/external-apps/manifest.json` — **no source edits**
