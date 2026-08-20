# LIGH Architecture

## Thesis

The iOS **guest** stays Apple’s. LIGH owns the **host**: boot, framebuffer, input, and structured observe — without `Simulator.app`.

```text
Simulator.app:  SimRenderServer + AppKit window
LIGH:           same CoreSimulator guest + Rust Metal import + IndigoHID
```

Falsifiable product bar: a **30–50 step** `observe → act → verify` agent workload must beat MCP + simctl (and ideally WDA) on wall-clock + reliability + structured observation. Screenshot latency alone is not the thesis.

Not a fake runtime. Not a `simctl` MCP wrapper. A programmable host process.

## Capability matrix (v0.3)

| # | Primitive | Status | Notes |
|---|-----------|:------:|-------|
| 1 | Extremely low-latency display | ✅ | IOSurface → Metal zero-copy |
| 2 | Persistent native connection | ✅ | `lighd` Unix socket RPC at `~/.ligh/lighd.sock` |
| 3 | Structured state / observation | ✅ | `observe` JSON: frame + session + headless a11y (`wait` requires 2 consecutive dumps; `tap --label`) |
| 4 | Input | 🟡 | IndigoHID tap/swipe/home/type. Hold ~32ms; AX empty mid-transition — use `wait`. Not “excellent.” |
| 5 | Streaming | 🟡 | Continuous `poll_stream` + `stream_stats` / `frame_meta` over RPC |
| 6 | Deterministic automation | ✅ | `ligh up/run/tap/screenshot/observe/wait/type/status/down` + `--json` |
| 7 | Agent-oriented primitives | ✅ | JSON-lines RPC + `ligh bench agent` (~4× vs WDA/Appium, 0/44 fail) |
| 8 | Headless execution | ✅ | Local headless CoreSimulator (no Simulator.app required) |

## Layers

| Layer | Crate | Responsibility |
|-------|-------|----------------|
| Entrypoint | `ligh-cli`, `ligh-daemon` | CLI / long-lived daemon |
| Orchestration | `ligh-runtime` | Boot → stream → compositor → GUI |
| Private bridge | `ligh-host` | CoreSimulator boot, IOSurface callbacks, HID |
| GPU | `ligh-gpu` | IOSurface → MTLTexture, screenshot, optional window |
| Plumbing | `ligh-sim` | simctl, measure, bench |
| Shared | `ligh-core` | Session, presets, profiles, observe snapshot |

## Display path (zero-copy)

```text
SpringBoard → backboardd → IOSurface
  → SimDevice.io → com.apple.framebuffer.display
  → SimulatorKit registerScreenCallbacks
  → ligh-host → ligh-gpu → MTLTexture
  → present (gui) | screenshot PNG | discard (probe)
```

## Boot path

1. `ligh_host_boot` — private `SimDevice` boot (no Simulator.app)
2. Fallback — `simctl boot` (+ optional `--disabledJob`)
3. Wait userspace / SpringBoard
4. `ligh_host_stream_start` — IOSurface callbacks
5. Optional Metal window (`ligh gui`) or daemon stream (`lighd`)

## Input path

`IndigoHID` via `SimDeviceLegacyHIDClient`:

- `tap` — touch down → up at normalized (0..1) or point coords
- `swipe` — down → interpolated move → up
- `home` — hardware home button
- `type` — IndigoHID keyboard
- `wait` / `exists` — poll AX until label
- `pointer` — down / move / up (GUI + future RPC)

**Agent hot path:** `ligh tap|observe|screenshot` talks to `lighd` over `~/.ligh/lighd.sock` by default.
Use `--direct` only for cold-path benchmarks. Headline measure: `ligh bench agent --steps 40` (or `./scripts/agent-workload-bench.sh`).

## `lighd` socket protocol

**Endpoint:** `~/.ligh/lighd.sock` (Unix domain socket, **local Mac only**)

**Transport:** JSON lines — one request object per line, one response object per line.

### Request

```json
{"cmd":"<command>", ...params}
```

| `cmd` | Params | Effect |
|-------|--------|--------|
| `status` | — | Session + frame heartbeat |
| `boot` | `device?` | Headless boot + attach IOSurface stream |
| `install` | `app` | `simctl install` + launch (detects bundle id) |
| `launch` | `bundle_id` | Launch installed app |
| `tap` | `x`, `y`, `normalized?`, `label?`, `timeout_ms?` | IndigoHID tap (optional AX wait) |
| `swipe` | `from_x`, `from_y`, `to_x`, `to_y`, `normalized?` | Swipe gesture |
| `home` | — | Home button |
| `type` | `text` | IndigoHID keyboard |
| `wait` | `label`, `timeout_ms?` | Poll AX until label |
| `exists` | `label` | AX membership query |
| `screenshot` | `path?` | Dump latest MTLTexture → PNG (file or base64) |
| `frame_meta` | — | `{w,h,id,fps,imports_ok}` |
| `observe` | `ax?` (default true) | Structured observation snapshot |
| `stream_stats` | — | Compositor counters (frames, imports, fps) |
| `shutdown` | — | Shutdown sim + remove socket |

### Response

```json
{"ok": true, "data": { ... }}
{"ok": false, "error": "..."}
```

### Example (local agent)

```bash
# start daemon (keeps stream hot)
lighd &

# boot via RPC
echo '{"cmd":"boot","device":"iphone-15-pro"}' | nc -U ~/.ligh/lighd.sock

# observe
echo '{"cmd":"observe"}' | nc -U ~/.ligh/lighd.sock

# tap center
echo '{"cmd":"tap","x":0.5,"y":0.5,"normalized":true}' | nc -U ~/.ligh/lighd.sock

# screenshot to file
echo '{"cmd":"screenshot","path":"/tmp/frame.png"}' | nc -U ~/.ligh/lighd.sock
```

Or use the CLI (preferred for scripts):

```bash
ligh up && ligh observe --json && ligh tap --x 0.5 --y 0.5 && ligh screenshot -o /tmp/frame.png
```

## vs other tools

| | simctl MCP wrappers | SimSlim | LIGH |
|--|:---:|:---:|:---:|
| Real UIKit apps | ✓ | ✓ | ✓ |
| Primary lever | CLI spawn | guest launchd | **host GPU + HID + AX + RPC** |
| No Simulator.app | often | often | **✓** |
| Zero-copy Metal | — | — | **✓** |
| Persistent host daemon | rare | — | **lighd** |
| Structured observe | screenshots | — | **observe + AX wait/tap** |
| External bench | — | — | **`ligh bench agent` vs WDA/Appium** |

## Private APIs

Same class as simsapp / idb / SimDeck:

- `CoreSimulator` — SimDevice boot/shutdown  
- `SimulatorKit` — screen callbacks + `framebufferSurface`  
- `IOSurface` / `Metal` — cross-process GPU frames  

Sensitive to Xcode updates. Load via `DEVELOPER_DIR`.

## Roadmap

- Tap hold ~32 ms; AX empty mid-transition — always `wait` the destination
- `type` is ASCII via USB HID usages (NSEvent keyCode:0 typed `a` for every glyph)
- AX match prefers buttons/fields; ignores static-text false positives
- WidgetKit kept enabled by default; `--requires nowidgets` blanks home widgets on purpose
- Richer a11y (predicates, scroll-into-view, AX actions)
- Continuous binary frame stream
- Digitizer HID (pressure / multi-touch)
