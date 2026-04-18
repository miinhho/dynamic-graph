# Performance Priorities

This list reflects the current code shape after the core refactors. The main
goal is not "optimize everything", but to measure the places where the new
stage boundaries exposed likely cost centers.

## Priority 1: Engine batch hot path

Files:
- `crates/graph-engine/src/engine/batch/compute.rs`
- `crates/graph-engine/src/engine/pipeline.rs`
- `crates/graph-engine/src/engine/apply.rs`
- `crates/graph-engine/src/engine/emergence_apply.rs`

Why:
- This is the dominant runtime path for `Simulation::step()` and `Engine::tick()`.
- The refactor made `compute -> build -> apply -> settle` explicit, which makes
  repeated allocation and repeated world lookup more visible.

Likely optimization candidates:
- reduce intermediate `Vec` churn in `build_changes`
- cache repeated relationship/locus lookups inside batch compute
- reuse common per-batch values when recording cross-locus emergence
- reduce per-prediction record assembly overhead in warm-update paths

Benchmark points:
- `graph-engine/benches/engine.rs::bench_batch_hot_path`
- compare `cold_emerge` vs `warm_update`

Current measured baseline:
- `cold_emerge`: ~`3.03..3.94 ms`
- `warm_update`: ~`2.22..2.77 ms`

## Priority 2: Query filtered pipeline

Files:
- `crates/graph-query/src/query_api/filtered.rs`
- `crates/graph-query/src/query_api/filtered/candidates.rs`
- `crates/graph-query/src/query_api/filtered/predicates.rs`
- `crates/graph-query/src/query_api/filtered/sorting.rs`

Why:
- Query execution spends noticeable time in candidate collection, predicate
  chains, and summary sorting.
- The refactor exposed the exact split between seed selection and sorting, so
  the cost of full scan vs seeded scan is now easy to measure.

Likely optimization candidates:
- avoid over-collecting candidates before limit/sort decisions
- short-circuit predicate chains earlier
- delay summary projection until after filtering and truncation
- specialize full-scan vs seeded-scan paths more aggressively

Benchmark points:
- `graph-query/benches/graph_query.rs::bench_filtered_pipeline`
- compare `full_scan/sort` vs `touching_seed/sort` vs `touching_seed/no_sort`

Current measured baseline after first optimization:
- `full_scan/sort_strength/limit=100`: ~`3.25..3.49 ms`
- `touching_seed/sort_strength/limit=100`: ~`14.09..14.29 us`
- `touching_seed/no_sort/limit=100`: ~`4.19..4.26 us`

## Priority 3: Phase 9 objective pipeline

Files:
- `crates/graph-engine/src/plasticity/objective.rs`
- `crates/graph-engine/src/plasticity/objective/context.rs`
- `crates/graph-engine/src/plasticity/objective/ranking.rs`
- `crates/graph-engine/src/plasticity/objective/types/events.rs`
- `crates/graph-engine/src/plasticity/objective/types/metrics.rs`

Why:
- The measurement contract is now explicit, which makes ranking cost and
  score-window cost separable.
- This path is a likely research loop bottleneck if we iterate objective and
  learner tuning often.

Likely optimization candidates:
- reduce event-pair extraction allocations
- reuse ranking buffers across evaluations
- borrow more and clone less in window assembly
- precompute symmetric pair normalization where possible

Benchmark points:
- `graph-engine/benches/phase9_plasticity.rs::bench_phase9_objective_window`
- compare `rank` vs `score_window`

Current measured baseline after window-range optimization:
- `medium/rank`: ~`40.44..46.95 us`
- `medium/score_window`: ~`279.80..295.87 us`
- `school_scale/rank`: ~`118.48..129.93 us`
- `school_scale/score_window`: ~`1.20..1.69 ms`

## Priority 4: Structural relationship creation path

Files:
- `crates/graph-engine/src/engine/batch/structural.rs`

Why:
- It sits on the boundary between structural proposal resolution and world
  mutation.
- The refactor exposed a clean `resolve -> apply` split that can later be
  batched or parallelized.

Likely optimization candidates:
- batch create/update decisions before touching the store
- reuse initial state templates per kind
- fuse synthetic relationship creation with batched insert paths

Current coverage:
- indirectly stressed by `bench_batch_hot_path`

## Priority 5: Emergence/entity mutation lifecycle path

Files:
- `crates/graph-engine/src/engine/world_ops/entity_mutation.rs`
- `crates/graph-engine/src/emergence/default.rs`
- `crates/graph-engine/src/emergence/default/proposals.rs`

Why:
- This is more workload-sensitive than the batch path, but can dominate in
  entity-heavy worlds.
- The refactor exposed proposal-family boundaries, which should make future
  batching feasible.

Likely optimization candidates:
- pre-bucket proposal families to reduce match overhead
- fuse entity mutation writes for split/merge paths
- reuse temporary vectors during proposal assembly

Current coverage:
- indirectly stressed by existing `emergence` benches in `graph-engine/benches/engine.rs`
