# LIGH — strategy & pitch

## Falsifiable thesis (locked)

> **LIGH exists only if the agent loop is faster, more structured, and headless than alternatives.**

The product is **not** “screenshot in 17 ms.”  
The product is a structured agent loop:

```text
observe()  →  framebuffer + a11y + elements + app state
tap(label=…) / wait(for=…) / type(…)
observe()  again   # verify
```

**Market:** replace **MCP + simctl + photos** for coding agents and headless CI.  
**Not:** TestFlight · Appium-as-QA-platform · XCTest-as-test-framework.

**Status (2026-08-20):** ~**4× vs WDA/Appium** on a 44-step script (0% fail).

**Falsify further:** 50×40 reliability, AX-empty rate, per-action observe→act→verify latency.

---

## Positioning

**LIGH — open-source programmable host for the real iOS Simulator.**

One-liner:

> Agents control a real CoreSimulator guest through a persistent host (`lighd`): IOSurface frames, IndigoHID input, headless AX — **without Simulator.app**.

Not:

> lightweight iOS · nicer Rust simctl · “Playwright for iOS” MCP wrapper · QA platform

---

## Killer demo (the loop)

**Messages (real app, screen-record ready):**

```bash
ligh daemon start && ligh up
./scripts/demo-type-agent.sh
# → Messaggi → Nuovo messaggio → type pitch line
```

**Settings (search field):**

```bash
ligh daemon start
ligh up
ligh wait --label Impostazioni   # or Settings
ligh tap --label Impostazioni
ligh wait --label Generali       # destination, not hope
ligh tap --label Cerca           # prefers AXSearchField / top-most
ligh type --text Bluetooth
ligh wait --label Bluetooth
ligh observe --json              # frame + AX tree
ligh screenshot -o /tmp/out.png
```

Prefer `ligh gui` or Simulator.app in frame for recording.  
`tap --x/--y` alone is not the demo.

---

## Measured comparison (from `ligh bench agent`)

Last checked sample (iPhone 15 Pro / iOS 18.1, Italian locale, 2026-08-20):

| Driver | Time | Failures |
|--------|------|----------|
| **LIGHd** | **10.6–13.2 s** | **0/44** |
| **WDA / Appium XCUITest** | **~50–53 s** | **0/44** |

**~4× vs WDA/Appium** on the same 44-step script.

```bash
ligh daemon start
ligh bench agent --steps 40
```

JSON: `docs/assets/agent-bench-latest.json`. Start Appium in a normal Terminal first for the WDA cell.

**Headline metrics:**

- **total wall-clock** for the 30–50 step script
- **failure rate** / pass-fail per step
- per-op class **p50 / p95**
- optional observe JSON size (bytes)

Screenshot vs `simctl io` is a **supporting** micro-metric. It must not lead the pitch.

If `workload` is `FAIL`, do not pitch LIGH as agent-ready.

---

## Why not “just MCP”

The market already has SimPilot, ios-mcp, simulator-mcp, Appium/WDA, idb, and raw `simctl`.

**MCP tools = commodity.**  
**Persistent host** (framebuffer + HID + AX + daemon) = only differentiation that can survive.

MCP belongs **on top of** `lighd`, not instead of it.

```text
Agent / IDE / CI / viewer / MCP
            │
            ▼
          LIGH (lighd)
   observe · wait · tap · type · verify
            │
      CoreSimulator → iOS
```

---

## Why LIGH vs WDA

WDA/Appium can drive the Simulator. LIGH wins on the **same 30–50 step workload** with lower wall-clock, structured `observe` (frame + AX + app state), and a headless host without Simulator.app.

Measured: **~4× vs WDA/Appium**, 0/44 failures (see table above).

---

## Open source (this repo)

MIT host substrate:

- headless CoreSimulator boot (no Simulator.app)
- IOSurface → Metal
- IndigoHID: tap · swipe · home · type
- headless AX: `observe` / `ax` / `wait` / `exists` / `tap --label`
- persistent `lighd` JSON-lines RPC (`~/.ligh/lighd.sock`)

---

## What we will not claim

- Guest RAM / “30% lighter iOS”
- Perfect home widgets on slim boots (`--requires nowidgets` blanks WidgetKit on purpose)
- Screenshot latency as the reason LIGH exists

---

## Kill criteria

After the checked-in agent workload suite:

1. Loop time ≈ existing WDA/idb/`simctl`+vision tooling → **no thesis**
2. Flake rate worse than WDA on the same machine → **not a substrate**
3. Private API breaks every Xcode train with no gate → **unmaintainable**

Compete only where numbers win.
