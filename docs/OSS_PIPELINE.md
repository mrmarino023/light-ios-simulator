# LIGH OSS stranger pipeline — bulletproof contract

Last updated: 2026-08-28

Two products, same motor — see architecture v5 in `COMPETITIVE.md`.

| Product | Input | KPI |
|---------|-------|-----|
| **Agent loop** (primary) | `ligh_init(agent_project)` → build (agent/Xcode) → `ligh_test` | time-to-ok per patch |
| **Stranger proof** (secondary) | `--app` / sim artifact / git_cold URL | `tier_b_verify_pass` |

## Acquire tiers

| Tier | Mode | Build |
|------|------|-------|
| A | `agent` — agent workspace | agent/XcodeBuild |
| B | `app` / `workspace` + prebuilt `.app` | **none** — LIGH verifies only |
| C | `git` — cold clone URL | xcodebuild (benchmark; `build_failed` ≠ LIGH broken) |

## Stages (fault ownership)

```text
HostCapability → acquire → preflight_v2(SPM) → SessionBootstrap
  → [build Tier C only] → chrome_trust(motor) → ligh_test → harness_repair?
```

| Stage | Owns failure | Skip vs fail |
|-------|--------------|--------------|
| HostCapability | `disk_exhausted`, `missing_ios_runtime` | skip / refuse session |
| acquire | `acquire_not_found` (404) | skip |
| **preflight_v2** | full `Package.swift` tree + SPM resolve → `swift_tools_too_new` | **skip** (before xcodebuild) |
| gate_project | watchOS / Xcode format / macOS | **skip** |
| build | `build_failed`, `build_timeout` | fail (Tier C only) |
| SessionBootstrap | `sim_boot_hung` | **host fail** |
| discover | `discover_no_chrome` / `app_crashed` / `app_not_running` | app fail — crash ≠ no chrome; no goal without motor proof when alive |
| ligh_test | motor / goal faults | app (unless eyes) |
| **harness_repair** | retry motor/discover/test | **host pipeline only** |

### Process health (agent-speed)

Every observe with `expected_bundle_id` stamps `process_health`:

- `running` / `pid` from sim `launchctl`
- `crashed_recently` + `crash_report_path` / `crash_signal` from DiagnosticReports
- `hint` points the agent at `.ips` / atos — **does not** invent Swift root cause

`discover_no_chrome` is **illegal** when `crashed_recently` — use `app_crashed`.

### System auth overlay

AX dump prefers SafariViewService / AuthenticationServicesUI when present (`ax_source: system_auth`). Overlay `system_auth` is sheet-like: fire AX inside, never auto-dismiss.

## Paradise: two repair paths

| Failure in | Auto-fix target | Never |
|------------|-----------------|-------|
| **Stranger app** (login gate, tab missing) | TRAIL `repair_engine.py` on `source_root` | per-app URL maps |
| **Host pipeline** (chrome, eyes, daemon) | `ligh_harness_repair.py` retry | edit vendored Swift |

Harness repair is **on by default** in `ligh_oss_smoke.py`. Log: `.oss-trial/harness-repairs.jsonl`

## Chrome trust

- **motor_only** — `wait-label` must prove chrome before `ligh_test`
- Static scrape = hints only; i18n dot-keys rejected (`ligh_chrome.py`)
- No `REPLACE_ME` goals in production path
- **`ligh_invariants.py`** — enforced at discover write + smoke certify

## Metrics

Artifact schema 5: separate KPIs — `tier_b_verify_pass` (stranger sim_app) vs cold-build benchmark.

```bash
# Tier B — prebuilt .app, no xcodebuild (primary stranger path)
python3 scripts/ligh_oss_smoke.py \
  --app /path/to/App.app \
  --bundle-id com.example.app \
  --source-root /path/to/source

# Preflight only — catch swift_tools 6.1+ before 5 min xcodebuild
python3 scripts/ligh_oss_smoke.py --preflight-only --urls-file scripts/oss-stranger-urls-proven.txt

# Invariant gate (fast, no sim — run first)
./scripts/gate-invariants.sh

# Full batch (runnable + host taxonomy probes)
./scripts/gate-oss-stranger-batch.sh

# Proven golden — preflight v2 should skip Nextcloud on Swift 6.0 host
python3 scripts/ligh_oss_smoke.py --preflight-only --urls-file scripts/oss-stranger-urls-proven.txt

python3 scripts/ligh_oss_smoke.py --no-repair-loop URL   # disable harness retry
```

Modules: `ligh_chrome.py` · `ligh_harness_repair.py` · `ligh_host_capability.py` · `ligh_discover.py` · `ligh_oss_smoke.py`

Artifact: [`assets/oss-stranger-trial-latest.json`](assets/oss-stranger-trial-latest.json)
