# TRAIL results

Last updated: 2026-08-26

## Verdict

**3/3 verified** on the frozen killer suite. Gate claim (≥2/3 with wall ≤120s) **PASS**.

Artifact: [`assets/trail-holy-multi-latest.json`](assets/trail-holy-multi-latest.json)  
Gate: `./scripts/gate-trail-holy-multi.sh`  
Architecture: [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md)

## Results

| Task | Mode | File | Wall | Tokens |
|------|------|------|------|--------|
| login-never-navigates | `state_gate_stuck` | `LoginViewModel.swift` | **40s** | 1.8k |
| onboarding-home-broken | `blocked_overlay` | `OnboardingView.swift` | **64s** | 3.8k |
| kix-notes-tab-missing | `tab_chrome_missing` | `MainTabView.swift` | **126s** | 7.8k |

## Comparisons

### Login (XCUITestDemo)

| Arm | Wall | Tokens | Verified |
|-----|------|--------|----------|
| Vision chat | 622s | 212k | no |
| Autopilot chat | 61s | 14k | yes |
| **TRAIL** | **40s** | **1.8k** | yes |

### Kix Notes tab

| Arm | Wall | Tokens | Verified |
|-----|------|--------|----------|
| Vision chat | 460s | 128k | yes |
| Autopilot chat | 644s | 148k | yes |
| **TRAIL** | **126s** | **7.8k** | yes |

### Onboarding

TRAIL: **64s**, verified.

## Protocol

```text
TraceFailure v2 → Effect Classifier → Causal localize → ≤2 LLM fixes → build → certify
```

- CLI: `ligh cap repair-job …`
- MCP: `ligh_cap_repair_job`

## Reproduce

```bash
./scripts/gate-trail-holy-multi.sh

LIGH_TRAIL_TASK=fixtures/frozen/tasks/login-never-navigates/task.json \
  ./scripts/gate-trail-holy.sh
```

Requires `OPENAI_API_KEY`, release `ligh` / `lighd`, and Simulator.
