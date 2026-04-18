use graph_core::{BatchId, EntityId, RelationshipId};
use graph_world::WorldDiff;

/// The structural inverse of applying a batch range — describes what must be
/// "undone" to reconstruct the world as it was at `target_batch`.
///
/// Returned by [`crate::time_travel`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeTravelResult {
    /// The batch the caller requested to travel back to.
    pub target_batch: BatchId,
    /// The forward `WorldDiff` for `[target_batch, current_batch)`.
    /// Callers can use this to understand what *happened* in the range being reversed.
    pub forward_diff: WorldDiff,
    /// Relationships created in `(target_batch, current_batch]` — these would
    /// not exist at `target_batch` and should be excluded from the prior view.
    pub relationships_to_remove: Vec<RelationshipId>,
    /// Relationships that were pruned in the range and cannot be fully restored.
    /// The prior-batch view reports these as "irrecoverable" — their state at
    /// `target_batch` is unknown because the pruned-log only records the ID.
    pub relationships_irrecoverable: Vec<RelationshipId>,
    /// Entity IDs whose prior state is approximate because the layers in the
    /// target range have been compressed or skeletonised (snapshot dropped by
    /// weathering).
    pub entities_approximate: Vec<EntityId>,
    /// `Some(batch)` when the requested `target_batch` is older than the
    /// ChangeLog's trim boundary — the result reflects the earliest available
    /// state, not the exact requested one.
    pub trimmed_at: Option<BatchId>,
}

impl TimeTravelResult {
    pub fn is_exact(&self) -> bool {
        self.trimmed_at.is_none() && self.entities_approximate.is_empty()
    }
}
