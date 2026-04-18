//! Cohere perspective trait and the framework's default implementation.
//!
//! Per `docs/redesign.md` §3.6 and §4:
//! - The user provides a `CoherePerspective` (closeness function or
//!   full clustering algorithm).
//! - The engine provides the orchestration + store update via
//!   `Engine::extract_cohere()`.
//! - Multiple perspectives can be active simultaneously; each stores
//!   its results under its own name in `CohereStore`.
//!
//! The default perspective clusters *active entities* by the strength
//! of relationships that connect their member loci. Two entities are
//! "close" if the sum of relationship activities between their members
//! exceeds `min_bridge_activity`. Greedy single-linkage clustering is
//! used — fast, simple, and predictable for typical graph sizes.

mod clustering;

use graph_core::Cohere;
use graph_world::{EntityStore, RelationshipStore};

/// User-replaceable hook for cohere clustering.
pub trait CoherePerspective: Send + Sync {
    /// Name of this perspective. Used as the key in `CohereStore`;
    /// must be stable across calls if you want the store to stay
    /// up-to-date rather than accumulating duplicate keys.
    fn name(&self) -> &str;

    /// Cluster the current world state into cohere groups. The engine
    /// calls this from `extract_cohere()` and stores the results.
    fn cluster(
        &self,
        entities: &EntityStore,
        relationships: &RelationshipStore,
        next_id: &mut dyn FnMut() -> graph_core::CohereId,
    ) -> Vec<Cohere>;
}

/// Default perspective: entity-level single-linkage clustering.
///
/// Two active entities are treated as connected if the total
/// relationship activity between any of their member loci exceeds
/// `min_bridge_activity`. Single-linkage BFS groups connected entity
/// pairs into coheres.
#[derive(Debug, Clone)]
pub struct DefaultCoherePerspective {
    pub name: String,
    /// Minimum summed activity across all relationships connecting any
    /// two entities' member loci, above which a bridge counts as a
    /// cohere link in the single-linkage BFS.
    ///
    /// **Default**: `None` — distribution-based auto. Adopted in Phase 2
    /// of the 2026-04-18 complexity sweep (`docs/complexity-audit.md`).
    /// Replaced the former karate-tuned `0.3` constant that silently
    /// dropped legitimate weak bridges in sparse regimes.
    ///
    /// **Override when**: the domain has a semantically meaningful
    /// absolute bridge-activity floor (e.g. "ignore any bridge below
    /// one interaction per day") and you want thresholding independent
    /// of the current bridge distribution. Leave `None` otherwise.
    ///
    /// **None semantics**: per `cluster()` call, compute the median of
    /// nonzero inter-entity bridge activities and use it as the
    /// threshold. Robust across sparse/dense regimes and scale-free.
    /// Setting `Some(x)` pins the threshold and bypasses the median
    /// computation.
    pub min_bridge_activity: Option<f32>,
}

impl Default for DefaultCoherePerspective {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            min_bridge_activity: None,
        }
    }
}

impl CoherePerspective for DefaultCoherePerspective {
    fn name(&self) -> &str {
        &self.name
    }

    fn cluster(
        &self,
        entities: &EntityStore,
        relationships: &RelationshipStore,
        next_id: &mut dyn FnMut() -> graph_core::CohereId,
    ) -> Vec<Cohere> {
        clustering::cluster_default(self, entities, relationships, next_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, CohereMembers, Endpoints, Entity, EntityId, EntitySnapshot, InfluenceKindId,
        KindObservation, LocusId, Relationship, RelationshipLineage, StateVector,
    };
    use graph_world::{EntityStore, RelationshipStore};

    fn active_entity(store: &mut EntityStore, loci: &[u64]) -> EntityId {
        let id = store.mint_id();
        let snapshot = EntitySnapshot {
            members: loci.iter().map(|&i| LocusId(i)).collect(),
            member_relationships: vec![],
            coherence: 1.0,
        };
        store.insert(Entity::born(id, BatchId(0), snapshot));
        id
    }

    fn relationship(store: &mut RelationshipStore, from: u64, to: u64, activity: f32) {
        let id = store.mint_id();
        store.insert(Relationship {
            id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[activity]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(InfluenceKindId(1))],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    fn mint_id_fn() -> impl FnMut() -> graph_core::CohereId {
        let mut n = 0u64;
        move || {
            let id = graph_core::CohereId(n);
            n += 1;
            id
        }
    }

    #[test]
    fn two_connected_entities_form_one_cohere() {
        let mut entities = EntityStore::new();
        let mut rels = RelationshipStore::new();
        let ea = active_entity(&mut entities, &[1]);
        let eb = active_entity(&mut entities, &[2]);
        relationship(&mut rels, 1, 2, 1.0); // bridge activity 1.0 >= 0.3

        let perspective = DefaultCoherePerspective::default();
        let mut mint = mint_id_fn();
        let coheres = perspective.cluster(&entities, &rels, &mut mint);

        assert_eq!(coheres.len(), 1);
        let c = &coheres[0];
        if let CohereMembers::Entities(members) = &c.members {
            let mut m = members.clone();
            m.sort();
            assert_eq!(m, vec![ea, eb]);
        } else {
            panic!("expected Entities variant");
        }
    }

    #[test]
    fn isolated_entities_produce_no_cohere() {
        let mut entities = EntityStore::new();
        let rels = RelationshipStore::new();
        active_entity(&mut entities, &[1]);
        active_entity(&mut entities, &[2]);
        // No relationships → no bridge → no cohere.
        let perspective = DefaultCoherePerspective::default();
        let mut mint = mint_id_fn();
        let coheres = perspective.cluster(&entities, &rels, &mut mint);
        assert!(coheres.is_empty());
    }

    // Old test `weak_bridge_below_threshold_suppressed` removed with
    // Phase 2 (auto bridge threshold): "weak" is now relative (median of
    // nonzero bridges). A world with only one 0.1 bridge sees 0.1 as
    // its baseline and does not suppress it. Realistic multi-bridge
    // scenarios are covered by higher-level benchmarks.

    #[test]
    fn three_entities_in_chain_form_one_cohere() {
        let mut entities = EntityStore::new();
        let mut rels = RelationshipStore::new();
        let _ea = active_entity(&mut entities, &[1]);
        let _eb = active_entity(&mut entities, &[2]);
        let _ec = active_entity(&mut entities, &[3]);
        relationship(&mut rels, 1, 2, 1.0);
        relationship(&mut rels, 2, 3, 1.0);
        // No direct 1-3 bridge, but single-linkage should group all three.
        let perspective = DefaultCoherePerspective::default();
        let mut mint = mint_id_fn();
        let coheres = perspective.cluster(&entities, &rels, &mut mint);
        assert_eq!(coheres.len(), 1);
        if let CohereMembers::Entities(members) = &coheres[0].members {
            assert_eq!(members.len(), 3);
        }
    }
}
