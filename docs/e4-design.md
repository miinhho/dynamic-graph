# E4 Design — Logical Partition Parallelism

**Status:** Pending implementation. Locality measurement done 2026-04-17.

## 1. Motivation

`sim.step()` is single-threaded through the batch loop. Within a batch:

- The **compute phase** already runs via `rayon::par_iter` (one future per pending change).
- The **build phase** already runs via `rayon::par_iter` (one future per non-elided change).
- The **dispatch phase** (`process` + `structural_proposals`) runs via `rayon::par_iter`.
- **apply_emergence** (the heaviest per-change step in dense graphs) is sequential.

Partition-level parallelism targets a different granularity: run independent
per-partition dispatch + apply_emergence loops concurrently. Partitions share a
logical batch ID but process their loci independently, then merge at a sync point.

Locality data (2026-04-17):

| Workload | N | P | Within-partition % |
|---|---|---|---|
| stress_emergence | 10000 | 10 | 99% |
| neural_population | 1000 | 4 | 100% |
| celegans | 299 | 4 | 32% (24% touch-weighted) |

celegans is fully-connected and is a bad fit for partitioning. The API must be
**opt-in** so callers with poor locality see no overhead.

---

## 2. API Shape

```rust
/// Assigns each locus to a partition. Called once on World construction or
/// explicit repartition. Return value is arbitrary — the engine buckets by
/// equality, not range.
pub type PartitionFn = Arc<dyn Fn(&graph_core::Locus) -> u64 + Send + Sync>;

impl World {
    /// Attach a partition assignment. Loci added after this call are
    /// assigned on first tick. Pass `None` to revert to single-partition mode.
    pub fn set_partition_fn(&mut self, f: Option<PartitionFn>);

    /// Re-evaluate every locus through the current partition fn and rebuild
    /// the assignment index. O(L) where L = locus count.
    pub fn repartition(&mut self);
}
```

Callers that do not call `set_partition_fn` see **no change in behavior** —
the engine falls back to the existing single-partition loop.

Recommended caller pattern (range-aligned, respects community structure):

```rust
let n = world.loci().len() as u64;
let p = 10u64;
world.set_partition_fn(Some(Arc::new(move |locus| {
    locus.id().0 * p / n
})));
```

Hash-based (`id % P`) distributes uniformly but ignores community locality —
use range-based when locus IDs were allocated in community order.

---

## 3. Internal Data Model

`World` gains a partition index maintained alongside the locus store:

```rust
struct PartitionIndex {
    fn_: PartitionFn,
    /// locus_id → partition_id
    assignment: FxHashMap<LocusId, u64>,
    /// partition_id → Vec<LocusId> (for fast per-partition iteration)
    members: FxHashMap<u64, Vec<LocusId>>,
}
```

The index is rebuilt by `repartition()` and updated incrementally by
`create_locus` / `delete_locus` if a partition fn is active.

---

## 4. Sync Boundary

Partitions run **per-phase**, not per-batch. The phases within a batch are:

```
Compute   (already par, unchanged)
Build     (already par, unchanged)
Apply     ← NEW: split by partition, run partitions in parallel
  ├── apply locus state + changelog (per-locus, no cross-partition reads)
  ├── apply_emergence (relationship hot-loop, mostly within-partition)
  └── structural proposals (CreateRelationship/DeleteRelationship)
Dispatch  ← NEW: split by partition, run partitions in parallel
  └── LocusProgram::process + collect follow-up changes
Plasticity (Hebbian/STDP/BCM) — sequential after merge (see §6)
Decay      — sequential after merge (unchanged)
Advance batch ID
```

The merge point is after Dispatch. At merge:
- All per-partition `pending` change lists are concatenated into the shared queue.
- All per-partition `plasticity_obs` lists are concatenated for the Hebbian phase.
- All per-partition `structural_proposals` are concatenated and applied in batch.

The merge point is **cheap** — it is a Vec concatenation, no lock required,
because partitions write into independent Vecs during their phase.

---

## 5. Inter-Partition Relationships

A relationship between locus A (partition 0) and locus B (partition 1) is
**cross-partition**. apply_emergence always touches the relationship via
`world.relationships_mut().get_mut(rel_id)`.

Since Apply runs partitions in parallel, two partitions cannot hold `&mut World`
simultaneously. Options:

**Option A — Owned partition slices (race risk — NOT the default)**

Before the parallel Apply phase, split `RelationshipStore` into per-partition
shards by source locus. **Race condition**: if locus A (partition 0) and
locus B (partition 1) both emit changes in the same batch that touch the
shared relationship `rel(A,B)`, both partition Apply loops would write
`rel.state`, `rel.lineage.last_touched_by`, and `rel.lineage.change_count`
concurrently. `last_touched_by` becomes race-dependent; determinism is broken.

Option A is only safe if the cross-partition apply_emergence rule is
**source-only**: partition P only processes emergence for relationships
whose **from-endpoint** is in P. A change from locus B arriving at
`rel(A,B)` would be a no-op at the relationship level — only B's locus
state is updated. This requires the design to specify the asymmetry
explicitly and test that locus-state-only behaviour is correct.

**Option B — Mutex per relationship**

Wrap each `Relationship` in an `RwLock<Relationship>`. Cross-partition writes
contend on the lock. Straightforward but changes the hot-path data layout.
Not preferred — adds lock overhead to the already-tight Update hot path.

**Option C — Cross-partition buffer (preferred)**

During the parallel Apply phase, cross-partition emergence ops are pushed
to a per-source-partition buffer instead of immediately applied. After the
parallel phase, a sequential drain pass applies the cross-partition ops in
deterministic order. No races; locus state and relationship state separate
cleanly.

