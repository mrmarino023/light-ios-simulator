# TRAIL — bulletproof architecture for every OSS app

Last updated: 2026-08-26

This is the **product contract**, not a benchmark scoreboard. Any vendored SwiftUI app with accessibility identifiers gets the same pipeline.

## One pipeline (no per-app branches)

```text
broken tree only
  → R0 StructuralKB + identity index (cold, per app)
  → R1 TraceFailure v3 (motor + AX diff)
  → R2 Effect Classifier → RepairMode | refuse
  → R3 Causal Localizer (KB graph ascent)
  → R4 Structural operators (effect-class transforms)
  → R4b LLM fixer ≤2 shots (only on operator miss)
  → R5 hard certify (same exercise oracle)
  → R6 wall governor
```

**Product entry:** [`scripts/repair_engine.py`](../scripts/repair_engine.py)  
**Orchestrator:** [`scripts/trail_holy.py`](../scripts/trail_holy.py)

## Hard invariants (never break)

| Rule | Why |
|------|-----|
| Broken tree only | Index/localize the injected app — never `LIGH_IDENTITY_SOURCE` BACKUP |
| Mode from TraceFailure | Effect Classifier only — **never** `task["id"]` |
| No filename priors | No `MainTabView` / `LoginViewModel` / tab name string boosts |
| No golden reverse | Fixer never applies `bug.patch` backward |
| No `// BUG:` spoilers | Gates strip inject comments before build |
| Refuse > guess | `unknown` → `localize_failed` — no speculative wrong-file edit |
| Hard certify | Same exercise + postconditions — no autopilot soft-pass |

## R0 — Structural KB (per app, from broken sources)

Built once per repair from `task.source_root`:

| Edge | Meaning |
|------|---------|
| identity → View file | who declares `accessibilityIdentifier` |
| View → `@StateObject` / `@EnvironmentObject` type | who owns interaction |
| type → ObservableObject file | who publishes gate state |
| parent → `ChildView(` | composition / overlay host |
| `@Binding is*Visible` | overlay dismiss writer |

Module: [`scripts/structural_kb.py`](../scripts/structural_kb.py)

## R2 — Effect Classifier (pure TraceFailure)

Closed modes:

| Mode | Signal |
|------|--------|
| `tab_chrome_missing` | expected `tab_*` absent; Tab Bar / sibling tabs present |
| `state_gate_stuck` | tap + control still visible + sig flat |
| `blocked_overlay` | finish-like control; overlay atoms persist |
| `wrong_navigation` | sig changed but acceptance still pending |
| `motor_rejected` | type/motor rejected; presentation block likely |
| `target_never_visible` | expected id never in AX (non-tab case) |
| `unknown` | **refuse edit** |

Module: [`scripts/effect_classifier.py`](../scripts/effect_classifier.py)

## R3 — Causal Localizer (KB ascent, not filenames)

| Mode | Ascent |
|------|--------|
| `tab_chrome_missing` | observed sibling `tab_*` → TabView composition missing expected |
| `state_gate_stuck` | control → **presentation block** (`.disabled`) **or** ObservableObject writer |
| `blocked_overlay` | control view → host with `@Binding` / finish handler |
| `motor_rejected` | control → declaration site (view interaction fault) |
| `target_never_visible` | identity index → composition |

**Login disabled vs login gate:** same classifier symptom; localizer distinguishes via KB — `.disabled(true)` on declaring view → edit **view**, not ViewModel.

Module: [`scripts/repair_engine.py`](../scripts/repair_engine.py) → `causal_localize()`

## R4 — Structural operators (effect-class, not task patches)

Transforms registered by **mode**, parameterized by TraceFailure + broken tree:

| Operator | Mode | When safe (single-site) |
|----------|------|-------------------------|
| `structural_tab_restore` | `tab_chrome_missing` | `XxxView` type still in tree; TabView host localized |
| `gate_bool_flip` | `state_gate_stuck` | one wrong `= false` in method body (not `@Published` init) |
| `overlay_dismiss_restore` | `blocked_overlay` | finish handler missing dismiss assignment |
| `control_enable` | presentation block | one `.disabled(true)` on control |

If operator misses → LLM gets graph neighborhood + fix plan (≤2 shots).

Module: [`scripts/repair_operators.py`](../scripts/repair_operators.py)

## What “all OSS apps” means

Works **without retraining per app** when:

1. App exposes stable `accessibilityIdentifier`s (or labels on exercise steps)
2. Bug is one of the closed failure classes above
3. Broken tree still contains the types/composition needed to infer the fix site

Does **not** claim: arbitrary logic bugs, deleted types, blind bugs with zero AX signal, or vision-only UIs.

Measurement tiers:

| Tier | Meaning |
|------|---------|
| L2\* | Instrumented frozen suite — regression for architecture |
| L3 | Sealed held-out pack — same code path, no co-design |
| L4 | Unknown app + unknown bug at runtime — future |

## Anti-patterns (removed from product path)

- `task["id"]` → mode overrides
- `LIGH_IDENTITY_SOURCE` healthy twin index
- Tab name priors (`"favorites"`, `"notes"` in repair_mode_from_trace)
- Filename priors in localizer
- Autopilot certify recovery
- Chasing gate pass with task-specific if/else

## Reproduce

```bash
# Architecture path (any task.json)
LIGH_TRAIL_TASK=fixtures/frozen/tasks/<task>/task.json ./scripts/gate-trail-holy.sh

# L2* regression suite
./scripts/gate-trail-holy-multi.sh
```
