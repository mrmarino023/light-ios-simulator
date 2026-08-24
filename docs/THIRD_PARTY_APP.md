# Third-party Debug `.app` dogfood

The fixture (`LighFixture`) proves the motor. **The wedge requires an app you did not design around LIGH.**

## Requirements

Your app must expose stable **accessibility identifiers** (or labels) for:

1. Home / entry chrome
2. One text field (optional but recommended for type step)
3. Primary action button
4. Success / done screen element

## Run (XCUITestDemo — vendored OSS sample)

```bash
./scripts/gate-xcuitestdemo-bakeoff.sh   # LIGH vs Maestro, N=10
```

App: [himalidev/XCUITestDemo](https://github.com/himalidev/XCUITestDemo) — login flow with `accessibilityIdentifier`s (not written for LIGH).

Also vendored: [byKosta/Kix-app](https://github.com/byKosta/Kix-app) — catalog / auth / tabs / notes (`fixtures/third-party/Kix`). Build with `./scripts/build-kix.sh`.

Published: [`docs/assets/third-party-bakeoff-latest.json`](../docs/assets/third-party-bakeoff-latest.json)

## Run (your app)
```bash
export LIGH_APP_PATH=/path/to/Debug-iphonesimulator/YourApp.app
export LIGH_APP_BUNDLE_ID=com.your.bundle
export LIGH_APP_HOME_ID=YourHomeId
export LIGH_APP_FIELD_ID=YourFieldId   # omit type step if no field
export LIGH_APP_GO_ID=YourSubmitId
export LIGH_APP_DONE_ID=YourDoneId

LIGH_APP_N=20 ./scripts/gate-app-reliability.sh
```

## If it fails

- Publish the JSON with `claim_pass: false` — that is valuable.
- Narrow the public claim to: *"apps with accessibility identifiers on Simulator Debug builds"*
- Do not revert to Settings/SpringBoard demos.

## Suggested candidates

- A minimal open-source iOS app with explicit `accessibilityIdentifier` in SwiftUI/UIKit
- Your own in-development app (best story)
- **Not:** Apple system apps (Maps, Settings) — research only
