# Changelog

## Unreleased

### Added

- Messages typing demo: `scripts/demo-type-agent.sh` + `docs/assets/ligh-messages-demo.{gif,mp4}`
- Screen-record friendly Settings demo: `scripts/demo-agent.sh`
- Clearer MIT install path (`scripts/install.sh` PATH hints, Homebrew notes)
- Agent reliability harness + workloads + `docs/OBSERVE.md` + `ROADMAP.md`
- `scripts/time-to-first-loop.sh` + `scripts/agent-harness.sh` (local-only gate)
- `docs/AGENT.md` + `docs/XCODE.md`
- Observe `schema_version: 1`

## 0.3.0 — 2026-08-20

### Added

- **ligh-host** — ObjC bridge to CoreSimulator + SimulatorKit (private APIs)
- **ligh-gpu** — Metal compositor, IOSurface zero-copy import, `ligh gui` window
- **ligh-runtime** — boot → IOSurface stream → Metal orchestration
- **IndigoHID** touch + home via `SimDeviceLegacyHIDClient`
- `ligh probe` — headless GPU path smoke test
- `ligh gui` — interactive Metal window (no Simulator.app)
- `ligh gui --verify` — 5s Metal present smoke test (exit 0/1)
- Reuse `LIGH-*` simulators (no new device every boot)
- SpringBoard wait via host `pgrep` (sim root has no grep/pgrep)
- IOSurface stream retries after boot
- Homebrew formula + `scripts/install.sh`
- `--disabledJob` boot profile (171 SimSlim-compatible labels, UDID-before-flags fix)

### Changed

- Architecture pivot: host GPU path primary; launchctl slim demoted
- `lighd` v3 attaches IOSurface stream + Metal compositor

### Known limits

- Private API stability tied to Xcode version
- RAM savings vs stock not benchmarked to ≥30% yet
- Touch uses legacy mouse HID path (digitizer dispatch planned)

## 0.2.0

Early simctl / launchctl supervisor (superseded by v3 GPU path).

## 0.1.0

Initial Rust CLI, device create, headless boot readiness fix.
