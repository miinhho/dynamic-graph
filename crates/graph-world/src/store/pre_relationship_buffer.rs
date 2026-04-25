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
