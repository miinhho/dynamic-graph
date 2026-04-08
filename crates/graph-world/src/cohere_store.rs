//! Store for cohere clusters (Layer 4).
//!
//! Unlike the entity store, coheres are *ephemeral by design* — each
//! call to `extract_cohere` replaces the previous set for a given
//! perspective key. The store is indexed by a user-supplied perspective
//! name so multiple perspectives can coexist.

use std::collections::HashMap;

use graph_core::{Cohere, CohereId};

#[derive(Debug, Default, Clone)]
pub struct CohereStore {
    /// Perspective name → current set of coheres for that perspective.
    by_perspective: HashMap<String, Vec<Cohere>>,
    next_id: u64,
}

impl CohereStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mint_id(&mut self) -> CohereId {
        let id = CohereId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Replace the cohere set for `perspective` entirely. Old coheres
    /// for the same perspective are discarded — coheres are not
    /// sedimentary; they are a live view.
    pub fn update(&mut self, perspective: impl Into<String>, coheres: Vec<Cohere>) {
        self.by_perspective.insert(perspective.into(), coheres);
    }

    /// Get the current cohere set for `perspective`.
    pub fn get(&self, perspective: &str) -> Option<&[Cohere]> {
        self.by_perspective.get(perspective).map(Vec::as_slice)
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
        store.update("default", vec![cohere(id2, vec![1], 0.8), cohere(id3, vec![2], 0.6)]);
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
