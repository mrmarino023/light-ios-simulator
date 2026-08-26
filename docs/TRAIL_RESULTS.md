# TRAIL results

Last updated: 2026-08-26

## Verdict

TRAIL repair generalizes on the frozen suite: **3/3 verified ≤120s**, without golden diffs, per-app templates, or reversing a known `bug_patch`.

Artifact: [`assets/trail-holy-multi-latest.json`](assets/trail-holy-multi-latest.json)  
Comparisons: [`assets/trail-holy-compare-latest.json`](assets/trail-holy-compare-latest.json)

Gate: `./scripts/gate-trail-holy-multi.sh`

## Results

| Task | Mode | File | Wall | Tokens | ≤120s |
|------|------|------|------|--------|-------|
| login-never-navigates | `state_gate_stuck` | `LoginViewModel.swift` | **41s** | 1.2k | yes |
| onboarding-home-broken | `blocked_overlay` | `OnboardingView.swift` | **67s** | 3.3k | yes |
| kix-notes-tab-missing | `tab_chrome_missing` | `MainTabView.swift` | **91s** | 3.2k | yes |

## Comparisons

### Login (XCUITestDemo)

| Arm | Wall | Tokens | Verified |
|-----|------|--------|----------|
| Vision chat | 622s | 212k | no |
| Autopilot chat | 61s | 14k | yes |
| **TRAIL** | **41s** | **1.2k** | yes |

### Kix Notes tab

| Arm | Wall | Tokens | Verified |
|-----|------|--------|----------|
| Vision chat | 460s | 128k | yes |
| Autopilot chat | 644s | 148k | yes |
| **TRAIL** | **91s** | **3.2k** | yes |

### Onboarding

TRAIL: **67s**, verified.

## Protocol

```text
TraceFailure → hybrid localize → constrained fix (≤2 shots) → build → certify
```

Product entry points:

- CLI: `ligh cap repair-job …`
- MCP: `ligh_cap_repair_job` (`task_path` → full TRAIL; or `app`+`exercise` → daemon prove/contract)

## Reproduce

```bash
./scripts/gate-trail-holy-multi.sh

LIGH_TRAIL_TASK=fixtures/frozen/tasks/kix-notes-tab-missing/task.json \
  ./scripts/gate-trail-holy.sh
```

Requires `OPENAI_API_KEY`, release `ligh` / `lighd`, and Simulator.
