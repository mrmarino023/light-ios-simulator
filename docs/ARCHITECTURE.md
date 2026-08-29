# LIGH architecture — universal motor, agent-first

This doc is the **design contract**. Motor regression fixtures (LighFixture, LighFeed, …) exercise the pipeline; they are **not** generalization evidence. See [`gate-external-apps.sh`](../scripts/gate-external-apps.sh).

## Product wedge

> A coding agent gives LIGH an **app-level goal** and receives **structured, verifiable** outcomes — not opaque simulator manipulation.

## Four layers

| Layer | Responsibility | Agent-facing |
|-------|----------------|--------------|
| **L1 Session** | `lighd`, sim boot, HID, app install/swap; physical DevDriver hub | `ligh_up`, `ligh_ready`, `ligh device wait` |
| **L2 Perception** | AX dump, scene, overlay, `actionable_topk`, `rank_candidates`, `screen_sig` | `ligh_observe` |
| **L3 Motor** | ready → resolve → clear_path → **fire+verify** → settle; physical = **WDA arms** | `reach`, `dismiss_overlay`, taps via `app_goal` |
| **L4 Goal** | Declarative setup + postconditions | `ligh_cap_app_goal`, `ligh_cap_app_job` |

### Physical split (owned Expo / Debug apps)

| | Path | Notes |
|--|------|-------|
| Eyes | `@mm-labs/ligh-expo` DevDriver | Fast in-app AX over LAN |
| Hands | Appium → WebDriverAgent | System events; required for RN tab bars |
| Verify | `screen_sig` before/after | Fail closed if unchanged |

Details and device prove results: [`PHYSICAL.md`](PHYSICAL.md).

**Human motor (target):** cognition layer (settle judge, probe planner, universal search) + full gesture vocabulary — see [`HUMAN_MOTOR.md`](HUMAN_MOTOR.md).

## Motor pipeline (invariant for every app)

```
ready → perceive → ensure_path → fire_verified → settle
```

### clear_path (overlay FSM)

- **Keyboard** — dismiss before tapping chrome under keys
- **Sheet / alert** — do **not** auto-dismiss if target lives on the overlay; fire AX-first
- **System surface** — foreign-process occlusion (auth / share / permission / …). Discover by **hit-test**, classify by role, motor policy from role table — never Safari-only special cases
- **Transition** — wait, do not tap

### Certify artifact (competitive product surface)

Every `ligh_test` writes **`.ligh/last-certify.json`**:

```json
{ "ok", "fault", "fault_owner", "process_health", "system_surface", "overlay", "screen_sig", "trail_allowed" }
```

Agents and CI consume this file — not screenshots.

### Session hard-gate (before discover / test / TRAIL)

```text
process_health.running?
  no + crashed_recently → app_crashed (refuse TRAIL / harness discover_extended)
  no                   → app_not_running
  yes                  → proceed
```

TRAIL may edit Swift **only** when `trail_allowed: true` (app alive + app-owned fault).

### Stranger KPI

Primary: **`tier_b_verify_pass`**. Tier C cold build is benchmark/skip only — never inflates `holy_shit`.


### fire_verified (no fake ok)

Strategies tried in order until **observable UI change** or exhausted:

1. AX press (sheet/alert or fallback)
2. HID tap
3. HID hold
4. AX press fallback

If all fire but UI unchanged → fault **`motor_no_effect`** (not `ok: true`).

Verification signals: overlay change, screen title change, sheet dismissed, target id left viewport, AX identifier set changed, new sense events.

### reach (host-owned)

`reach(id|label)` = dismiss + scroll + wait until target is on a clear path. Agents should prefer this over manual swipe loops.

### scroll_until

Success when target is **on-screen** (`find_onscreen_id_in_dump`), not merely present in a virtualized AX tree.

## Fault taxonomy (fail-closed)

| fault | Meaning |
|-------|---------|
| `ok` | Postcondition satisfied |
| `target_missing` | Not reachable — read `evidence.candidates` |
| `motor_no_effect` | Fire ack'd but UI unchanged — try `reach`, AX, or fix app |
| `blocked` | Overlay could not be cleared |
| `wrong_surface` | Wrong app in foreground |
| `infra` / `eyes_unusable` | Call `ligh_ready` |
| `app_crashed` | Process dead + recent `.ips` — open DiagnosticReports / atos (not “no chrome”) |
| `app_not_running` | Expected bundle not in sim launchctl |
| `timeout` | Budget exhausted |

