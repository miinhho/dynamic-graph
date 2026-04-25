//! Pending-evidence buffer for graded relationship emergence
//! (Phase 2 of the trigger-axis roadmap).
//!
//! Stores accumulated cross-locus evidence that has not yet crossed the
//! per-kind `EmergenceThreshold`. Once enough evidence accumulates within
//! the configured window, the buffer entry is promoted by the engine into
//! a real `Relationship` in `RelationshipStore` and removed from here.
//!
//! ## Phase 2a.i status (2026-04-25)
//!
//! The buffer type and its World integration are in place; the
//! engine's threshold-active write path (insert / accumulate / promote /
//! evict-on-window-expiry) lands in Phase 2b. With every registered
//! influence kind currently using `EmergenceThreshold::bypass()`, the
//! engine's `interpret_evidence` step never reaches the buffer and this
//! store remains empty across all ticks. The bit-equivalence canary
//! (`partition_determinism::ring_p4`) verifies that property in CI.
//!
//! ## Why graph-world (not graph-engine)
//!
//! Pending evidence is itself a structural signal — the user can ask
//! "what is currently being observed but has not yet crystallised?" —
//! so it lives next to the other layer stores rather than buried in
//! engine-private state. Persistence (Phase 2a.ii: storage v2 → v3
//! migration, snapshot round-trip) builds on top of this placement.

use graph_core::{BatchId, ChangeId, EndpointKey, InfluenceKindId};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// One pending-evidence record, keyed by `(EndpointKey, InfluenceKindId)`
/// in the buffer. The fields anticipate Phase 2b's promotion logic:
/// `accumulated` is the running signed sum, `first_seen_batch` /
/// `last_touched_batch` bound the window, and `contributing_changes`
/// becomes the `predecessors` vector of the eventual `RelationshipEmerged`
/// change record.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingEvidence {
    /// Running signed sum of `signed_activity_contribution(...)` values
    /// across all observations in the current window.
    pub accumulated: f32,
    /// Batch in which the first contribution was recorded. Used together
    /// with `last_touched_batch` and the kind's `window_batches` to
    /// detect window expiry.
    pub first_seen_batch: BatchId,
    /// Batch of the most recent contribution. The expiry rule is
    /// `current_batch − last_touched_batch > window_batches`.
    pub last_touched_batch: BatchId,
    /// `ChangeId`s of every cross-locus predecessor that has fed evidence
    /// into this pending entry. On promotion these become the
    /// `predecessors` list of the `RelationshipEmerged` change record,
    /// preserving the full causal lineage of the eventual relationship.
    pub contributing_changes: SmallVec<[ChangeId; 4]>,
}

/// Append-only-from-the-API buffer of pending cross-locus evidence.
///
/// Mutation is restricted to the engine via `pub(crate)` accessors on
/// `World`; user-facing query methods are read-only so callers can
/// inspect what is brewing without disturbing it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PreRelationshipBuffer {
    pending: FxHashMap<(EndpointKey, InfluenceKindId), PendingEvidence>,
}

impl PreRelationshipBuffer {
    /// Number of pending entries currently held. Cheap (`HashMap::len`).
    #[inline]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// `true` iff no entries are pending. With every registered kind
    /// using `EmergenceThreshold::bypass()` (Phase 2a.i), this returns
    /// `true` after every tick.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Read-only lookup for inspection / boundary tooling. Returns
    /// `None` when no evidence is pending for the given endpoint+kind.
    pub fn get(&self, key: &EndpointKey, kind: InfluenceKindId) -> Option<&PendingEvidence> {
        self.pending.get(&(key.clone(), kind))
    }

