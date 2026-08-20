# Xcode / Simulator pin (local)

LIGH uses Apple **private** frameworks (`CoreSimulator`, `SimulatorKit`, IndigoHID, AXPTranslator).  
Everything runs on **your Mac** — pin what works.

## Record what you run

| Item | Example |
|------|---------|
| macOS | 14.x / 15.x |
| Xcode | 16.x (`xcodebuild -version`) |
| iOS Simulator runtime | 18.1 |
| LIGH commit | `git rev-parse --short HEAD` |

When a new Xcode breaks boot, AX, or HID: open an issue with the table above + `ligh doctor` output.

## Local recovery

```bash
sudo xcode-select -s /Applications/Xcode.app
xcrun simctl runtime list
ligh down
ligh up --device iphone-15-pro
ligh doctor
```

No cloud fallback — fix the local toolchain.