### System surface (foreign-process overlays)

Observe stamps `system_surface` + `overlay: system_surface` when AX came from a process other than the host app (hit-test first; UIService catalog classifies role). Motor uses role policy — auth never auto-dismisses.

## What counts as evidence

| Tier | Gate | Claim allowed |
|------|------|----------------|
| **Motor regression** | `gate-workflow-matrix.sh` | Motor ops work on known fixtures |
| **Third-party frozen** | `gate-external-apps.sh`, rigor N=50 | Generalization (per app, publish failures) |
| **Agent loop** | `gate-autonomous-agent.sh` | Agent can close the loop on structured faults |
| **Developer trial** | Human + agentic baseline A/B | Product wedge |
| **Customer discovery** | "How does your agent use Simulator?" | Segment exists or not |

**Do not** conflate motor regression 25/25 with “works on any app.”

## GoalSpec acceptance (L5)

Acceptance is **identity**, not a string-shape guess:

| Field | Meaning |
|-------|---------|
| `id` | Exact accessibilityIdentifier / tree id / tab alias |
| `label` | Visible accessibility label only |
| `identity` | Needle on the full AX surface (`identifier ∪ label ∪ text ∪ tab alias`) |

Frozen tasks that stuff identifiers into `must_see_labels` compile those tokens to `identity`. The host never decides “looks like camelCase ⇒ id”.

Acceptance is evaluated on the **on-screen** AX / WorldModel surface only. Off-screen leftovers from prior views must not satisfy `all` or poison `none`.

Wire contract is **typed** (`PilotGoal` on `DaemonRequest::Autopilot`, not raw `Value`). `GoalPredicate` uses `deny_unknown_fields`: a host that does not know `identity` must error at the RPC boundary instead of silently turning `{identity:…}` into `{}` and thrashing forever.

One resolver in `ligh-core`: `node_matches_identity_needle` — used by autopilot + verify.

### Session quarantine (L0.5 — non-negotiable)

Killer / multi-app loops **failed** when bootstrap treated `¬SpringBoard ⇒ foreground_ok`. Mae (or any other app) satisfied that check; preconditions then ran on the wrong UI.

**Invariant:** before perceive predicates, exercise, or GoalSpec certify:

`owned ⇔ (observed_bundle == expected_bundle) ∨ (task ownership markers ⊆ AX keys)`

Otherwise `wrong_surface` / bootstrap fail — never soft-ok. Harness: `scripts/killer_loop_verify.py` (`quarantine_bundles`, `surface_owned`, `bootstrap_app`).

Killer / TRAIL gates **must** run with `LIGH_UI=sim` so a live physical DevDriver (Mae on LAN) cannot steal AX under `auto`. `gate-killer-loop.sh` restarts `lighd` with that env by default.

**Hard isolation (daemon):** under `LIGH_UI=sim`, `lighd` must not register HybridPhysical, must not warm WDA from `~/.ligh/wda.env`, and must not copy a LAN DevDriver `bundle_id` into `observe.app_bundle_id` / `expected_bundle_id`. AX + identity are CoreSimulator-only. Harness quarantine alone is insufficient if metadata still says Mae while AX keys are Kix.

## Scene IR (agent perception transform)

Feel atoms are not enough for an LLM: they lack topology. **Scene IR** projects on-screen AX atoms into a fail-closed `SceneDigest`:

1. **Partition** — every framed on-screen atom ∈ exactly one region  
2. **Fit** — `row|col|grid|radial|chrome_band|strip_h|strip_v|point|flat` only if residual ≤ ε(kind)  
3. **Digest** — enum-only wire (≤3KB): regions, dynamics, salience≤8, `motor.allowed`

Under-claim over over-claim: bad fit → `flat` with `confidence=0`. No per-app templates, no vision. Built in `ligh-core::scene`, attached on every `FeelIR` as `scene`, exposed via `feel_agent_view`.

## Speculative navigation (optimistic agency)