    /// Iterate every pending entry. Order is unspecified (HashMap).
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&(EndpointKey, InfluenceKindId), &PendingEvidence)> + '_ {
        self.pending.iter()
    }

    /// Engine-only by convention (the buffer's invariants — single
    /// transition point, lookup-order discipline — assume this is the
    /// only mutating entry point used by the apply pipeline). Returns the
    /// resulting outcome — either the entry stayed pending (no
    /// relationship action), or it crossed `min_evidence` within the
    /// `window_batches` window and the caller should now mint a
    /// `Relationship`.
    ///
    /// **Reset-on-fresh-contribution semantics**: this is the *attempt*
    /// window. When `current_batch − first_seen_batch > window_batches`
    /// and a fresh contribution arrives, the existing accumulation is
    /// reset and the contribution seeds a *new* attempt starting at
    /// `current_batch`. Without reset we would lose the evidence
    /// entirely; the call would still arrive but the entry would behave
    /// like a stale ghost.
    ///
    /// This is **not** the same as `evict_expired`'s sweep — see that
    /// method's doc for the housekeeping path that drops idle entries
    /// without ever seeing a fresh contribution.
    ///
    /// Public so `graph-engine::emergence_apply` can call it across the
    /// crate boundary; the discipline is enforced by `pending` being a
    /// private field (no `insert`/`remove` reachable from user code).
    pub fn record_evidence(
        &mut self,
        key: EndpointKey,
        kind: InfluenceKindId,
        contribution: f32,
        change_id: ChangeId,
        current_batch: BatchId,
        window_batches: u64,
        min_evidence: f32,
    ) -> RecordOutcome {
        let entry_key = (key, kind);
        let entry = self.pending.entry(entry_key.clone()).or_insert_with(|| PendingEvidence {
            accumulated: 0.0,
            first_seen_batch: current_batch,
            last_touched_batch: current_batch,
            contributing_changes: SmallVec::new(),
        });

        // Window expiry — the entry is older than its window allows.
        // Reset rather than drop: this contribution itself is fresh evidence
        // and seeds the next window. Without reset we'd lose the signal
        // entirely.
        let age = current_batch.0.saturating_sub(entry.first_seen_batch.0);
        if age > window_batches {
            entry.accumulated = 0.0;
            entry.first_seen_batch = current_batch;
            entry.contributing_changes.clear();
        }

        entry.accumulated += contribution;
        entry.last_touched_batch = current_batch;
        entry.contributing_changes.push(change_id);

        if entry.accumulated.abs() >= min_evidence {
            // Promotion: hand the accumulated state back to the caller and
            // remove the entry. Single transition point — once an entry
            // promotes, it lives in `RelationshipStore`, never both stores
            // simultaneously (advisor's #4 in the Phase 2 design review).
            let promoted = self.pending.remove(&entry_key).expect("entry was just inserted");
            RecordOutcome::Promoted {
                accumulated: promoted.accumulated,
                contributing_changes: promoted.contributing_changes,
            }
        } else {
            RecordOutcome::StillPending
        }
    }

    /// Housekeeping sweep: drop entries that have been idle (no fresh
    /// contribution) for longer than `default_window_batches`. **Idle
    /// expiry semantics** — measured against `last_touched_batch`, not
    /// `first_seen_batch`. This complements `record_evidence`'s
    /// reset-on-fresh-contribution semantics: `record_evidence` handles
    /// the case where new evidence arrives after a stale window
    /// (restart), while `evict_expired` handles the case where no
    /// further evidence ever arrives (garbage-collect).
    ///
    /// **Phase 2b status (2026-04-25)**: the engine does **not** call
    /// this from any per-tick housekeeping path yet. Idle entries
    /// therefore persist in the buffer until either a fresh contribution
    /// triggers `record_evidence`'s reset path or a caller invokes this
    /// method directly. Wiring an automatic sweep is deferred to a
    /// follow-up — the per-tick cost vs. memory-leak trade-off is one
    /// `EmergenceThreshold` configurations in real datasets need to
    /// inform.
    ///
    /// Returns the count of entries evicted, for telemetry.
    pub fn evict_expired(
        &mut self,
        current_batch: BatchId,
        default_window_batches: u64,
    ) -> usize {
        let before = self.pending.len();
        self.pending.retain(|_, entry| {
            let age = current_batch.0.saturating_sub(entry.last_touched_batch.0);
            age <= default_window_batches
        });
        before - self.pending.len()
    }
}

/// Outcome of `PreRelationshipBuffer::record_evidence`.
#[derive(Debug, Clone)]
pub enum RecordOutcome {
    /// Evidence was added but the running magnitude is still below
    /// `min_evidence`. No relationship action; the entry remains in the
    /// buffer for future contributions or window expiry.
    StillPending,
    /// Accumulated evidence crossed `min_evidence` on this contribution.
    /// The entry has been removed from the buffer; the engine must now
    /// mint a `Relationship` whose initial activity is `accumulated` and
    /// whose `Change.predecessors` is `contributing_changes`.
    Promoted {
        accumulated: f32,
        contributing_changes: SmallVec<[ChangeId; 4]>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::LocusId;

    #[test]
    fn default_buffer_is_empty() {
        let buf = PreRelationshipBuffer::default();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn get_on_empty_returns_none() {
        let buf = PreRelationshipBuffer::default();
        let key = EndpointKey::Symmetric(LocusId(1), LocusId(2));
        assert!(buf.get(&key, InfluenceKindId(0)).is_none());
    }
}