Cost: one extra serial pass over cross-partition ops only (not all of R).
At 99% locality (stress_emergence N=10000 P=10) this is ~1% of total ops.

**Decision: Option C.** The determinism guarantee is non-negotiable (the
integration test §10 step 5 asserts exact world state equality). Option A
is only safe with the source-only asymmetry rule, which changes semantics.
Option C has negligible cost at the observed locality levels.

### 5a. Cross-partition Drain Order (determinism rule)

The sequential drain pass must apply cross-partition ops in a **fixed, deterministic
order** — otherwise two runs with the same seed but different thread schedules
can produce different relationship states. Rule:

> Process partitions in **ascending bucket ID order**. Within a bucket, apply
> ops in the order they were buffered (source-locus iteration order, which is
> deterministic because `PartitionIndex::members_of` returns loci sorted by
> insertion order, not hash order — enforced in the drain loop by sorting the
> cross-partition buffer by `(bucket_id, locus_id)` before draining).

### 5b. `CreateRelationship` ID Minting at Partition Boundaries

`RelationshipStore::mint_id()` advances a shared monotone counter. During the
parallel Apply phase, two partition threads cannot both call it without a mutex.
**Resolution: defer `CreateRelationship` structural proposals to the sequential
post-phase.**

During parallel Apply, each partition accumulates `Vec<StructuralProposal>` for
cross-partition creates. After the parallel phase, the sequential drain loop:
1. Processes cross-partition emergence ops (see §5 / §5a).
2. Processes all buffered `CreateRelationship` proposals in sorted order (bucket
   ascending, then proposal index) — minting IDs sequentially.
3. Applies `DeleteRelationship` proposals similarly.

Within-partition creates whose both endpoints are local can mint IDs freely
during the parallel phase (each partition owns a pre-reserved ID range,
allocated before the parallel split via `RelationshipStore::reserve_id_range`).

---

## 6. BCM Thresholds and STDP at Partition Boundaries

### BCM

`world.bcm_thresholds: FxHashMap<LocusId, f32>` is updated in the Plasticity
phase which is **after the partition merge** — no partition-boundary concern.

### STDP `is_feedback_in_dag`

`is_feedback_in_dag` walks the predecessor DAG via `world.log()` reads. This is
read-only and can run in parallel without changes — `ChangeLog` is immutable
during Apply + Dispatch phases (no new commits happen until Advance batch ID).

### Refractory tracking (`last_fired`)

`last_fired: FxHashMap<LocusId, u64>` is per-tick, not per-partition. It must
remain shared. Since `last_fired` writes happen only in Dispatch and we run
partition Dispatch phases in parallel, this needs to be either:
- `Arc<RwLock<FxHashMap>>` shared across partition Dispatch threads, or
- Duplicated per-partition and merged (max-of-two) after Dispatch.

**Decision: merge after Dispatch.** Each partition accumulates its own
`last_fired_partial`; after the parallel Dispatch phase, reduce by
`max(existing, new)` across all partitions. Avoids any lock on the hot dispatch
path.

---

## 7. ChangeId Reservation

`world.reserve_change_ids(n)` advances a monotone counter. It is called in the
Build phase (already single-threaded pre-partition split) and must remain
sequential. Per-partition Dispatch accumulates `ProposedChange`s with no IDs
assigned; IDs are reserved in the next batch's Build phase. No change needed.

---

## 8. WorldEvent Aggregation

Each partition accumulates `Vec<WorldEvent>` during Apply. After the partition
merge, all partition event lists are concatenated into `TickResult::events`.
Order within a batch is arbitrary across partitions; within a partition,
existing ordering is preserved.

---

## 9. SubscriptionStore Notifications

Relationship change notifications (`pending_rel_notifications`) are collected
during Apply, resolved to subscriber loci in a sequential pass after Apply.
Since notification resolution is already a single scan over the subscriber map,
partition parallelism does not change this — each partition builds its own
`pending_rel_notifications` list, and the lists are concatenated before
resolution.

---

## 10. Implementation Plan

1. **Add `PartitionIndex` to `World`** — `set_partition_fn`, `repartition`,
   incremental update on create/delete locus. No engine changes yet.
2. **Shard split/reassemble** — `extract_by_source_bucket` + `reinsert_many`
   already implemented in `RelationshipStore`. Unit-test round-trip correctness
   (included in relationship_store tests). ✓ Done (657a91d)
3. **Parallel Apply** — replace sequential Apply loop with `rayon::scope` over
   partition shards. Each partition gets its own shard + its subset of
   `computed` changes. Cross-partition emerge ops buffered; drained sequentially
   in ascending `(bucket_id, locus_id)` order (§5a). `CreateRelationship`
   proposals deferred to sequential post-phase using pre-reserved ID ranges (§5b).
4. **Parallel Dispatch** — same structure; merge `last_fired_partial` and
   `pending` after join.
5. **Integration test (determinism harness)** — run `neural_population` N=1000
   P=4 with and without partition fn; assert identical `World` state after 100
   ticks. Write this *before* parallel Apply to establish the pass/fail oracle.
6. **Benchmark** — compare `ring_scaling` N=1024 P=4 vs single-partition as the
   E4 criterion group. Expected gain: ~17% on emerge-heavy workloads (emerge ≈5ms
   of ~24ms tick; dispatch is already `par_iter`).

Rayon `scope` (not `spawn`) is appropriate here because all partition work
is bounded by the current batch — no unbounded futures.

---

## 11. Out of Scope

- Distribution across machines (explicitly excluded from E4).
- Dynamic repartitioning within a tick (deferred; requires quiescent point).
- Partition-aware `path_between` / BFS queries (graph-query is read-only; no change needed).