Agents may **predict** the next `SceneDigest` / GoalSpec while animations run (speculative decoding for UI), but may **certify** success only when settle evidence matches. Conditions C1–C10 (deterministic Scene IR, fail-closed ε, stable identity, observable settle, transition model T, host-side `match`, atomic rollback, single outstanding fire, certify timeout) are required for soundness. Default speculation level: GoalSpec predicates (L0) + region enter/exit (L1). Full Scene′ (L2) only with a warm transition prior. Claim/`reached` only in state `Certified` — never while `Speculative`.

Implemented in `ligh-core::speculate` + Autopilot loop: `predict_after_act_on_feel` → `begin_speculate` (+ same-screen `preplanned` act) → poll `certify` → on `Certified`, fire preplanned if still valid (`act_valid_on_feel`). **L1 `expect_enter`/`expect_exit` bind certify** (not decorative). Ablation: `LIGH_SPECULATE=0` disables optimism; every autopilot result includes `speculate_stats`. Gate: `scripts/gate-speculate-ablation.sh` (H0 vs H1 on the same frozen goal — publish nulls).

Trace: `speculate_begin` / `speculate_preplan` / `speculate_fire_preplanned` / `speculate_end`. Agent wire (`feel_agent_view`) is Scene-IR-first (`perception: scene_ir`).

## Agent loop

```
observe → app_goal | reach | (edit + build) → app_goal → done
```

Rules:

- No screenshots on the happy path
- Never claim success without LIGH `ok: true`
- On `target_missing` / `motor_no_effect`: read `evidence`, then `reach` or fix source

## Repair plane (L5–L8) — generalization

Motor + GoalSpec (L0–L4) win when the path is reachable. **Generalization fails** when the host
proves a bug and a coding agent must fix source — that is a separate plane, not more diagnosis strings.

| Layer | Responsibility | Agent-facing |
|-------|----------------|--------------|
| **L5 RepairContract** | Failure mode → invariant, edit scope, oracles, WorldModel evidence | `repair_contract` on every `run_goal` failure |
| **L6 Repo coupling** | Static domains (TabView, Auth gate, overlay) from repo layout | `scope.edit_globs`, `scope.forbidden_globs`, `scope.primary_path` |
| **L7 Scoped patch compiler** | LLM edits **inside** contract scope only; same IR as motor | `write_file` rejected outside scope |
| **L8 Verify farm** | Up to k scoped candidates → build → `run_goal` → strict harness | `verify_farm` (one LLM turn, host-owned) |

### RepairContract wire

On autopilot failure the daemon emits:

- `mode` — closed enum (`tab_chrome_missing`, `state_gate_stuck`, …)
- `scope` — where the agent may edit (derived in Rust, not inferred by LLM)
- `evidence` — `feel_agent_view` + missing identities + tab chrome
- `oracle_pre` / `oracle_post` — what must become true

Implemented in `ligh-core::repair`, attached in `pilot_cap` failure payloads.

### Product claim (generalized)

**Product claim** requires frozen killer suite **≥2/3** with autopilot `claim_pass` **and** wall time
≤ vision on each task — not login-only. Task shapes:

| Shape | Motor | Repair plane |
|-------|-------|----------------|
| Form/login | Autopilot path discovery | `StateGateStuck` → Auth/AppState |
| Tab/composition | Autopilot after login | `TabChromeMissing` → Navigation/TabView |
| Overlay/onboarding | `reach` + dismiss | `BlockedOverlay` → finish handler |

Scene IR / speculate remain **motor latency** (L2.5), not the competitive wedge for repair.

### Agent protocol

- Scored/honest A/B: baseline gets vision only; autopilot gets **RepairContract + evidence** (not stripped).
- `LIGH_REPAIR_FARM=1` (default): `verify_farm` enabled on autopilot arm.

## TRAIL repair architecture (replaces L9–L11 product path)

**TRAIL** = Trace-Repair with Autopilot Identity Localization.

