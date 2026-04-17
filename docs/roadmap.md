# Roadmap

This document tracks forward-looking work for the engine. It supersedes the
"Open" subsection of `identity.md §8` — that section previously claimed "No
open roadmap items"; this file is now the authoritative forward plan.

Last updated: 2026-04-17. Phase 2 complete; Phase 3 E3/E2 done; E4 in progress.

## 1. Scope

**Active tracks:**

| Track | Name | Items |
|---|---|---|
| A | Control plane | A1, A2, A3 |
| B | Observation (partial) | B3, B4 |
| D | Causality | D1, D2, D3, D4 |
| E | Performance & scale | E1, E2, E3, E4 |

**Deferred until engine matures:**

| Track | Rationale |
|---|---|
| C — LLM integration | Reactivate once substrate APIs stabilize; LLM-inferred configs need a stable target to regression-test against. |
| F — Domain cookbook / external API | Reactivate once API surface is locked; cookbook and HTTP/gRPC/CLI layers should not chase a moving target. |

## 2. Phases

### Phase 0 — Landing

Promote experimental files currently untracked in the working tree to
committed state with tests and documentation. Establishes the baseline
for the rest of the roadmap.

| File | Role | Action |
|---|---|---|
| `crates/graph-engine/src/controller.rs` | Track A foundation | Keep; extend tests |
| `crates/graph-engine/src/handle.rs` | Track A foundation | Keep; extend tests |
| `crates/graph-query/src/causal_strength.rs` | D1 foundation | Keep; add unit tests |
| `crates/graph-llm/src/configure.rs` | Track C (deferred) | Keep with regression smoke test only; no further work |

### Phase 1 — Measurement (E1)

Profile representative workloads to rank hotspots. Phase 3 priorities
are reshuffled based on findings — no optimization work starts before
measurement.

**Hypotheses (written before profiling, compared against findings):**

| # | Hypothesis | Potentially affects |
|---|---|---|
| H1 | `EmergencePerspective::recognize` (BFS over relationships) dominates on dense graphs | E4 |
| H2 | `ChangeLog::push` + predecessor tracking dominates on deep DAGs | E2 |
| H3 | Hebbian/STDP weight update iterates all relationships each batch (O(R)) | E3 |
| H4 | `PropertyStore` lookup via `LocusContext::properties` is on hot path | micro-opt |
| H5 | `Storage::commit_batch` fsync dominates when storage feature is on | storage tier |
| H6 | `StateVector` (Vec&lt;f32&gt;) allocation/clone in hot path | SmallVec transition |

**Tooling:**
- `samply` — primary profiler (cross-platform, no root)
- `cargo flamegraph` — Linux/perf fallback
- `dhat-rs` — heap allocation profiling
- `criterion` — microbenchmarks and regression baselines

**Workloads (scaling curves required — single-N measurements hide algorithmic complexity):**

| Workload | Type | Sizes |
|---|---|---|
| `ring_dynamics` | synthetic, topology-controlled | N ∈ {16, 64, 256, 1024} |
| `neural_population` | dense, interaction-heavy | N ∈ {100, 500, 2000} |
| `celegans` | biological reference | fixed (~300 neurons) |
| `knowledge_graph` | heterogeneous reference | fixed |
| `stress_emergence` *(new)* | entity split/merge heavy | N ∈ {100, 1000, 10000} |

`stress_emergence` is new because existing workloads barely exercise the
Entity layer — without it, emergence-related scaling is invisible.

**Metrics:**
- Wall-clock: per-tick ms (p50/p99), normalized by changes per batch
- CPU: flamegraph self-time per function
- Heap: peak RSS, allocation count and size per tick
- Query cost: B4 benches captured as the E1 baseline

**Deliverables:**
1. `docs/perf/phase1-report.md` — hypotheses vs findings, top-10 ranked hotspots, Phase 3 reprioritization
2. `scripts/profile.sh` — reproducible workload × size profiling matrix
3. `benches/baselines/` — committed criterion baselines for regression guard
4. Updated Phase 3 ordering in this document

