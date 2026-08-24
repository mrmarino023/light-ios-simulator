# `@mm-labs/ligh-expo`

Universal Expo config plugin: in-process **LIGH DevDriver** for physical iPhone
**development** builds. Works for any Expo / React Native app.

On device, this package is the **eyes** (AX dump over LAN). Physical **hands**
are WebDriverAgent via `lighd` — see [`docs/PHYSICAL.md`](../../docs/PHYSICAL.md).

## Install

```bash
# From the LIGH repo into your Expo app (recommended for EAS)
./scripts/sync-ligh-expo.sh /path/to/YourExpoApp
```

Or dependency:

```bash
npm i -D @mm-labs/ligh-expo
# until published: "file:./packages/ligh-expo"
```

`app.json` / `app.config.js`:

```json
{
  "expo": {
    "plugins": ["@mm-labs/ligh-expo"]
  }
}
```

Optional props:

```js
["@mm-labs/ligh-expo", { "port": 7700, "host": "192.168.1.10" }]
```

Skipped automatically on EAS `production` and `preview` profiles.

## Contract

- Phone connects to `lighd` on your Mac (Metro-shaped transport, default `:7700`)
- `hello` advertises `driver_version` + gesture capabilities
- AX dump powers `ligh observe` / Feel IR on physical sessions
- Gesture IR exists in-process for lab; **production physical motor is WDA**
  (fake UITouch alone is not trusted for RN tab bars)

## Layout

Native sources live under `native/` (not `ios/`) so Expo app `.gitignore`
patterns like bare `ios/` cannot drop them when this package is vendored.

## Rebuild note

Native driver changes require a **new native build** (EAS / `npx expo run:ios`).
JS reload is not enough.
