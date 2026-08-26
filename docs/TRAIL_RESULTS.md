# TRAIL results

Last updated: 2026-08-26

## Verdict

TRAIL repair generalizes on the frozen suite: **3/3 verified**, **2/3 ≤120s**, without golden diffs, per-app templates, or reversing a known `bug_patch`.

Artifact: [`assets/trail-holy-multi-latest.json`](assets/trail-holy-multi-latest.json)  
Comparisons: [`assets/trail-holy-compare-latest.json`](assets/trail-holy-compare-latest.json)

Gate: `./scripts/gate-trail-holy-multi.sh`

## Results

| Task | Mode | File | Wall | Tokens | ≤120s |
|------|------|------|------|--------|-------|
| login-never-navigates | `state_gate_stuck` | `LoginViewModel.swift` | **39s** | 1.5k | yes |
| onboarding-home-broken | `blocked_overlay` | `OnboardingView.swift` | **73s** | 3.8k | yes |
| kix-notes-tab-missing | `tab_chrome_missing` | `MainTabView.swift` | 232s | 3.5k | no (verified) |

## Comparisons

### Login (XCUITestDemo)

| Arm | Wall | Tokens | Verified |
|-----|------|--------|----------|
| Vision chat | 622s | 212k | no |
| Autopilot chat | 61s | 14k | yes |
| **TRAIL** | **39s** | **1.5k** | yes |

### Kix Notes tab

| Arm | Wall | Tokens | Verified |
|-----|------|--------|----------|
| Vision chat | 460s | 128k | yes |
| Autopilot chat | 644s | 148k | yes |
| **TRAIL** | **232s** | **3.5k** | yes |

### Onboarding

TRAIL: **73s**, verified. No prior published vision A/B in assets for this task.

## Protocol

```text
TraceFailure → hybrid localize → constrained fix (≤2 shots) → build → certify
```

Same path across login gates, missing tabs, and stuck onboarding.

**Not claimed:** golden `patch -R`, frozen identity cheats, or multi-turn chat thrash as the product proof.

## Reproduce

```bash
./scripts/gate-trail-holy-multi.sh

LIGH_TRAIL_TASK=fixtures/frozen/tasks/login-never-navigates/task.json \
  ./scripts/gate-trail-holy.sh
```

Requires `OPENAI_API_KEY`, release `ligh` / `lighd`, and Simulator.
