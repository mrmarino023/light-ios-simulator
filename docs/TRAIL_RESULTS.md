# TRAIL results

Last updated: 2026-08-26

## Job under test

Inject a bug into a real iOS app. The agent must **fix the Swift** and **prove the fix in Simulator**.

| Metric | Meaning |
|--------|---------|
| Wall | seconds until verified fix |
| Tokens | LLM tokens burned |
| ✓ / ✗ | postconditions passed |

## What each stack is

| Stack | Plain English |
|-------|----------------|
| **Vision LLM agent** | Screenshots → LLM decides taps → LLM edits code in chat. Typical “vision coding agent” baseline. |
| **Chat agent + LIGH taps** | Unconstrained chat still owns repair; LIGH Autopilot only drives UI. Proves better taps ≠ repair. |
| **LIGH (TRAIL)** | Host proves failure → localizes file → structural restore and/or ≤2 scoped patches → rebuild → certify. |

## Head-to-head

| Bug | Vision LLM agent | Chat agent + LIGH taps | **LIGH (TRAIL)** |
|-----|------------------|------------------------|------------------|
| Login never navigates | 622s · 212k · ✗ | 61s · 14k · ✓ | **33s · 1.3k · ✓** |
| Notes tab missing (Kix) | 460s · 128k · ✓ | 644s · 148k · ✓ | **78s · 0 · ✓** |
| Onboarding stuck | — | — | **64s · 4.4k · ✓** |

Baselines: `docs/assets/killer-loop-ab-v2-*.json`. Onboarding has no vision A/B published yet.

| | LIGH vs Vision | LIGH vs Chat+taps |
|--|----------------|-------------------|
| Login wall | ~19× (vision failed) | ~1.9× |
| Kix wall | ~5.9× | ~8× |
| Login tokens | ~160× fewer | ~10× fewer |

## LIGH absolute

| Bug | Mode | File | Wall | Tokens |
|-----|------|------|------|--------|
| login-never-navigates | `state_gate_stuck` | `LoginViewModel.swift` | **33s** | 1.3k |
| onboarding-home-broken | `blocked_overlay` | `OnboardingView.swift` | **64s** | 4.4k |
| kix-notes-tab-missing | `tab_chrome_missing` | `MainTabView.swift` | **78s** | 0 |

**3/3 verified ≤120s** · claim **PASS**  
[`assets/trail-holy-multi-latest.json`](assets/trail-holy-multi-latest.json) · [`assets/trail-holy-compare-latest.json`](assets/trail-holy-compare-latest.json)

Kix used **structural tab restore** (omitted `tab_*` + View type still in tree) — no LLM shot required.

## Protocol (LIGH)

```text
TraceFailure → Effect Classifier → Causal localize
  → structural tab restore (when possible) / ≤2 LLM fixes
  → build → certify
```

- CLI: `ligh cap repair-job …`
- MCP: `ligh_cap_repair_job`
- Architecture: [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md)

## Reproduce

```bash
./scripts/gate-trail-holy-multi.sh
```

Requires `OPENAI_API_KEY`, release `ligh` / `lighd`, and Simulator.
