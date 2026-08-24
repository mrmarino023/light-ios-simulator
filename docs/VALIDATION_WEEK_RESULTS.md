# Validation week results

Generated: `2026-08-24T13:42:40Z` · git `faa0b885867f`

**Week complete:** no  
**Claim stronger:** not yet
**Scored pairs:** 1 (minimum N=5)  
**Claim refusal:** paired_n_below_minimum:1<5

## Coverage vs minimum bar

- runnable tasks: 15 (planned: 9)
- runnable per core app: `{"lighonboard": 1, "xcuitestdemo": 4, "kix": 6}`
- autopilot repeats by task: `{"login-never-navigates": 1}`

Gaps:

- need 5 runnable tasks per core app; have {'lighonboard': 1, 'xcuitestdemo': 4, 'kix': 6}
- need 5 autopilot repeats on 6 priority tasks; enough on []

## Paired bugfix loop only

Only complete scored task+repeat pairs enter these medians. Navigation/generality smoke is excluded.

| Arm | Runs | Pass rate | Median wall | p90 wall | Median tokens |
|-----|------|-----------|-------------|----------|---------------|
| Autopilot | 1 | 1.0 | 41862.0 | 41862.0 | 9034.0 |
| Vision | 1 | 1.0 | 152391.0 | 152391.0 | 67040.0 |

- speedup vs vision (median wall): **3.64**
- token ratio vs vision: **7.42**
- navigation smoke (autopilot only): runs=6 pass_rate=1.0 median_wall=12053.0

## Failure taxonomy

No failed runs in the current artifact set.

## Stop conditions hit

None yet (dataset may still be too small to trigger them).

