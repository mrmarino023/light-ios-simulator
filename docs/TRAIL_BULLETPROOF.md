# TRAIL bulletproof — architecture reassessment

Last updated: 2026-08-26  
Status: **product path live** — Effect Classifier + causal localize (View→VM / leaf→overlay host / TabView). Honest multi: **3/3 verified, 2/3 ≤120s** (`docs/assets/trail-holy-multi-latest.json`). Kix wall still gated by 2nd LLM shot after compile miss.

## What the honest run proved

| Task | Broken-tree result | Localize |
|------|--------------------|----------|
| **login-never-navigates** | verified **40s** · `state_gate_stuck` | `LoginViewModel.swift` via control→ObservableObject |
| **kix-notes-tab-missing** | verified **126s** (over budget) · `tab_chrome_missing` | TabView composition (no filename prior) |
| **onboarding-home-broken** | verified **64s** · `blocked_overlay` | `OnboardingView.swift` via leaf→host + presentation Binding |

Smoking-gun TraceFailure (login) now classifies before localize:

```text
step 3 tap loginButton
fault: motor_failed          ← misclassified; control still on-screen
observed: … loginButton, usernameTextField, passwordSecureField, Welcome …
expected post: homeTitle     ← never evaluated; exercise aborted on tap fault
```

Bug is `isLoggedIn = false` in ViewModel. System edited the **view that declares the button**, not the **state the button writes**. That is the architectural hole — not “need more heuristics on task id”.

---

## Failure taxonomy (must drive design)

| Class | Symptom in TraceFailure | Correct RepairMode | Correct localize target |
|-------|-------------------------|--------------------|-------------------------|
| **A. Missing chrome** | expected `tab_*` absent from AX; Tab Bar siblings present | `tab_chrome_missing` | TabView composition that hosts observed siblings, **missing** expected |
| **B. Dead control** | control **still visible** after tap; fingerprint/scene unchanged | `state_gate_stuck` | Writer of gate state bound to that control (ViewModel / `@Published`), **not** the Button’s view file |
| **C. Stuck overlay** | overlay/sheet still blocking; finish control fired | `blocked_overlay` | Finish handler / Bool that keeps overlay presented |
| **D. True missing target** | expected id never in AX and not a tab sibling case | `target_never_visible` | Declaration site or composition that should expose it |

Today’s mapper used to collapse B into `unknown` when fault=`motor_failed`, then localize the leaf. **Bulletproof path classifies B before localize** (`motor_no_effect` → `state_gate_stuck` → ObservableObject writer).

---

## Bulletproof stack (replace current R0–R6)

```text
R0  Structural KB (cold, per app, from BROKEN tree only)
      identity index + call graph: View ⇄ ViewModel ⇄ @Published / bindings
R1  Trace Oracle (richer than today’s TraceFailure)
R2  Effect Classifier  → RepairMode   (NEW — before localize)
R3  Causal Localizer   → primary_path (mode-specific ascent)
R4  Scoped Fixer       ≤2 shots, compile-gated
R5  Same-trace Certify hard fail-closed
R6  Wall governor      prove/fix/build/certify budgets with early abort
```

### R1 — TraceFailure v2 (required fields)

```json
{
  "step": 3,
  "action": "tap",
  "control": "loginButton",
  "control_still_visible": true,
  "expected_identity": "loginButton",
  "acceptance_pending": ["homeTitle", "Home"],
  "observed_identities": ["…"],
  "screen_sig_before": "…",
  "screen_sig_after": "…",
  "sig_changed": false,
  "fault": "motor_no_effect",
  "scene_before": "…",
  "scene_after": "…"
}
```

Rules:

- If tap ACK and `control_still_visible` and `!sig_changed` → fault **must** be `motor_no_effect` / `control_fired_no_transition`, never opaque `motor_failed`.
- Exercise must optionally continue to **acceptance probe** after a dead control (one perceive) so `acceptance_pending` is filled — without claiming prove success.

### R2 — Effect Classifier (pure function of TraceFailure v2)

```text
if expected.startswith("tab_") and expected ∉ observed and "Tab Bar"∈observed
    → tab_chrome_missing
elif control_still_visible and not sig_changed and action==tap
    → state_gate_stuck
elif blocked / overlay atoms persist after finish-like control
    → blocked_overlay
elif expected ∉ observed
    → target_never_visible
else
    → unknown  (refuse to edit; do not guess ContentView)
```

