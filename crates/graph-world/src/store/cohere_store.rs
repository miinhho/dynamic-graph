//! Store for cohere clusters (Layer 4).
//!
//! Unlike the entity store, coheres are *ephemeral by design* — each
//! call to `extract_cohere` replaces the previous set for a given
//! perspective key. The store is indexed by a user-supplied perspective
//! name so multiple perspectives can coexist.
//!
//! Optionally, callers can configure `max_history` > 0 to retain a
//! rolling window of previous cohere snapshots per perspective,
//! enabling temporal analysis ("which entities were clustered together
//! over the last N recognition passes?").

mod history;

use graph_core::{BatchId, Cohere, CohereId};
use rustc_hash::FxHashMap;
use std::collections::VecDeque;

/// One historical cohere snapshot: the batch when it was captured
/// and the clusters that were active at that time.
#[derive(Debug, Clone)]
pub struct CohereSnapshot {
    pub batch: BatchId,
    pub coheres: Vec<Cohere>,
}

#[derive(Debug, Default, Clone)]
pub struct CohereStore {
    /// Perspective name → current set of coheres for that perspective.
    by_perspective: FxHashMap<String, Vec<Cohere>>,
    /// Perspective name → rolling history of past cohere snapshots.
    history: FxHashMap<String, VecDeque<CohereSnapshot>>,
    /// Maximum number of history snapshots to retain per perspective.
    /// `0` disables history (default).
    max_history: usize,
    next_id: u64,
}

impl CohereStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with history enabled.
    pub fn with_history(max_history: usize) -> Self {
        Self {
            max_history,
            ..Self::default()
        }
    }

    /// Set the maximum history window. Existing history entries beyond
    /// this limit are trimmed on the next `update`.
    pub fn set_max_history(&mut self, max_history: usize) {
        self.max_history = max_history;
    }

    pub fn mint_id(&mut self) -> CohereId {
        let id = CohereId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Replace the cohere set for `perspective`. If history is enabled,
    /// the *previous* set is pushed to the history ring buffer before
    /// being replaced.
    pub fn update(&mut self, perspective: impl Into<String>, coheres: Vec<Cohere>) {
        history::update(self, perspective, coheres);
    }

    /// Replace the cohere set and tag the outgoing snapshot with `batch`.
    pub fn update_at(
        &mut self,
        perspective: impl Into<String>,
        coheres: Vec<Cohere>,
        batch: BatchId,
    ) {
        history::update_at(self, perspective, coheres, batch, true);
    }

    /// Get the current cohere set for `perspective`.
    pub fn get(&self, perspective: &str) -> Option<&[Cohere]> {
        self.by_perspective.get(perspective).map(Vec::as_slice)
    }

    /// Get the history ring buffer for `perspective`.
    pub fn history(&self, perspective: &str) -> Option<&VecDeque<CohereSnapshot>> {
        self.history.get(perspective)
    }

    /// Iterate all coheres across all perspectives.
    pub fn iter_all(&self) -> impl Iterator<Item = (&str, &Cohere)> {
        self.by_perspective
            .iter()
            .flat_map(|(name, coheres)| coheres.iter().map(move |c| (name.as_str(), c)))
    }

    /// Number of active perspectives.
    pub fn perspective_count(&self) -> usize {
        self.by_perspective.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_perspective.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{CohereMembers, EntityId};

    fn cohere(id: CohereId, entity_ids: Vec<u64>, strength: f32) -> Cohere {
        Cohere {
            id,
            members: CohereMembers::Entities(entity_ids.into_iter().map(EntityId).collect()),
            strength,
        }
    }

    #[test]
    fn update_replaces_previous_coheres() {
        let mut store = CohereStore::new();
        let id1 = store.mint_id();
        store.update("default", vec![cohere(id1, vec![0], 1.0)]);
        assert_eq!(store.get("default").unwrap().len(), 1);

        let id2 = store.mint_id();
        let id3 = store.mint_id();
        store.update(
            "default",
            vec![cohere(id2, vec![1], 0.8), cohere(id3, vec![2], 0.6)],
        );
        assert_eq!(store.get("default").unwrap().len(), 2);
    }

    #[test]
    fn multiple_perspectives_coexist() {
        let mut store = CohereStore::new();
        let id_a = store.mint_id();
        let id_b = store.mint_id();
        store.update("structural", vec![cohere(id_a, vec![0, 1], 1.0)]);
        store.update("temporal", vec![cohere(id_b, vec![2, 3], 0.7)]);
        assert_eq!(store.perspective_count(), 2);
        assert!(store.get("structural").is_some());
        assert!(store.get("temporal").is_some());
    }
}
