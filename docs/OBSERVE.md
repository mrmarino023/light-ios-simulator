# Observe contract

Stable agent-facing snapshot from `ligh observe` / `lighd` RPC `observe`.

Agents should depend on **these fields**, not on CLI text output.

## Envelope (RPC)

```json
{ "ok": true, "data": { /* ObserveSnapshot */ } }
{ "ok": false, "error": "…" }
```

CLI: `ligh --json observe` prints the snapshot (or error) as JSON.

## `ObserveSnapshot` — schema_version **2**

Breaking vs the flat v1 dump: agents should read **`actionable_topk`** + **`events`** first; full `accessibility_tree.nodes` remains for tooling.

| Field | Type | Meaning |
|-------|------|---------|
| `schema_version` | u32 | **`2`** |
| `udid` | string | Active simulator |
| `booted` | bool | Guest booted |
| `simulator_app_running` | bool | Simulator.app present (LIGH prefers headless) |
| `frame` | object? | IOSurface/Metal meta: `w`, `h`, `id`, `fps`, `imports_ok` |
| `app_bundle_id` | string? | Foreground app if known |
| `accessibility_tree` | tagged | See below |
| `scene` | object? | Screen-level summary (title, keyboard, alerts) |
| `actionable_topk` | array | Default LLM view: hittable/on-screen interesting nodes (capped) |
| `events` | array | Sensation events since last observe (focus/value/alert/…) |
| `ax_quality` | string | `ready` \| `empty` \| `stale` \| `error` \| `transition` |
| `settled` | bool | True when `ax_quality==ready` after settle |
| `phase` | string? | Control plane: `booting` \| `ax_warming` \| `ready` \| `acting` \| `settling` \| `degraded` \| `dead` |
| `eyes_unusable` | bool | If true, agents must `ligh ready` — do not invent UI |
| `observe_ms` | number? | Server build time |
| `path` | string? | `"lighd"` (hot) or `"direct"` (cold) |

Source of truth: `crates/ligh-core/src/observe.rs` → `ObserveSnapshot` / `OBSERVE_SCHEMA_VERSION`.

### `scene` (v2)

| Field | Meaning |
|-------|---------|
| `bundle_id` | Same as `app_bundle_id` when known |
| `screen_title` | Best heading / large title guess from AX |
| `keyboard_visible` | Heuristic from keyboard-ish AX roles |
| `keyboard_frame` | Optional frame object |
| `alerts` / `sheets` | Dialog-like nodes in foreground (labels) |

### `actionable_topk[]` node fields

| Field | Meaning |
|-------|---------|
| `id` | Stable path hash (`n` + 8 hex) |
| `role` / `traits` | AX role + trait hints |
| `text` / `label` / `value` / `placeholder` | Visible / field text |
| `focused` / `selected` / `enabled` / `hittable` / `visible` | Affordance |
| `frame` | `{x,y,width,height}` device points |
| `center_norm` | `{x,y}` in 0..1 |
| `parent_id` | Parent node id when known |

### `events[]` (sensation)

```text
{ "t": <unix_secs>, "kind": "focus_changed"|"value_changed"|"alert_appeared"
         |"keyboard_shown"|"navigated"|"ax_empty"|"action_result", "payload": {…} }
```

Also available via RPC/CLI `sense` (recent buffer only).

## `accessibility_tree`

Tagged by `status`:

| status | Meaning |
|--------|---------|
| `available` | Live AX dump — use `nodes` |
| `empty` | No tree / no frontmost app |
| `error` | Bridge failed (`message`) |
| `not_implemented` | Legacy stub |

When `available`:

- `nodes[]` — flat interactive elements (enriched in v2)
- `root` — optional nested root (includes `id` / tree links)
- `element_count` — optional
- `point_size` — `[width, height]` device points for normalize

### Match rules (`wait` / `tap --label` / `tap --id`)

- **label**: case-insensitive contains on label/identifier/value; prefer editable roles for Search/Cerca; prefer top-most among equals
- **id**: exact match on `id`
- Mid-transition trees can be `empty` — **always `wait` the destination** before acting

## Related RPCs

```text
wait   { label? , id? , timeout_ms }
exists { label? , id? }
tap    { label? , id? } | { x, y, normalized }
long_press { label? , id? , hold_ms? } | coords
scroll_until { label? , id? , max_swipes? }
type   { text }
clear  { count? }          # delete/backspace N times
key    { name }            # return | delete | escape | tab | …
sense  {}                  # recent sense_events
home / swipe / screenshot  # screenshot = debug
```

## Minimal agent loop (v2)

```bash
ligh daemon start
ligh up
ligh --json observe --settle-ms 2500   # never act on transition/empty
ligh wait --label Impostazioni
ligh tap --label Impostazioni
ligh --json observe --settle-ms 2500   # verify surface/events
```

`typed` / `host_accepted` means HID accepted keystrokes — Messages body may still be missing from AX `value`.

Design: [`STRUCTURED_CONTROL.md`](STRUCTURED_CONTROL.md).