**Refuse `unknown` edits.** Returning localize_failed is better than a wrong file.

### R3 — Causal Localizer (per mode)

| Mode | Algorithm (broken tree only) |
|------|------------------------------|
| `tab_chrome_missing` | Keep current: TabView files scored by observed sibling `tab_*` + `.tabItem`; prefer files **missing** expected id |
| `state_gate_stuck` | (1) Find files declaring `control` AX id. (2) Parse SwiftUI bindings / `action:` / button handlers. (3) Resolve referenced `ViewModel` / `@ObservedObject` / `@StateObject` types. (4) Primary = type that **writes** a `@Published` Bool/enum used by root navigation. Prefer writer over view. |
| `blocked_overlay` | Control site → handler → `@Published` / `isPresented` / `fullScreenCover` source |

No filename bonuses (`LoginViewModel`, `MainTabView`, `OnboardingView`).

Minimum viable for login (no full Swift parser yet):

1. Files containing `loginButton` (or control id).
2. Extract nearby type names (`LoginViewModel()`, `@StateObject`, `@ObservedObject`).
3. Open those `.swift` files; score `@Published` assignments in methods (not the View body).
4. Pick highest score — empirically lands on ViewModel for XCUITestDemo.

### R4 — Fixer constraints (stop compile thrash)

- Scope **one file** = `primary_path` only; forbid deleting types referenced by App entry (`ContentView` must remain if App references it).
- Pre-flight: `swift` syntax brace balance + **symbol retention check** (if file was exporting `struct ContentView`, candidate must still define it — or reject shot).
- `max_completion_tokens` sized to file length; reject truncated outputs (already partial).
- Shot 2 only on compile error or certify fail with harness fault text.

### R5 — Certify

Unchanged contract: same exercise + postconditions. No autopilot recovery.

### R6 — Wall governor (hit ≤120s without cheats)

Budget (hot, infra excluded, published as such):

| Phase | Cap | Notes |
|-------|-----|-------|
| Prove | ≤25s | install-once + settle caps; classify dead-control without long retries |
| Localize | ≤2s | pure static |
| Fix+build | ≤45s | 1 shot preferred; 2nd only if compile fail |
| Certify | ≤35s | install fixed once + exercise; no 90s autopilot |
| Slack | ≤13s | |
| **Total** | **≤120s** | |

Kix at 179s lost on prove (~58s) + 2 builds + certify. Governor must **fail-soft**: if prove >25s still continue, but optimize settle; if shot1 compiles, never burn shot2.

---

## What stays deleted (non-negotiable)

| Anti-pattern | Why |
|--------------|-----|
| Healthy BACKUP identity index | Soft golden for missing ids |
| Mode from `task["id"]` | Suite overfitting |
| Filename priors | Per-app templates |
| `// BUG:` as signal | Spoiler |
| Autopilot certify soft-pass | Fake verify |
| Golden `patch -R` | Answer key |

---

## Implementation order (to regain a real claim)

1. **Motor fault mapping** — map dead-control taps → `motor_no_effect` + `control_still_visible` in attempt/prove (daemon + harness).
2. **Effect Classifier** in `ligh-core::repair` + Python parity; unit tests from the three TraceFailure shapes (no task ids).
3. **Causal localizer for state_gate** (View→ViewModel ascent); unit test: XCUITestDemo broken tree → `LoginViewModel.swift`.
4. **Fixer symbol-retention + reject unknown mode**.
5. **Wall governor** on prove/certify settle.
6. **Re-run multi** under broken-tree protocol; publish PASS or FAIL without editing the suite.
7. **Held-out** favorites/cart + one new app — only after multi is green under (1–5).

Pass criterion for “bulletproof L2”:

- Same 3 tasks, **broken-tree only**, no spoilers, hard certify  
- ≥2/3 verified ≤120s  
- Login primary_path ends with `LoginViewModel.swift` (or equivalent gate writer)  
- Kix primary_path is TabView composition  

L3 (unknown apps) remains a separate sealed protocol after L2 is honest-green.

---

## Claim language (until then)

> TRAIL is being rebuilt for causal localize from TraceFailure.  
> Prior 3/3 ≤120s used contaminated oracles and is **withdrawn** as a generalization claim.  
> Next published number is from this contract or is marked FAIL.
