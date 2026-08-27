# LIGH OSS stranger pipeline — bulletproof contract

Last updated: 2026-08-27

One pipeline for every stranger iOS repo. **No per-app scheme / label / bundle maps.**

## Stages (fault ownership)

```text
HostCapability → acquire → ProjectResolve → gate_project → build
  → EyesReady → label-first discover → ligh_test
```

| Stage | Owns failure | Skip vs fail |
|-------|--------------|--------------|
| HostCapability | `disk_exhausted`, `missing_ios_runtime` | skip / refuse session |
| acquire | `acquire_not_found` (404) | skip |
| gate_project | `missing_watchos_runtime`, `xcode_format_too_new` | **skip** (do not thrash xcodebuild) |
| build | `build_failed`, `build_timeout`, `codesign` | fail |
| EyesReady | `sim_boot_hung`, `eyes_unusable` | **host fail** — do not blame the app |
| discover | `discover_no_chrome` | app fail (only after EyesReady ok) |
| ligh_test | motor / goal faults | app (unless eyes) |

Invariant: **never** write `REPLACE_ME` goals. **Never** invent taps from `Text(` scrape.

## Entry points

```bash
./scripts/gate-oss-stranger-batch.sh          # scripts/oss-stranger-urls.txt
./scripts/gate-oss-stranger-smoke.sh          # Countries + Food Truck
python3 scripts/ligh_oss_smoke.py URL[#subdir]
```

Modules: `ligh_host_capability.py` · `ligh_project.py` · `ligh_discover.py` · `ligh_oss_smoke.py`

Artifact: [`assets/oss-stranger-trial-latest.json`](assets/oss-stranger-trial-latest.json)  
Competitive map: [`COMPETITIVE.md`](COMPETITIVE.md)
