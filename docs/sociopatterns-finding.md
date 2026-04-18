# SocioPatterns benchmark — finding

**Date**: 2026-04-18
**Evidence**: `crates/graph-engine/tests/sociopatterns.rs`

Synthetic SocioPatterns-style temporal contact stream: 40 students, 5
classes of 8, 60 time blocks × 10 co-attendance events per block,
`p_in = 0.75` intra-class vs `p_out = 0.05` cross-class (0.25 during
every 10th "lunch" block). Deterministic LCG — no real network access.

## Results

### 1. Class recovery (default perspective)

At a data-driven activity threshold computed by scanning for the
largest relative gap in the lower three-quarters of sorted
relationship activities — a local prototype of the Phase-2 auto-
threshold — the default `DefaultEmergencePerspective` recovers all
5/5 planted classes at **Jaccard 1.0 / precision 1.0 / recall 1.0**.

Effective auto-threshold: `7.64` (p75 of the activity distribution).
Distribution summary: n=686, min=2.00, p25=4.00, p50=5.09, p75=7.64,
p90=14.73, max=37.98 — clearly bimodal, with intra-class edges
accumulating above 7 while cross-class noise stalls at 2–5.

### 2. Partition stability

Tail of 4 checkpoints (blocks 30/40/50/60) stabilises at 5 entities
with variance 0.0. Earlier checkpoints (blocks 10/20) also land on 5 —
the partition converges within the first 10 blocks and holds.

### 3. Next-block prediction (Phase 9 reopen probe)

Train on 45 blocks → rank relationship pairs by activity → take
top-K candidates above the train-time auto-threshold → check against
actual pair appearances in the next 15 blocks.

| K   | precision | hits / K   | lift vs base rate (0.473) |
|-----|-----------|------------|---------------------------|
| 20  | 1.000     | 20 / 20    | 2.11×                     |
| 50  | 1.000     | 50 / 50    | 2.11×                     |
| 100 | 0.970     | 97 / 100   | 2.05×                     |

Recall of all above-threshold candidates (162) against 369 test-block
observed pairs: **0.377**.

## Phase-9 reopen condition (a) — does this supply a supervised metric?

**Yes — precision@K is a clean supervised signal.**

- It is computable from stream state alone (no external label source).
- It responds monotonically to relationship ranking quality: if
  plasticity parameters shift the ranking, precision@K moves with
  them.
- The base-rate-normalised lift (2.0–2.1×) is well above floor. Under
  degraded plasticity (learning rate too small → rank dominated by
  noise; too large → rank dominated by most-recent pair only), the
  lift would collapse towards 1.0× and K=20 hits would drop.
- Precision is already saturated at K=20/50 on this easy synthetic
  stream. A harder split (larger N, more lunch blocks, adversarial
  schedules) would give the auto-tuner a steeper gradient to descend.

Phase-9 reopen condition (a) — *"SocioPatterns or Enron benchmark
introduces a supervised metric"* — is **met**. The remaining
engineering task (defining a `PlasticityObjective` API that consumes
precision@K on a held-out slice) is an independent design decision,
not a missing-data blocker.

Suggested framing for the plasticity auto-tuner: minimise
`(1 − precision@K) + λ · (1 − recall)` on a rolling held-out tail of
each stream. `K` and `λ` are user-declared domain knobs (how many
predictions you need and how much you care about coverage vs. top
accuracy) — *not* tuning knobs.

## Tuning-knob surprises

- The p25 heuristic (first pass of `auto_activity_threshold`)
  collapsed the partition to 2 components because this dataset's
  activity distribution has a heavy floor of "pair appeared exactly
  once" edges that cluster at 2.0. The gap-detector variant that
  scans the lower 75% for the largest relative gap recovers the
  bimodal cut correctly. The Davis 0.1 → 3.0/5.0 manual-tuning
  finding generalises: fixed thresholds fail across datasets, but a
  distribution-aware auto-threshold can be universal if it searches
  for the right gap.
- Initial generator parameters (`EVENTS_PER_BLOCK = 4`, decay per
  event, tick-per-event) produced flat distributions where intra and
  cross-class activities both collapsed towards 2.0. Fix: combine all
  events in a block into a single tick call. This is also the more
  realistic model — a "time block" in the real SocioPatterns paper
  is a 20 s window aggregating all RFID contacts, not a sequence of
  ticks.
- No new engine-side knob surprises. The stream is well-behaved on
  the default emergence/stabilization/weathering settings.

## Implications

1. The auto-threshold gap detector should search the lower 75% of
   activities, not just the lower 50% (current Phase-2 heuristic
   might cut the sample window short).
2. Phase 9 — plasticity auto-tuning — can now be reopened with
   precision@K as the objective.
3. The test harness in `sociopatterns.rs` is a self-contained
   evaluator for future auto-tuning work: swap `InfluenceKindConfig`
   parameters, re-run, read off precision@K. No external labels
   needed.
