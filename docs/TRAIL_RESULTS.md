# TRAIL results

Last updated: 2026-08-26

## Verdict

**3/3 verified** on the frozen killer suite. Gate claim (≥2/3 with wall ≤120s) **PASS**.

Artifact: [`assets/trail-holy-multi-latest.json`](assets/trail-holy-multi-latest.json)  
Compare: [`assets/trail-holy-compare-latest.json`](assets/trail-holy-compare-latest.json)  
Gate: `./scripts/gate-trail-holy-multi.sh`  
Architecture: [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md)

## Head-to-head

| Task | Vision chat | Autopilot + chat | **TRAIL** |
|------|-------------|------------------|-----------|
| login-never-navigates | 622s · 212k · ✗ | 61s · 14k · ✓ | **40s · 1.8k · ✓** |
| kix-notes-tab-missing | 460s · 128k · ✓ | 644s · 148k · ✓ | **126s · 7.8k · ✓** |
| onboarding-home-broken | — | — | **64s · 3.8k · ✓** |

Vision / autopilot+chat numbers from killer-loop A/B assets (`killer-loop-ab-v2-*.json`). Onboarding has no published vision A/B yet.

| | vs Vision (wall) | vs Autopilot+chat (wall) | vs Vision (tokens) |
|--|------------------|--------------------------|--------------------|
| Login | ~16× (vision failed) | ~1.5× | ~118× fewer |
| Kix | ~3.7× | ~5.1× | ~16× fewer |

## TRAIL absolute

| Task | Mode | File | Wall | Tokens |
|------|------|------|------|--------|
| login-never-navigates | `state_gate_stuck` | `LoginViewModel.swift` | **40s** | 1.8k |
| onboarding-home-broken | `blocked_overlay` | `OnboardingView.swift` | **64s** | 3.8k |
| kix-notes-tab-missing | `tab_chrome_missing` | `MainTabView.swift` | **126s** | 7.8k |

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
