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

**Option A — Owned partition slices (preferred)**

Before the parallel Apply phase, split `RelationshipStore` into per-partition
shards. Each partition owns its shard. Cross-partition relationships land in the
partition of their **source locus** (from-endpoint). After Apply, shards are
reassembled. No locking; ownership is transferred.

Downside: shard split/reassemble is O(R) per batch, adding overhead for
callers with small P or low locality.

**Option B — Mutex per relationship**

Wrap each `Relationship` in an `RwLock<Relationship>`. Cross-partition writes
contend on the lock. Straightforward but changes the hot-path data layout.
Not preferred — adds lock overhead to the already-tight Update hot path.

**Option C — Cross-partition buffer (deferred)**

During Apply, cross-partition emergence ops are pushed to a per-source-partition
buffer. After the parallel phase, a sequential pass drains cross-partition
buffers. This is the cleanest separation but adds a second serial phase.

**Decision: start with Option A.** It preserves the existing single-`&mut` world
write model and requires no data structure changes to `Relationship`. If shard
overhead is measurable (expected only at P >> 10), revisit Option C.

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
2. **Shard split/reassemble** — `RelationshipStore::split_by_partition` +
   `reassemble`. Unit-test round-trip correctness.
3. **Parallel Apply** — replace sequential Apply loop with `rayon::scope` over
   partition shards. Each partition gets its own shard + its subset of
   `computed` changes.
4. **Parallel Dispatch** — same structure; merge `last_fired_partial` and
   `pending` after join.
5. **Integration test** — run `neural_population` N=1000 P=4 with and without
   partition fn; assert identical `World` state after 100 ticks (determinism
   check).
6. **Benchmark** — compare `ring_scaling` N=1024 P=4 vs single-partition as the
   E4 criterion group.

Rayon `scope` (not `spawn`) is appropriate here because all partition work
is bounded by the current batch — no unbounded futures.

---

## 11. Out of Scope

- Distribution across machines (explicitly excluded from E4).
- Dynamic repartitioning within a tick (deferred; requires quiescent point).
- Partition-aware `path_between` / BFS queries (graph-query is read-only; no change needed).
