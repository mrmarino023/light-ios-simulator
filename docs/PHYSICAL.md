# Physical iPhone — eyes, hands, Expo

LIGH on a **real device** is for **Debug / Expo development builds you own** — not
unmodified App Store apps.

Simulator and physical use the same agent loop (`observe` → `tap`/`swipe` →
effect check). The motors differ.

## Architecture

```text
Coding agent / CLI
        ↓
      lighd
   ┌────┴────┐
   │         │
 eyes      hands
   │         │
DevDriver   WDA / Appium XCUITest
 (in-app)   (system UI events)
   │         │
   └────┬────┘
        ↓
  Your Expo / RN Debug app
```

| Role | Path | Why |
|------|------|-----|
| **Eyes** | `@mm-labs/ligh-expo` DevDriver → AX dump over LAN (`:7700`) | Fast in-app tree; Metro-shaped transport |
| **Hands (physical)** | Cascade: `activate` → DevDriver tap → **WDA fallback** | In-app fast path; WDA only when effect fails |
| **Hands (Simulator)** | IndigoHID | Unchanged; not WDA |
| **Law** | `screen_sig` before/after every physical act | ACK without ΔUI = fail closed (`effect` ≠ ok) |

`HybridPhysical` + `physical_motor`: DevDriver for eyes + first motor attempt; WDA
only when in-app paths do not change `screen_sig`. Motors in JSON:
`devdriver_activate`, `devdriver_tap`, `wda_tap`, `wda_label`.

### Motor cascade

```text
activate (VoiceOver-style) → DevDriver tap → WDA tap → WDA label
         ↓ screen_sig effect check (~480ms poll budget) per attempt
```

Benchmark: `./scripts/holy-shit-bench.sh` or `ligh bench holy [--physical]`.

### What failed (and must not return)

| Approach | Result |
|----------|--------|
| In-app DevDriver fake `UITouch` as the only motor | API `ok`, UI often unchanged (esp. RN tab bar) |
| Deep link as “gesture proof” | Navigation worked; **not** a tap |
| Treating Simulator IndigoHID success as phone success | Trust burn |

Deep links remain a useful **owned-app sidecar** for setup. They are not motor
evidence.

## Expo integration — `@mm-labs/ligh-expo`

Universal Expo config plugin. Injects the DevDriver into **development** builds
only (skipped on EAS `production` / `preview`).

```bash
# Vendor into any Expo app (EAS-safe)
./scripts/sync-ligh-expo.sh /path/to/YourExpoApp
```

```json
{
  "expo": {
    "plugins": ["@mm-labs/ligh-expo"]
  }
}
```

Optional: `["@mm-labs/ligh-expo", { "port": 7700, "host": "192.168.1.10" }]`.

Package layout: native sources under `native/` (not `ios/`) so app `.gitignore`
patterns that ignore bare `ios/` cannot drop the driver when vendored.

Full package notes: [`packages/ligh-expo/README.md`](../packages/ligh-expo/README.md).

npm publish of `@mm-labs/ligh-expo` is optional; vendoring via
`file:./packages/ligh-expo` is the supported path today.

## Run physical arms

1. **Quick start (sim + optional Expo app):**

```bash
./scripts/ligh-init.sh
# or with Expo vendoring:
./scripts/ligh-init.sh /path/to/YourExpoApp
# equivalent: ligh init [/path/to/ExpoApp]
```

2. **Phone:** Developer Mode on; trust the computer; install a **dev client**
   with the LIGH plugin; open your app (Mae-class Expo app).
3. **Trust WDA:** first Appium session installs `WebDriverAgentRunner` — accept
   UI testing / trust the developer on device or acts hang with
   `Not authorized for performing UI testing actions`.
4. **Mac env** — copy and edit:

```bash
cp scripts/wda.env.example ~/.ligh/wda.env
# set UDID, bundle id, Apple team id
```

5. **Appium** (keep it running; do not `pkill appium` mid-prove):

```bash
./scripts/start-appium-wda.sh
# or:
APPIUM_HOME=.appium ./node_modules/.bin/appium \
  --address 127.0.0.1 --port 4723 --relaxed-security
```

6. **Daemon + wait:**

```bash
./target/release/lighd &
./target/release/ligh device wait --timeout 45
./target/release/ligh observe --json
./target/release/ligh tap --json --label 'TabProfile'
./target/release/ligh swipe --json --from-x 0.5 --from-y 0.78 --to-x 0.5 --to-y 0.28
```

Prefer `./target/release/ligh` / `lighd` from this repo over an older
`~/.cargo/bin/ligh`.

## Proven on device (2026-08-24)

Target: Expo Mae app (`com.mattisky999.MaeApp`) on physical iPhone, hybrid motor.

| Act | Result |
|-----|--------|
| DevDriver session | `lan` / `lan+wda`, `driver_version: 2` |
| `observe` actionable tabs | Clean labels (`TabEventsHome` … `TabProfile`) |
| Tap **Profile** | `motor: physical`, **`effect: ok`**, `screen_sig` changed |
| Tap **Home** | `motor: physical`, **`effect: ok`**, sig restored |
| Swipe scroll | `ok` via WDA |

Earlier false positives (DevDriver-only touch, deep-link-as-tap) are **not**
counted as motor proof.

## Scope honesty

- Works for apps **you build** with the Expo plugin (or a native Debug embed).
- Does **not** claim App Store automation without a signed WDA + entitlement
  story you own.
- Simulator Autopilot ×3 evidence remains Simulator-scoped until Autopilot is
  wired to the physical WDA motor and re-gated on device.