Literature synthesis: mobile repair wins when **interaction traces localize**, **static graphs scope**,
**1–2 bounded LLM fixes** certify on the **same trace** — not multi-turn chat or frozen golden diffs.
See [References](#references-literature) below.

**Target:** ≤120s repair wall on ≥2/3 frozen killer tasks without golden reverse.

### Architecture invariants

| Invariant | Enforcement |
|-----------|-------------|
| Broken-tree only | `trail_holy` indexes `task.source_root` after inject; gates unset `LIGH_IDENTITY_SOURCE` |
| Mode from trace | Effect Classifier on TraceFailure — never `task["id"]` |
| Missing identity | `localize_missing_tab` via TabView + observed sibling tabs |
| Dead control / overlay | control → ObservableObject writer / overlay host |
| Hard certify | exercise + postconditions only |
| No spoilers | gate strips `// BUG:` after patch |

Full writeup: [`TRAIL_BULLETPROOF.md`](TRAIL_BULLETPROOF.md)

### Measured today

| Task | Arm | Wall | Pass | ≤120s |
|------|-----|------|------|-------|
| login-never-navigates | **TRAIL** | **33s** | ✓ | ✓ |
| onboarding-home-broken | **TRAIL** | **64s** | ✓ | ✓ |
| kix-notes-tab-missing | **TRAIL** | **78s** | ✓ | ✓ |
| login-never-navigates | autopilot chat | 61s | ✓ | ✓ |
| login-never-navigates | vision chat | 622s | ✗ | ✗ |
| kix-notes-tab-missing | autopilot chat | 644s | ✓ | ✗ |
| kix-notes-tab-missing | vision chat | 460s | ✓ | — |
| motor generality | autopilot 0 LLM | median **11.5s** / 6 apps | ✓ | motor only |

Full compare: [`assets/trail-holy-compare-latest.json`](assets/trail-holy-compare-latest.json) · writeup: [`TRAIL_RESULTS.md`](TRAIL_RESULTS.md)

**Pattern:** win when host **drives UI + scopes edit + ≤2 patches**. Lose when LLM **explores repo in chat**.

Product path: **`./scripts/gate-trail-holy-multi.sh`** (`scripts/trail_holy.py`). Chat agent is ablation only.

### TRAIL layers

| Layer | Name | Responsibility | LLM | Budget |
|-------|------|----------------|-----|--------|
| **R0** | App Repair KB | Identity index + SwiftUI view-graph (cold, once per app) | 0 | offline |
| **R1** | Trace Oracle | Autopilot exercise → structured `TraceFailure` | 0 | 20s |
| **R2** | Hybrid Localizer | AX id → file:line + **view ascent** (TabView/Router parent) | 0 | 2s |
| **R3** | RepairContract v2 | `mode` + `scope` + **`oracle_trace`** (not label-only) | 0 | 1s |
| **R4** | Constrained Fixer | Mode fix-plan + localized snippet ±40 lines → full file block | 1 call | 15s |
| **R5** | Trace Certify | Same trace re-run inside `lighd` (no Python MCP harness) | 0 | 20s |
| **R6** | Bounded ReFix | Shot 2 only if certify fails; harness fault as feedback (ChatRepair-style) | 1 call | +30s |

Hot wall budget (infra excluded): prove 20 + patch 15 + build 35 + certify 20 + slack 30 = **120s**.

### TraceFailure wire (TaskAudit-inspired)

Errors are **functiona11ity** — visible only after interaction. Oracle is the trace delta, not a static screenshot.

```json
{
  "step": 4,
  "action": "tap",
  "expected_identity": "tab_notes",
  "observed_identities": ["tab_home", "tab_favorites"],
  "scene_before": "chrome_band|tabs_2",
  "scene_after": "chrome_band|tabs_2",
  "fault": "target_never_visible",
  "motor_evidence": { "fp_before": "…", "fp_after": "…" }
}
```

Fixer input = `TraceFailure` + fix plan from `RepairMode`, not prose task prompt.

### Hybrid localization (FixAlly-inspired)

Identity index alone is insufficient (Kix: `tab_notes` declared in `NotesView` but bug is **TabView composition** in `MainTabView`). Localizer must:

1. Map `accessibilityIdentifier` → declaration site (static index).
2. Ascend SwiftUI view graph to **composition owner** (`TabView {`, router, gate handler).
3. Emit `scope.primary_path` at composition layer, not leaf view.

### Product pipeline (in-daemon)

```
cap_repair_job(task)
  → trace_exercise (autopilot, 0 LLM)
  → trace_failure + repair_contract
  → hybrid_localize (KB)
  → fixer_shot_1 (optional LLM, scoped snippet)
  → incremental_build
  → trace_certify (same steps, in-process)
  → [fixer_shot_2 if fail] → build → certify
```

Implement in `ligh-core::repair` + `pilot_cap`; retire Python orchestrator for gates.

### Claim ladder (honest)

| Level | Criterion | Status |
|-------|-----------|--------|
| **L0 Motor** | Autopilot generality 6/6 apps, 0 LLM | ✅ |
| **L1 Narrow repair** | One frozen task ≤120s, ≥3× vision, no UI LLM | ✅ login |
| **L2 Generalized repair** | ≥2/3 frozen verified ≤120s, no `bug_patch`, ≤2 LLM shots | ✅ **3/3 ≤120s** (login 41s, onboarding 67s, Kix 91s) |
| **L3 OSS unknown** | Blind apps, no golden, ≥50% pass | ❌ not measured |

**Published:** `docs/assets/trail-holy-multi-latest.json` — `tasks_verified: 3`, `tasks_holy_shit: 3`, `claim_pass: true`.

Gate: `./scripts/gate-trail-holy-multi.sh` (orchestrator `scripts/trail_holy.py`).

### Deprecated — do not ship as product proof

| Path | Why |
|------|-----|
| `golden_diff` / `patch -R bug_patch` | Frozen cheat — knows answer |
| `frozen_fast` (identity from `task.json`) | Frozen cheat |
| L11 hardcoded tab templates | Per-app hack |
| Python `strict_verify` on gate hot path | 100s+ MCP tax + flake |
| Chat loop >2 repair shots | Contradicts bounded APR literature |

### Still delete from product path

- Multi-turn chat loop for killer proof
- `sim_clean_reboot` per task
- Full-repo `read_file` before patch — host sends file slice only

## Performance targets (agent session)

- Hot path via `lighd` only
- Adaptive settle (stop when `settled && actionable`)
- Per-app relaunch, not full sim reboot between jobs
- Slim evidence to LLM (topk ≤ 8, candidates ≤ 5)

## Next experiments (priority order)

Research order (falsifiable — publish failures):

1. **`cap_repair_job` in-daemon** — trace prove + certify in Rust; kill Python MCP harness on hot path.
2. **SwiftUI view-graph ascent** — hybrid localizer (identity index + composition owner).
3. **`TraceFailure` wire** — emit from autopilot trace; Fixer + gate consume it.
4. **Constrained Fixer** — 1–2 shots, mode fix-plan, scoped snippet (FixAlly/ChatRepair bounded loop).
5. **Gate TRAIL** — `./scripts/gate-trail.sh` / `gate-trail-multi.sh`; login + kix + onboarding localize within prove budget; full verify when R4–R5 land.
6. **Speculate ablation** — `gate-speculate-ablation.sh`, N≥3 (motor latency only).
7. Developer trials — [`DEVELOPER_TRIAL.md`](DEVELOPER_TRIAL.md)
8. Add external apps to `fixtures/external-apps/manifest.json` — **no source edits**

## References (literature)

External papers informing TRAIL (not prior art claims — design inputs):

| Paper | Venue | Relevant idea for LIGH |
|-------|-------|------------------------|
| [FixAlly](https://arxiv.org/abs/2408.03827) | arXiv 2024 | Plan → localize → fix; identity → view hierarchy ascent; assess with same GUI test; ≤3 refine loops |
| [TaskAudit](https://arxiv.org/abs/2510.12972) | CHI 2026 | Functiona11ity errors via interaction traces; executor + analyzer on trace outcomes |
| [ChatRepair](https://lingming.cs.illinois.edu/publications/issta2024.pdf) | ISSTA 2024 | Interleave patch generation with test failure feedback; bounded conversation |
| [FLAMES](https://arxiv.org/abs/2410.16655) | arXiv 2024 | Semantic test-guided patch search; best-first with validation feedback |
| [AppAgent](https://arxiv.org/abs/2312.13771) | arXiv 2023 | Explore-once knowledge base; discrete AX-grounded action space |
| [Mobile-Agent-v2](https://arxiv.org/abs/2406.01014) | NeurIPS 2024 | Planner / decision / reflection split; reflection for ops not repo edits |

**LIGH differentiator vs FixAlly:** native AX motor (~52ms observe p50) replaces XCTest crawl; same trace detects and certifies (TaskAudit executor + oracle in one host).

**Do not copy from vision agents:** screenshot-first navigation on the repair path — login A/B shows AX autopilot wins when UI is keyed.
