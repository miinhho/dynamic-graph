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

use graph_core::{Cohere, CohereMembers, EntityId};
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
    /// two entities' member loci. Default: 0.3.
    pub min_bridge_activity: f32,
}

impl Default for DefaultCoherePerspective {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            min_bridge_activity: 0.3,
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
        use std::collections::VecDeque;
        use graph_core::Endpoints;
        use rustc_hash::{FxHashMap, FxHashSet};

        let active: Vec<EntityId> = entities.active().map(|e| e.id).collect();
        if active.is_empty() {
            return Vec::new();
        }

        // Build locus → entity index once: O(E × avg_members).
        // Avoids O(E² × R) inner scan — instead scan relationships once O(R)
        // and look up which entity pair each relationship bridges.
        let mut locus_to_entity: FxHashMap<graph_core::LocusId, EntityId> =
            FxHashMap::default();
        for &eid in &active {
            if let Some(e) = entities.get(eid) {
                for &locus in &e.current.members {
                    locus_to_entity.insert(locus, eid);
                }
            }
        }

        // Single O(R) pass: accumulate bridge activity per (ea, eb) pair.
        // Key is (min(ea,eb), max(ea,eb)) to avoid double-counting.
        let mut pair_activity: FxHashMap<(EntityId, EntityId), f32> = FxHashMap::default();
        for rel in relationships.iter() {
            let (from, to) = match &rel.endpoints {
                Endpoints::Directed { from, to } => (*from, *to),
                Endpoints::Symmetric { a, b } => (*a, *b),
            };
            let Some(&ea) = locus_to_entity.get(&from) else { continue };
            let Some(&eb) = locus_to_entity.get(&to) else { continue };
            if ea == eb {
                continue;
            }
            let key = if ea < eb { (ea, eb) } else { (eb, ea) };
            *pair_activity.entry(key).or_default() += rel.activity();
        }

        // Build bridge adjacency from pairs above threshold.
        let mut bridges: FxHashMap<EntityId, Vec<EntityId>> = FxHashMap::default();
        for ((ea, eb), activity) in &pair_activity {
            if *activity >= self.min_bridge_activity {
                bridges.entry(*ea).or_default().push(*eb);
                bridges.entry(*eb).or_default().push(*ea);
            }
        }

        // BFS connected components over the bridge graph.
        let mut visited: FxHashSet<EntityId> = FxHashSet::default();
        let mut coheres: Vec<Cohere> = Vec::new();

        for &start in &active {
            if visited.contains(&start) {
                continue;
            }
            let mut component: Vec<EntityId> = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back(start);
            visited.insert(start);
            while let Some(node) = queue.pop_front() {
                component.push(node);
                if let Some(neighbors) = bridges.get(&node) {
                    for &nb in neighbors {
                        if !visited.contains(&nb) {
                            visited.insert(nb);
                            queue.push_back(nb);
                        }
                    }
                }
            }

            if component.len() >= 2 {
                // Strength = average bridge activity across all pairs in
                // this cohere — a proxy for how tightly bound the group is.
                let pair_count = component.len() * (component.len() - 1) / 2;
                let total_activity: f32 = bridges
                    .values()
                    .flatten()
                    .count() as f32
                    / 2.0; // each bridge counted twice above
                let strength = if pair_count > 0 {
                    (total_activity / pair_count as f32).min(1.0)
                } else {
                    0.0
                };
                let mut members = component;
                members.sort();
                coheres.push(Cohere {
                    id: next_id(),
                    members: CohereMembers::Entities(members),
                    strength,
                });
            }
            // Single-entity "clusters" are not coheres — a cluster of
            // one is just an isolated entity.
        }

        coheres
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Entity, EntitySnapshot, Endpoints, InfluenceKindId, KindObservation,
        LocusId, Relationship, RelationshipLineage, StateVector,
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

    #[test]
    fn weak_bridge_below_threshold_suppressed() {
        let mut entities = EntityStore::new();
        let mut rels = RelationshipStore::new();
        active_entity(&mut entities, &[1]);
        active_entity(&mut entities, &[2]);
        relationship(&mut rels, 1, 2, 0.1); // below 0.3 threshold
        let perspective = DefaultCoherePerspective::default();
        let mut mint = mint_id_fn();
        let coheres = perspective.cluster(&entities, &rels, &mut mint);
        assert!(coheres.is_empty());
    }

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
