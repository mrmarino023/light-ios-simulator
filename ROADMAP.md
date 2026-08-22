# Roadmap — bet the project on one experiment

**Thesis:** A coding agent can reliably use LIGH to verify an arbitrary iOS Debug `.app`, with fail-closed structured outcomes — and do so better than existing tools (Maestro first).

Not: “faster Simulator host.” Not: Settings LLM demos.

## The job

```text
Cursor → build .app → LIGH app-job → launch → resolve → act → settle → verify
                                                              ↓
                                                    verified | fault (explicit)
```

Agent must never get “probably tapped Login.” It gets:

```json
{ "ok": true, "fault": "ok", "capability": "app_job", "detail": { "…" } }
```

or:

```json
{ "ok": false, "fault": "target_missing", "detail": { "step": 2, "op": "tap" } }
```

**Danger ranking for agents:** wrong action ≫ slow action ≫ explicit failure.

## Kill metric (multidimensional)

`claim_pass` requires all of:

| Dimension | Bar |
|-----------|-----|
| Reliability | `pass_rate == 1.0` at publish N (currently N=50 on fixture) |
| No silent wrong-target | Every success includes postcondition wait on Done chrome |
| Explicit faults only | Failures emit `FaultClass` — never soft-success |
| Latency | workflow `p95_ms` (iters after first install) under budget |
| Recovery | Mid-flight AX/overlay recovery is host-owned (`ensure_path` / relaunch), scored in results |

Published: [`docs/assets/app-reliability-latest.json`](docs/assets/app-reliability-latest.json).

## Sequence (do not reorder / do not move goalposts)

```text
① N=50 reliability (multidimensional claim_pass)
       ↓
② Maestro bakeoff — same semantic job, same machine
       ↓
③ Third-party Debug .app (NOT LighFixture) — dogfood or narrow the claim
       ↓
④ Cold Mac < 5 min — clone → install → first app-job
       ↓
⑤ Cursor MCP — app-job first-class, structured outcomes
       ↓
⑥ 5 real users
```

If ① fails → fix or kill.  
If ② loses → understand why (publish the table).  
If ③ breaks → fix or narrow (“apps with accessibility identifiers”).  
If ⑥ gets nobody → kill the product thesis.

## Competitive bakeoff (vs Maestro)

Same app, same workflow, same Mac. User-level job first — not “our primitive vs their YAML.”

Example semantic job (fixture today; real app tomorrow):

```text
launch → Home → type → GoNext → verify Done
```

Table to publish: workflow success · explicit fail · wrong-target · AX-empty/blocked · p50/p95 action · total workflow · recovery · cold start.

Harness: `./scripts/gate-app-bakeoff.sh`

## Debug `.app` narrowing (honest)

“Simulator Debug `.app`” is a real market shrink. That is OK **only if** third-party dogfood works.

- Fixture proves the motor.
- **Third-party app proves the wedge.** Prefer an app we did not design around LIGH.
- If only identifier-rich apps work → claim must say that out loud.

## Agent recovery (wedge, not patch)

Deterministic test frameworks stop. Agents need:

```text
tap → AX empty → wait → re-observe → resolve → tap → verify
```

Host owns recovery (`ensure_ready`, overlay `ensure_path`, relaunch). Agent sees structured fault or verified — not raw CLI noise.

## MCP (not optional)

```text
Cursor → MCP → ligh app-job → Simulator → { ok, fault, detail }
```

`ligh_cap_app_job` must be first-class. Without this bridge, LIGH is another automation CLI.

## Cold Mac < 5 min

Promote over any “× vs WDA” number:

```text
git clone → ./scripts/install.sh → app-job green
```

Measurable. Developer tools die at install.

Harness: `./scripts/gate-cold-start.sh` → [`docs/assets/cold-start-latest.json`](docs/assets/cold-start-latest.json)

**Note:** build with workspace binaries (`unset CARGO_TARGET_DIR && cargo build --release`) — sandboxed builds may land in a cache dir, not `target/release/`.

## Done

- [x] Control plane: phase + overlay + `FaultClass`
- [x] Motor: `ensure_ready → resolve → ensure_path → fire → settle`
- [x] `ligh cap app-job` + fixture
- [x] Reliability gate + N=50 fixture publish (multidimensional `claim_pass`)
- [x] Maestro bakeoff — LIGH wins reliability tie + **~11× faster p50** on fixture (publish either way)
- [x] Demote SpringBoard / vision / “~4× WDA” from marketing
- [x] MCP `ligh_cap_app_job` + compact verified/fault payload
- [x] Cold Mac proof — daemon bounce → app-job **10.6s** (budget 5 min)

## Next (in order)

- [x] Third-party Debug `.app` — XCUITestDemo bakeoff published
- [ ] 5 real users

## Demote (research only — never marketing)

- SpringBoard / Settings LLM breadth
- Vision PNG bakeoffs
- Scripted “~4× WDA” microbench (footnote at most)

## Out of scope

Cloud farms · remote TCP product · physical device fleets · OCR-as-product