**Estimated duration:** ~3 days (tooling/workload setup 1d, runs 1d, analysis + report 1d).

### Phase 2 — Parallel tracks

Track A, Track D, and B4 are independent and may proceed in parallel.
B3 is a dependent sub-phase, gated on A3 and a design sign-off.

#### Track A — Control plane

- **A1.** Keep `tokio` behind an optional `async` feature (not in `default-features`). Add a both-features CI matrix. Extend `ChangeDriven` / `ClockDriven` integration tests beyond current smoke-level coverage.
- **A2.** `LocalHandle::subscribe_world_events() -> Stream<WorldEvent>` backed by the `SubscriptionStore` audit log. Tokio channel underneath. Cleanly disposes on handle drop.
- **A3.** Bounded `pending_stimuli` queue with `BackpressurePolicy::{Block, DropOldest, DropNewest, Reject}`. Default `Reject` (preserves invariants over liveness when the queue saturates).

#### Track D — Causality

- **D1.** Promote `causal_strength::{causal_direction, dominant_causes, dominant_effects, causal_in_strength, causal_out_strength, feedback_pairs}` to `graph-query::api::Query` variants. Wire into the planner's `explain()` with a cost class.
- **D2.** Granger-style score: lagged mutual information over the `ChangeLog`. For each (locus A, locus B), count A's changes followed within N batches by a B change. Compare empirically against D1's STDP-weight-derived score; document when each is appropriate.
- **D3.** Structural counterfactual replay — **no LLM** (Track C is deferred). API shape: `counterfactual_replay(world, remove: Vec<ChangeId>) -> WorldDiff`. Deep-copy world, drop the specified changes and their causal descendants, re-simulate from the divergence point, diff against the original.
- **D4.** Entity-level causality: trace `BecameDormant` / `Merged` / `Split` layer events through `causal_ancestors` to identify which upstream entity transitions caused a given downstream one.

#### Track B (partial)

- **B4.** Criterion benches for `graph-query`: `path_between`, `reachable_from_active`, `causal_ancestors`, `counterfactual`. Commit baselines under `benches/baselines/query/`. Small work; can interleave with any other Phase 2 item.
- **B3.** Time-travel queries — **approach (b): WorldDiff reverse replay**.
  - Chosen over approach (a) (WAL + snapshot replay-from-nearest) by explicit user decision.
  - **Flagged concern (recorded, not overriding):** reverse-applying a diff is expensive per batch step and scales with relationship/change count in the diff window. Viability depends on the expected query pattern.
  - **Gated on:**
    1. A3 complete (both touch `Storage` semantics and should not race).
    2. A design document `docs/b3-time-travel.md` signed off, covering:
       - Diff inversion semantics for each `WorldDiff` field
       - Invertibility of structural mutations (`CreateRelationship` / `DeleteRelationship`)
       - Behavior when the requested time range crosses a trimmed `ChangeLog` window
       - Query API surface (how callers request a prior-batch view)
  - Implementation follows design sign-off.

### Phase 3 — Performance & scale

Ordering updated from E1 findings (see `docs/perf/phase1-report.md`).
H3 (O(R) Hebbian sweep) confirmed as top bottleneck on dense graphs;
H1 (EmergencePerspective BFS) likely second. Original E2→E3→E4 revised to
**E3→E2→E4**.

- **E3.** Automatic relationship demotion: `InfluenceKindConfig::demotion_policy: Option<DemotionPolicy>`, where `DemotionPolicy ∈ { ActivityFloor(f32), IdleBatches(u64), LruCapacity(usize) }`. Runs in the engine tick after decay. Manual `Simulation::promote_relationship*` remains as the escape hatch.
  - **Rationale (E1):** O(R) Hebbian + decay sweep is the dominant tick cost on dense graphs. Reducing R via demotion is the highest-leverage optimization.
- **E2.** `ChangeLog::trim_before_batch` emits summary entries when discarding a batch range: `{locus, batch_range, change_count, net_Δstate, kinds_observed}`. Long-range causal queries return a "coarse trail" instead of "trail goes cold". Summary entries weather slower than raw changes (own policy).
  - **Rationale (E1):** stress_emergence at N=1000/10000 pending; entity churn data needed to confirm priority. Proceeds after E3.
