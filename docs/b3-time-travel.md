# B3 — Time-Travel Queries Design Document

**Status**: Design sign-off required before implementation.  
**Gate**: A3 complete (✓ as of 2026-04-17).  
**Approach**: (b) WorldDiff reverse replay (chosen over WAL + snapshot replay).

---

## 1. Problem Statement

Callers need to answer "what did the world look like at batch N?" without storing
per-batch snapshots. The `ChangeLog` records every committed change forward in time;
the question is whether we can reconstruct prior state by **inverting** a `WorldDiff`.

---

## 2. Diff Inversion Semantics

Each field of `WorldDiff` must be invertible for reverse-replay to work.

### 2.1 Change Log (`change_ids`)

Changes are append-only — they cannot be un-committed. Inversion here means
**ignoring** the `change_ids` of the range when reconstructing prior locus/relationship
state. We do not actually remove changes from the log; we compute what the state would
have been before the range by walking the `before` fields of each relevant change.

**Invertible**: Yes — walk changes in `[from, to)` backward, apply `before` states.

### 2.2 Locus State

For each `ChangeSubject::Locus` change in the range, the `before` field holds the
pre-change state. Reversing means setting each affected locus back to its `before`
value from the earliest change in the range.

**Invertible**: Yes — use earliest `before` field for each locus in range.

### 2.3 Relationship State (`relationships_created`, `relationships_updated`)

- **Created** (`lineage.created_by` in range): the relationship did not exist before
  the range. Reversal requires removing it from the world view.
- **Updated** (`lineage.last_touched_by` in range, but not created): existed before,
  state changed. Reversal means restoring the `before` activity from the earliest
  relationship change in the range.

**Invertible**: Yes for updates. Created relationships require removal — the "prior
world" does not contain them. Store `relationships_created` IDs in the inverted diff
as "to remove".

### 2.4 Entity Layer Inversion (`entities_changed`)

Entities are sedimentary — they never lose layers, only gain them. To reconstruct the
prior entity view, ignore all layers deposited in `[from, to)`.

**Invertible**: Yes — filter layers by `layer.batch < from`.

For compressed/skeleton layers where `snapshot` has been dropped, the entity's
`current` field (which reflects the newest layer) cannot be restored exactly.
This is a known limitation: entity state can only be time-traveled to the resolution
of the surviving snapshot data (i.e., `CompressionLevel::Full` layers).

**Limitation**: Entity current state is approximate when the target batch falls in
a compressed or skeleton range.

### 2.5 Subscription Events (`subscriptions_added`, `subscriptions_removed`)

- `subscriptions_added` → reverse means unsubscribing those `(subscriber, rel_id)` pairs.
- `subscriptions_removed` → reverse means re-subscribing.

**Invertible**: Yes — the audit log (`SubscriptionStore::events_in_range`) records both
directions with batch tags. Inversion is straightforward.

**Caveat**: If the subscription audit log has been trimmed (via
`trim_audit_before`), events before the trim point are gone. Time-travel into
a trimmed range returns a partial subscription view (subscriptions that existed
at trim time are assumed to be unchanged before that point).

### 2.6 Relationship Trajectory (`relationships_strengthening`, `relationships_weakening`)

These are derived from relationship changes (already covered in §2.3). No separate
inversion needed — the trajectory diff for the reversed range is simply the negation
of the original (strengthening becomes weakening, and vice versa).

### 2.7 Pruned Relationships (`relationships_pruned`)

Pruned relationships no longer exist in the live world. Time-traveling *before* the
prune requires **re-inserting** them. However, the pruned log only records the
`RelationshipId` and batch — not the full relationship state at the time of pruning.

**Limitation**: Pruned relationships cannot be fully restored. The time-travel API
will report them as "pruned-not-restorable" in the result rather than silently
omitting them.

---

## 3. Behavior When Range Crosses a Trimmed ChangeLog Window

`ChangeLog::trim_before_batch(B)` removes all changes with `batch < B`. A time-travel
query to batch `T < B` hits the trimmed boundary.

**Behavior**: Return the earliest available world state (at batch `B`) with a
`TrimmedAt(BatchId)` annotation in the result. Callers can detect that the target
batch was not fully reachable.

```rust
pub struct TimeTravel {
    pub world_at: /* reconstructed world or diff */,
    /// Some(batch) when the requested target_batch was earlier than the trim boundary.
    pub trimmed_at: Option<BatchId>,
}
```

---

## 4. Query API Surface

### 4.1 Core function

```rust
/// Reconstruct the world diff needed to go from `current_batch` back to `target_batch`.
/// 
/// Returns a `TimeTravelResult` containing:
/// - The `WorldDiff` covering `[target_batch, current_batch)` (for context)
/// - Per-field inverse instructions  
/// - `trimmed_at` if the target is older than the trim boundary
pub fn time_travel(world: &World, target_batch: BatchId) -> TimeTravelResult;
```

### 4.2 Result type

```rust
pub struct TimeTravelResult {
    pub target_batch: BatchId,
    /// The inverse diff — describes what to undo to reach `target_batch`.
    pub inverse: WorldDiff,
    /// Relationships that were created in (target, current) and must be removed
    /// to reconstruct the prior view.
    pub relationships_to_remove: Vec<RelationshipId>,
    /// Relationships that were pruned in (target, current) that cannot be restored.
    pub relationships_irrecoverable: Vec<RelationshipId>,
    /// Entity ids where the prior state is approximate (snapshots compressed/skeletonized).
    pub entities_approximate: Vec<EntityId>,
    /// Non-None if target_batch is older than the ChangeLog trim boundary.
    pub trimmed_at: Option<BatchId>,
}
```

### 4.3 Query API variant

```rust
// In Query enum:
TimeTravel { target_batch: BatchId },
// Result: QueryResult::TimeTravelResult(TimeTravelResult)
```

---

## 5. Complexity

| Operation | Cost | Notes |
|-----------|------|-------|
| Compute inverse diff | O(k + R + E·L_avg) | k=changes in range, same as `diff_between` |
| Identify pruned-irrecoverable | O(pruned_log length) | bounded by `trim_pruned_log_before` calls |
| Entity approximation check | O(E · L_avg) | scan layers, check CompressionLevel |

The dominant cost is the same as `WorldDiff::compute`. Time-travel is **not**
incrementally cheaper than recomputing the diff — it scales with the range width.

For large ranges (many batches), callers should prefer narrowing to the smallest
window that answers their question. The API does not cache intermediate diffs.

---

## 6. Out of Scope

- Full world clone (apply all inverse operations to produce a live `World` copy):
  expensive and complex; deferred to a later phase if needed.
- Relationship state restoration for pruned relationships: pruned state is not
  stored anywhere — not recoverable without a snapshot store.
- Sub-batch resolution: time travel operates at batch granularity, not change granularity.

---

## 7. Implementation Plan (post sign-off)

1. Add `time_travel.rs` to `graph-query/src/`.
2. Implement `TimeTravelResult` and `time_travel(world, target_batch)`.
3. Wire into `Query::TimeTravel` and `QueryResult::TimeTravelResult`.
4. Add planner `explain()` arm.
5. Unit tests: basic inversion, trimmed boundary, pruned relationships.
6. Update roadmap log.
