# Observe contract

Stable agent-facing snapshot from `ligh observe` / `lighd` RPC `observe`.

Agents should depend on **these fields**, not on CLI text output.

## Envelope (RPC)

```json
{ "ok": true, "data": { /* ObserveSnapshot */ } }
{ "ok": false, "error": "…" }
```

CLI: `ligh --json observe` prints the snapshot (or error) as JSON.

## `ObserveSnapshot`

| Field | Type | Meaning |
|-------|------|---------|
| `schema_version` | u32 | Contract version (**`1`**). Additive fields keep `1`; breaking changes bump. |
| `udid` | string | Active simulator |
| `booted` | bool | Guest booted |
| `simulator_app_running` | bool | Simulator.app present (LIGH prefers headless) |
| `frame` | object? | IOSurface/Metal meta: `w`, `h`, `id`, `fps`, `imports_ok` |
| `app_bundle_id` | string? | Foreground app if known |
| `accessibility_tree` | tagged | See below |
| `observe_ms` | number? | Server build time |
| `path` | string? | `"lighd"` (hot) or `"direct"` (cold) |

Source of truth: `crates/ligh-core/src/observe.rs` → `ObserveSnapshot` / `OBSERVE_SCHEMA_VERSION`.

## `accessibility_tree`

Tagged by `status`:

| status | Meaning |
|--------|---------|
| `available` | Live AX dump — use `nodes` |
| `empty` | No tree / no frontmost app |
| `error` | Bridge failed (`message`) |
| `not_implemented` | Legacy stub |

When `available`:

- `nodes[]` — flat elements
- `root` — optional nested root
- `element_count` — optional
- `point_size` — `[width, height]` device points for normalize

### Node fields used by `wait` / `tap --label`

| Field | Use |
|-------|-----|
| `label` | Primary match string |
| `identifier` | Secondary match |
| `value` | Field contents when present |
| `role` | Prefer search/text fields over static text |
| `frame` | `{x,y,width,height}` in device points → tap center |

Match rule (summary): case-insensitive contains on label/identifier; prefer editable roles; prefer top-most among equals. Mid-transition trees can be `empty` — **always `wait` the destination** before acting.

## Related RPCs (same label semantics)

```text
wait   { label, timeout_ms }
exists { label }
tap    { label } | { x, y, normalized }
type   { text }
home / swipe / screenshot
```

## Minimal agent loop

```bash
ligh daemon start
ligh up
ligh wait --label Impostazioni    # or Settings
ligh tap --label Impostazioni
ligh --json observe               # verify structured state
```

## Compatibility

Additive fields may appear. Do not require unknown keys. Treat unknown `accessibility_tree.status` as error and retry/`wait`.
