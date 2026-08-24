# Validation Week

The goal of this week is not to add features. The goal is to test whether Host
Autopilot remains the right architecture when we push it beyond the current
happy path.

## How to run

```bash
./scripts/validation-week.sh              # coverage vs the minimum bar
./scripts/validation-week.sh ingest       # import already-published artifacts
./scripts/validation-week.sh smoke        # generality gate (zero LLM tokens)
LIGH_VW_REPEAT=5 ./scripts/validation-week.sh paired   # needs OPENAI_API_KEY
python3 scripts/validation_week.py validate
./scripts/validation-week.sh summarize
```

Matrix: [`fixtures/validation-week/matrix.json`](../fixtures/validation-week/matrix.json).  
Outputs: [`docs/assets/validation-week-summary.json`](assets/validation-week-summary.json) ·
[`docs/VALIDATION_WEEK_RESULTS.md`](VALIDATION_WEEK_RESULTS.md).

`paired` is honest protocol for both arms: same task, bug, model, acceptance
target and strict harness. Autopilot gets `run_goal`; vision still drives every
tap. Neither arm receives a step list.

## Benchmark artifact contract

- Frozen tasks use protocol v2 and are validated against
  [`fixtures/frozen/task-v2.schema.json`](../fixtures/frozen/task-v2.schema.json)
  before the harness resolves any paths.
- Ledger rows use schema v2 at
  [`docs/assets/validation-week-run.schema.json`](assets/validation-week-run.schema.json).
  A row is appended from exactly one raw per-run artifact and records its path
  and SHA-256. Existing rows are never overwritten.
- Aggregate summaries are never expanded into trial rows. Legacy rows remain
  readable for compatibility but are unscored.
- Scored metrics join exactly one Autopilot and one baseline row on
  `(task, repeat_index)`, then require matching model, task-prompt hash, and
  protocol version. Incomplete or ambiguous pairs are reported and excluded.
- `minimum_bar.paired_min_n` controls the minimum complete-pair count. Below
  that N, descriptive metrics may be shown, but the reporter refuses a claim.
- `exercise_app` is a protocol violation in scored runs. Host `source_hint`,
  `coaching`, and `suggestion` fields are recursively stripped before either
  arm receives tool output.
- Raw runs log protocol/version, model, task and system prompt hashes, git SHA,
  and structured failure phase/class. Missing-artifact failures are preserved
  as raw infra-failure artifacts before ledger ingestion.

Default paired task is `login-never-navigates`. Expand with:

```bash
LIGH_VW_REPEAT=5 LIGH_VW_TASKS="login-never-navigates login-button-disabled login-delayed-ui" \
  ./scripts/validation-week.sh paired
```

## Rule zero

Do not redesign the system during validation week.

Only allow:

- bug fixes
- instrumentation
- new tasks
- new apps
- better artifact publishing

Do **not** add new product concepts until the current claim has either survived
or failed.

## Current claim under test

> For coding-agent iOS bugfix tasks, delegating UI operation to a deterministic
> host can make the fix-run-verify loop materially faster and cheaper than
> letting the LLM drive every UI step.

## Success criteria

By the end of the week, publish a report with:

- median wall time
- p90 wall time
- median LLM tokens
- pass rate
- top-1 patch acceptance
- failures grouped by diagnosis class

The claim gets stronger if:

- Autopilot keeps a **>=2x median wall-time win** vs the vision baseline
- Autopilot keeps a **clear median token win**
- the pass rate does not regress materially
- failures are understandable and mostly planner / app issues, not chaos

## Matrix to run

### Minimum bar for the week

Do not call the week complete unless all four of these are true:

- at least **3 apps** total, with at least **2 third-party apps**
- at least **5 tasks per app**
- at least **5 paired runs** on the most important tasks
- a published summary with medians, p90s, pass rates and failure classes

That is the minimum dataset. More is better, but less is not enough to support
the product claim.

### 1. More tasks per app

Target: **5 to 10 tasks per app**.

Each app should include:

- happy-path completion
- validation error
- retry after wrong input
- state recovery after overlay / modal
- navigation to a second screen and back

For login-style apps, add:

- wrong password then correct password
- disabled CTA becomes enabled
- success screen verification

### 2. More third-party apps

Target: **at least 3 additional third-party iOS apps** not designed for LIGH.

Selection criteria:

- buildable locally
- distinct UI patterns
- reasonable accessibility
- no per-app flow scripting

Good diversity targets:

- settings-heavy app
- commerce or catalog app
- note / productivity app

### 3. Real bugfix tasks

Target: **at least 10 non-trivial bugfix tasks**.

Prefer bugs such as:

