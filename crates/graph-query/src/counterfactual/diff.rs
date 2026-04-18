use graph_core::{BatchId, ChangeId, RelationshipId};
use rustc_hash::FxHashSet;

/// The structural impact of removing a set of changes from the world.
///
/// Returned by [`super::counterfactual_replay`]. Describes what the world would
/// look like structurally if the specified changes and all their causal
/// descendants had never been committed.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CounterfactualDiff {
    pub removed_roots: Vec<ChangeId>,
    pub suppressed_changes: FxHashSet<ChangeId>,
    pub absent_relationships: Vec<RelationshipId>,
    pub divergence_batch: Option<BatchId>,
}

impl CounterfactualDiff {
    pub fn is_empty(&self) -> bool {
        self.suppressed_changes.is_empty()
    }
}
