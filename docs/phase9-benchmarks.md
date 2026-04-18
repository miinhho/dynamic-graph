# Phase 9 benchmark notes

Date: 2026-04-18

This note documents the larger-profile benchmark harness added for the
Phase 9 discussion:

- reusable generator/evaluator: `crates/graph-testkit/src/sociopatterns.rs`
- suitability tests: `crates/graph-engine/tests/phase9_objective_benchmark.rs`
- Criterion bench: `crates/graph-engine/benches/phase9_plasticity.rs`

## What is being measured

Two different questions are separated on purpose.

1. Pair-prediction signal quality:
   can the engine rank likely future co-attendance pairs above random?

2. Plasticity-objective suitability:
   does changing `plasticity.learning_rate` actually change the ranking used
   by the proposed objective?

The second question matters because the current Phase 9 design draft ranks
pairs by `activity`, while the current Hebbian implementation updates the
relationship `weight` slot, not `activity`.

## Larger-profile suitability results

`cargo test -p graph-engine --test phase9_objective_benchmark -- --nocapture`

- `medium` profile:
  `base_rate=0.1923`, `precision@20=1.000`, `lift=5.20`, `recall=0.659`
- `school_scale` profile:
  `base_rate=0.1403`, `precision@20=1.000`, `lift=7.13`, `recall=0.709`

Interpretation:

- The engine remains suitable for pair prediction on larger synthetic streams.
- The signal is very strong even after scaling the profile well beyond the
  original unit-test size.

## Critical Phase 9 finding

The new test `activity_ranking_is_invariant_to_plasticity_learning_rate`
shows that, for the same stream and seed, `activity`-ranked top pairs are
identical with `learning_rate = 0.0` and `learning_rate = 0.2`.

Interpretation:

- the engine currently supports the *prediction task*;
- the engine does **not yet support the proposed Phase 9 objective as written**
  if that objective ranks pairs by `activity`.

Reason:

- `activity` is driven by relationship touches and decay;
- Hebbian plasticity updates `weight`;
- therefore an objective based on `activity` has no learning-rate signal.

That makes the current Phase 9 draft structurally disconnected from the
actual knob it wants to tune.

## Efficiency results

`cargo bench -p graph-engine --bench phase9_plasticity`

### Stream-only cost

- `medium`: `46.10 ms .. 57.96 ms`
- `school_scale`: `102.14 ms .. 114.32 ms`
- `xlarge`: `179.36 ms .. 189.67 ms`

Observed throughput stayed in the same rough band:

- `medium`: `27.9K .. 35.1K events/s`
- `school_scale`: `25.2K .. 28.2K events/s`
- `xlarge`: `27.0K .. 28.5K events/s`

### End-to-end prediction pipeline

- `medium/activity`: `128.46 ms .. 135.43 ms`
- `medium/strength`: `124.73 ms .. 129.94 ms`
- `school_scale/activity`: `256.37 ms .. 273.07 ms`
- `school_scale/strength`: `254.79 ms .. 270.29 ms`
- `xlarge/activity`: `439.74 ms .. 499.13 ms`
- `xlarge/strength`: `421.30 ms .. 429.77 ms`

Interpretation:

- end-to-end evaluation cost scales reasonably with profile size;
- the cost increase is closer to linear than explosive on these profiles;
- switching from `activity` ranking to `strength` ranking does not materially
  change runtime, so fixing the Phase 9 signal mismatch should be possible
  without a major efficiency penalty.

## Conclusion

The engine is efficient enough to support larger-profile pair-prediction
benchmarks, and the pair-prediction task itself is viable.

But the current Phase 9 proposal should not tune `learning_rate` from an
`activity`-ranked objective. If Phase 9 is revived, the objective needs to be
redefined around a signal that plasticity actually influences, such as
`weight` or `strength`.
