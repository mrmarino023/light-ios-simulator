# Milestone: Host Autopilot closes the loop

This is the first LIGH milestone that clearly matches the product promise.

## The shift

Before:

> LIGH makes Simulator actions faster.

Now:

> LIGH removes UI operation from the LLM and leaves the model with the job it
> is actually good at: understanding and modifying code.

That yields the new product statement:

> **The model writes the fix. The host uses the app and proves whether the fix
> works.**

## Why this matters

Previous versions improved pieces of the stack:

- faster motors
- AX-first observation
- routed perception
- UX graph experiments
- host-side jobs

But the coding agent still had to act as the UI executor. That was the core
architectural mistake. The honest A/B v1 made this clear: removing
`exercise_app` caused both arms to fail.

Host Autopilot fixes that mistake by moving UI execution into Rust:

- the LLM reads and edits Swift
- the host launches the app, perceives UI state, navigates, types and taps
- the strict harness accepts or rejects the patch immediately

## Evidence that this is real

### Honest paired A/B v2

Same task, same model, same bug, same acceptance target, same verifier, both
arms pass:

- **LIGH Host Autopilot:** 41.9 s, 9,034 tokens
- **Vision baseline:** 152.4 s, 67,040 tokens

Result:

- **3.64x faster wall-clock**
- **7.42x fewer LLM tokens**
- **1 patch / 1 build** on both arms
- **zero LLM UI tokens** for the Autopilot executor

Artifact: [`assets/killer-loop-ab-v2-latest.json`](assets/killer-loop-ab-v2-latest.json)

### Generality gate

The same policy passes **6/6 apps** across distinct flow shapes without
per-app branches or recorded flows:

- form
- wizard
- modal
- list to detail
- third-party login
- third-party catalog + auth + tabs (Kix)

Artifact: [`assets/autopilot-generality-latest.json`](assets/autopilot-generality-latest.json)

## What this does not prove yet

This milestone proves the mechanism, not the market.

It does **not** yet prove:

- broad robustness on messy production apps
- stability across many repeated runs
- superiority across many models
- long-horizon flows with richer edge cases
- a better CI artifact story than tools like Maestro

## What to do next

Do not change the architecture for a week.

Instead, try to break it:

1. more tasks per app
2. more third-party apps
3. real bugfix tasks, not only prepared scenarios
4. repeated runs with medians and distributions

If the result survives that week, this stops being just a promising mechanism
and starts becoming a credible product wedge.