- wrong navigation outcome
- disabled or miswired CTA
- validation state mismatch
- missing state update after async action
- modal dismissal / presentation bug

Avoid tasks that are mostly compile errors or obviously searchable string typos.

### 4. Repetitions

Target: **5 to 10 paired runs per task** where practical.

For every paired run:

- same task
- same model
- same bug
- same acceptance target
- same strict verifier
- clean simulator state

Publish distributions, not only best runs.

## Recommended weekly sequence

### Day 1: Freeze + instrumentation

- freeze architecture
- finalize artifact schema
- ensure every run captures wall time, tokens, pass/fail, diagnosis, app, task,
  model and git sha
- make failures machine-groupable

### Day 2: Expand tasks on existing apps

- add tasks first on the apps that already pass
- do not add new features while doing this
- use these runs to discover missing diagnosis classes

### Day 3: Add third-party app 2

- choose an app not designed for LIGH
- add 5 tasks
- run small smoke batches before spending on repeated paired runs

### Day 4: Add third-party app 3

- same process as Day 3
- prefer a different UI shape from the previous app

### Day 5: Repeated paired runs

- spend most of the budget here
- run 5 to 10 repeats on the most representative tasks
- publish distributions, not anecdotes

### Day 6: Failure analysis

- group failures by dominant class
- separate planner issues from bad source fixes and infra flake
- only fix bugs that improve the validity of the measurement

### Day 7: Publish

- write the markdown report
- include what got worse, not only what improved
- decide whether the claim got stronger, weaker or falsified

## Suggested minimal matrix

If time is constrained, use this exact floor:

- **App A:** existing fixture app (`LighOnboard`), 5 tasks
- **App B:** existing third-party app (`XCUITestDemo`), 5 tasks
- **App C:** new third-party catalog app (`Kix` / byKosta/Kix-app), 5 tasks across
  auth / notes / favorites / cart — not five login clones

Kix is vendored at `fixtures/third-party/Kix` (commit `0bc85035`). Build with
`./scripts/build-kix.sh`. Frozen tasks:

- `kix-login-never-authenticates` — auth session never opens
- `kix-notes-tab-missing` — tab navigation
- `kix-notes-add-noop` — notes CRUD
- `kix-favorites-tab-missing` — tab navigation
- `kix-cart-tab-missing` — tab navigation

These are five **different flow shapes**, not five login clones of XCUITestDemo.

Autopilot happy-path smoke on Kix is **green**: login → Home tab (`tab_home`)
in 3 steps / 12.7 s / 0 LLM tokens on a clean sim. `kix` is in the default
generality app list. Frozen bugfix tasks (missing tabs, noop Add, auth that
never opens) still belong in the matrix — failures there are signal.

```bash
LIGH_PILOT_APPS=kix ./scripts/gate-autopilot-generality.sh
```

Within those 15 tasks:

- at least 5 should be happy-path completions
- at least 5 should include an error or recovery state
- at least 5 should require a real source fix rather than pure navigation

Then choose the top **6 tasks** and run **5 paired repeats** each.

That yields a compact but meaningful dataset:

- **15 total tasks**
- **30 repeated paired runs**
- coverage of both controlled and uncontrolled apps
- coverage of both nominal and failure states

## Artifacts to publish

For each experiment family, publish:

- raw JSON artifact
- summary JSON with medians and pass rates
- short markdown note with what broke

Recommended top-level outputs:

- `docs/assets/validation-week-summary.json`
- `docs/assets/validation-week-runs.json`
- `docs/VALIDATION_WEEK_RESULTS.md`

Each run row should include at least:

- `app`
- `task`
- `arm`
- `model`
- `git_sha`
- `wall_time_ms`
- `llm_tokens`
- `pass`
- `diagnosis_class`
- `patches`
- `builds`
- `run_id`
- `repeat_index`

## Failure taxonomy

Every failed run should map to one dominant class:

- planner chose the wrong path
- perception mislabeled the UI
- app state changed asynchronously
- harness expectation too strict or wrong
- source fix was wrong
- simulator / infra flake

If a failure does not fit cleanly, improve diagnosis before adding more product
surface.

Good reports should answer:

- Did Autopilot fail because it used the app badly?
- Or did the model produce the wrong code fix?
- Or did the benchmark itself become invalid because of infra or hidden coaching?

## Stop conditions

Pause expansion and reassess if any of these happen:

- median speedup falls below **1.5x**
- token win disappears
- pass rate drops below the baseline on repeated runs
- success depends on hidden task coaching
- many failures require per-app heuristics

## Best possible outcome

At the end of the week, the repo should support this statement with evidence:

> Host Autopilot is not just faster on one demo. It is a repeatable advantage on
> a growing set of real coding-agent iOS bugfix loops.