- **E4.** Logical partitioning: `World::partition(fn(&Locus) -> PartitionId)` + per-partition batch loop. Inter-partition relationships cross an async boundary; partitions commit together in a single logical batch but process in parallel.
  - **Note:** this overrides the prior "rayon excluded" decision at a different granularity — partition-level parallelism, not within-batch rayon scatter. Distribution across machines remains out of scope.
  - **Rationale (E1):** Only worthwhile after E3 shrinks per-partition R. Measure after E3 lands.

## 3. Dependency graph

```
Phase 0
   │
   ▼
Phase 1 (E1)
   │
   ▼
Phase 2 ─┬─ Track A   (A1 → A2 → A3)
         ├─ Track D   (D1 → D2 → D4 → D3)
         ├─ B4        (parallel, small)
         └─ B3        (gated on A3 + design doc)
   │
   ▼
Phase 3 (E2 / E3 / E4, ordered by E1 findings)
```

- Track A and Track D are fully independent.
- B4 is small and fits alongside either track.
- B3 starts only after A3 lands and the design doc is signed off.
- Phase 3 starts only after Phase 2 completes enough of the query/engine surface
  to measure optimization impact credibly.

## 4. Deferred-track reactivation criteria

### Track C — LLM integration
Reactivate when: substrate APIs have stabilized through a release cycle
and LLM-inferred configs have a stable schema to regression-test against.
Candidate items on reactivation: `configure_*` evaluation harness, STDP-grounded
counterfactual narration (combines D3 with Track C), multi-perspective Cohere
selection, Anthropic prompt caching for repeated system prompts.

### Track F — Domain cookbook / external API
Reactivate when: public API surface is locked enough that external consumers
and cookbook examples will not churn. Candidate items: domain pattern library
(neural / social / supply-chain templates), HTTP/gRPC boundary layer,
`graph-cli` tool, documentation of `graph-boundary` and `graph-schema` roles.

## 5. Scope adjustments log

Decisions that changed item scope after the initial roadmap discussion:

- **D3** narrowed to structural counterfactual replay (no LLM narration) because Track C is deferred.
- **B3** uses approach (b) (WorldDiff reverse replay), not approach (a) (WAL + snapshot replay). Cost concern was flagged and documented; design doc required before implementation.
- **E1** approved as specified in §Phase 1 above; runs before E2/E3/E4 to convert hypotheses into measurements.
- **Phase 2 structural fix (2026-04-17)**: Instrumentation revealed `DefaultCoherePerspective::cluster`
  was O(E² × R) — 65M operations at N=1000. This was the dominant throughput bottleneck, not H3
  (Hebbian sweep). Fixed to O(E×M + R) via a `locus→entity` index + single relationship scan.
  Result: 17.8× speedup at N=1000 (517ms → 29ms/batch); N=10000 now practical (~300ms/batch).
  Concurrent finding: `find_communities` label propagation HashMap → Vec gave a secondary ~43%
  reduction in recognize_entities. Updated Phase 3 ordering: the cohere fix supersedes E3 as the
  immediate throughput win; E3 (reduce R) remains relevant for `sim.step()` cascade cost at scale.
- **E4 scope (2026-04-17)**: Partition locality measured across three workloads.
  stress_emergence N=10000 P=10: 99% within; neural_population N=1000 P=4: 100% within;
  celegans N=299 P=4: 32% within (24% touch-weighted). Decision: implement as **opt-in**
  user-supplied partition fn — callers without a meaningful partition fn see no benefit
  and no overhead. apply_emergence Update path optimised before E4 to clean the baseline.
  Implementation pending; see `docs/e4-design.md`.
- **BCM plasticity (2026-04-17)**: `PlasticityConfig::bcm_tau` adds the Bienenstock-Cooper-Munro
  sliding-threshold rule alongside plain Hebbian and STDP. θ_M persisted in World/Storage/Snapshot.
