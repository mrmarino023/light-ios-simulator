# Contributing

LIGH is **MIT open source**. Thanks for helping.

## Setup

```bash
git clone https://github.com/mrmarino023/light-ios-simulator.git
cd light-ios-simulator
./scripts/install.sh
ligh doctor
```

Requires macOS, Xcode + an iOS Simulator runtime, and Rust (`https://rustup.rs`).

## Before you PR

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings   # fix what you can
cargo test --workspace
./scripts/prove.sh                        # if you have a sim runtime
```

## High-value contributions

- Xcode version compatibility (private API drift)
- Digitizer HID path for newer iOS sim touch models
- `ligh gui` polish (resize, keyboard, rotation)
- Honest RAM/boot benchmarks with reproducible scripts
- CI improvements for simulator runtime on GitHub Actions
- Docs / install friction (if you got stuck, we want that fixed)

## Private APIs

Changes in `crates/ligh-host/src/bridge/` touch Apple private frameworks. Document which Xcode/iOS versions you tested.

## Code of conduct

Be direct, be kind, cite proof when claiming performance wins.
