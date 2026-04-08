//! Entity emergence: perspective trait and the framework's default
//! connected-components implementation.
//!
//! Per O3 in `docs/redesign.md` §8, the default perspective ships
//! without configuration: connected components in the relationship
//! graph above a minimum activity threshold, reconciled against
//! existing entity sediments using member overlap + hysteresis.
//!
//! The trait is `Send + Sync` so callers can store a `Box<dyn
//! EmergencePerspective>` safely alongside parallel locus processing.

use graph_core::{
    BatchId, EmergenceProposal, Entity, EntityId, EntityLayer, EntitySnapshot,
    LayerTransition, LocusId, Relationship,
};
use graph_world::{EntityStore, RelationshipStore};

/// User-replaceable hook for recognizing coherent bundles of loci.
pub trait EmergencePerspective: Send + Sync {
    /// Examine the current relationship graph (and existing sediments)
    /// and return a list of proposals for the engine to apply.
    ///
    /// The perspective is a *pure observer*: it does not mutate the
    /// store. The engine applies proposals atomically after the call.
    fn recognize(
        &self,
        relationships: &RelationshipStore,
        existing: &EntityStore,
        batch: BatchId,
    ) -> Vec<EmergenceProposal>;
}

/// Default perspective: activity-filtered connected components,
/// sediment-aware reconciliation.
///
/// Algorithm:
/// 1. Keep only relationships whose activity exceeds
///    `min_activity_threshold`.
/// 2. Build an undirected adjacency list from those edges (direction is
///    discarded for clustering; it's preserved in the relationship
///    records for lineage tracing).
/// 3. BFS/DFS to find connected components (each component is a
///    candidate entity).
/// 4. For each component, find the best-matching existing *active*
///    entity by member overlap ratio. If overlap ≥ `overlap_threshold`
///    for several batches (`hysteresis_batches` consecutive recognitions)
///    it's a continuation; otherwise it's a new birth.
/// 5. Existing active entities with no matching component this round
///    become dormant proposals.
#[derive(Debug, Clone)]
pub struct DefaultEmergencePerspective {
    /// Minimum relationship activity score to include an edge in
    /// the clustering graph. Default: 0.1.
    pub min_activity_threshold: f32,
    /// Member-overlap ratio required to treat a component as a
    /// continuation of an existing entity. Default: 0.5 (O4).
    pub overlap_threshold: f32,
}

impl Default for DefaultEmergencePerspective {
    fn default() -> Self {
        Self {
            min_activity_threshold: 0.1,
            overlap_threshold: 0.5,
        }
    }
}

impl EmergencePerspective for DefaultEmergencePerspective {
    fn recognize(
        &self,
        relationships: &RelationshipStore,
        existing: &EntityStore,
        batch: BatchId,
    ) -> Vec<EmergenceProposal> {
        let components = find_connected_components(relationships, self.min_activity_threshold);
        let mut proposals: Vec<EmergenceProposal> = Vec::new();
        let mut matched_entity_ids: Vec<EntityId> = Vec::new();

        for members in components {
            let coherence = coherence_score(&members, relationships, self.min_activity_threshold);
            let snapshot = EntitySnapshot {
                members: members.clone(),
                member_relationships: relationships_in_component(&members, relationships),
                coherence,
            };

            match best_match(&members, existing, self.overlap_threshold) {
                Some(entity_id) => {
                    matched_entity_ids.push(entity_id);
                    let existing_entity = existing.get(entity_id).unwrap();
                    // Only deposit a layer if the snapshot changed
                    // meaningfully from the current one.
                    if snapshot_changed(existing_entity, &snapshot) {
                        let transition =
                            membership_delta(existing_entity, &snapshot);
                        proposals.push(EmergenceProposal::DepositLayer {
                            entity: entity_id,
                            layer: EntityLayer::new(batch, snapshot, transition),
                        });
                    }
                }
                None => {
                    proposals.push(EmergenceProposal::Born {
                        members,
                        coherence,
                        parents: Vec::new(),
                    });
                }
            }
        }

        // Active entities with no component match become dormant.
        for entity in existing.active() {
            if !matched_entity_ids.contains(&entity.id) {
                proposals.push(EmergenceProposal::Dormant { entity: entity.id });
            }
        }

        proposals
    }
}

// --- helpers ---------------------------------------------------------------

/// BFS connected-components over the subset of relationships whose
/// activity exceeds `threshold`. Returns groups of `LocusId`s.
fn find_connected_components(
    store: &RelationshipStore,
    threshold: f32,
) -> Vec<Vec<LocusId>> {
    use std::collections::VecDeque;
    use rustc_hash::{FxHashMap, FxHashSet};

    // Build undirected adjacency (direction irrelevant for clustering).
    let mut adj: FxHashMap<LocusId, Vec<LocusId>> = FxHashMap::default();
    for rel in store.iter() {
        if rel.activity() < threshold {
            continue;
        }
        let (a, b) = endpoints_pair(rel);
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }

    let all_loci: Vec<LocusId> = adj.keys().copied().collect();
    let mut visited: FxHashSet<LocusId> = FxHashSet::default();
    let mut components: Vec<Vec<LocusId>> = Vec::new();

    for &start in &all_loci {
        if visited.contains(&start) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);
        while let Some(node) = queue.pop_front() {
            component.push(node);
            if let Some(neighbors) = adj.get(&node) {
                for &nb in neighbors {
                    if !visited.contains(&nb) {
                        visited.insert(nb);
                        queue.push_back(nb);
                    }
                }
            }
        }
        component.sort();
        components.push(component);
    }

    components
}

fn endpoints_pair(rel: &Relationship) -> (LocusId, LocusId) {
    use graph_core::Endpoints;
    match &rel.endpoints {
        Endpoints::Directed { from, to } => (*from, *to),
        Endpoints::Symmetric { a, b } => (*a, *b),
        Endpoints::NAry(ids) => (ids[0], ids[ids.len() - 1]),
    }
}

/// Average activity of relationships fully contained within the
/// component, normalised to [0,1] by clamping at 1.0.
fn coherence_score(
    members: &[LocusId],
    store: &RelationshipStore,
    threshold: f32,
) -> f32 {
    let internal: Vec<f32> = store
        .iter()
        .filter(|r| {
            let (a, b) = endpoints_pair(r);
            members.contains(&a) && members.contains(&b) && r.activity() >= threshold
        })
        .map(|r| r.activity())
        .collect();
    if internal.is_empty() {
        return 0.0;
    }
    (internal.iter().sum::<f32>() / internal.len() as f32).min(1.0)
}

fn relationships_in_component(
    members: &[LocusId],
    store: &RelationshipStore,
) -> Vec<graph_core::RelationshipId> {
    store
        .iter()
        .filter(|r| {
            let (a, b) = endpoints_pair(r);
            members.contains(&a) && members.contains(&b)
        })
        .map(|r| r.id)
        .collect()
}

/// Jaccard-like overlap: |intersection| / |union| of member sets.
fn overlap(a: &[LocusId], b: &[LocusId]) -> f32 {
    let intersection = a.iter().filter(|x| b.contains(x)).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 { 1.0 } else { intersection as f32 / union as f32 }
}

/// Find the active entity with the highest member overlap above
/// `threshold`, if any.
fn best_match(
    members: &[LocusId],
    existing: &EntityStore,
    threshold: f32,
) -> Option<EntityId> {
    existing
        .active()
        .map(|e| (e.id, overlap(members, &e.current.members)))
        .filter(|(_, score)| *score >= threshold)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(id, _)| id)
}

/// True if the snapshot differs enough from the entity's current state
/// to justify depositing a new layer.
fn snapshot_changed(entity: &Entity, new: &EntitySnapshot) -> bool {
    if entity.current.members != new.members {
        return true;
    }
    (entity.current.coherence - new.coherence).abs() > 0.05
}

fn membership_delta(entity: &Entity, new: &EntitySnapshot) -> LayerTransition {
    let old = &entity.current.members;
    let added: Vec<LocusId> = new.members.iter().filter(|m| !old.contains(m)).copied().collect();
    let removed: Vec<LocusId> = old.iter().filter(|m| !new.members.contains(m)).copied().collect();
    if added.is_empty() && removed.is_empty() {
        LayerTransition::CoherenceShift {
            from: entity.current.coherence,
            to: new.coherence,
        }
    } else {
        LayerTransition::MembershipDelta { added, removed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, LocusId, RelationshipLineage, StateVector,
    };
    use graph_world::{RelationshipStore, EntityStore};
    use graph_core::Relationship;

    fn rel(
        store: &mut RelationshipStore,
        from: u64,
        to: u64,
        activity: f32,
    ) {
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
                kinds_observed: vec![InfluenceKindId(1)],
            },
        });
    }

    #[test]
    fn finds_two_components_from_disconnected_pairs() {
        let mut store = RelationshipStore::new();
        rel(&mut store, 1, 2, 1.0);
        rel(&mut store, 3, 4, 1.0);

        let perspective = DefaultEmergencePerspective::default();
        let entities = EntityStore::new();
        let proposals = perspective.recognize(&store, &entities, BatchId(0));

        // Two components -> two Born proposals.
        let born_count = proposals
            .iter()
            .filter(|p| matches!(p, EmergenceProposal::Born { .. }))
            .count();
        assert_eq!(born_count, 2, "{proposals:?}");
    }

    #[test]
    fn low_activity_edge_excluded_from_clustering() {
        let mut store = RelationshipStore::new();
        rel(&mut store, 1, 2, 0.05); // below threshold 0.1
        let perspective = DefaultEmergencePerspective::default();
        let entities = EntityStore::new();
        let proposals = perspective.recognize(&store, &entities, BatchId(0));
        assert!(proposals.is_empty(), "below-threshold edge must not produce entity");
    }

    #[test]
    fn continuation_produces_deposit_layer_not_new_born() {
        // One component {L1,L2} with a pre-existing active entity
        // covering the same members → DepositLayer (no Born).
        let mut rel_store = RelationshipStore::new();
        rel(&mut rel_store, 1, 2, 1.0);

        let mut entity_store = EntityStore::new();
        let eid = entity_store.mint_id();
        let snapshot = EntitySnapshot {
            members: vec![LocusId(1), LocusId(2)],
            member_relationships: vec![],
            coherence: 1.0,
        };
        entity_store.insert(Entity::born(eid, BatchId(0), snapshot));

        let perspective = DefaultEmergencePerspective::default();
        let proposals = perspective.recognize(&rel_store, &entity_store, BatchId(1));

        let born_count = proposals
            .iter()
            .filter(|p| matches!(p, EmergenceProposal::Born { .. }))
            .count();
        assert_eq!(born_count, 0, "should not born again: {proposals:?}");
    }

    #[test]
    fn active_entity_with_no_component_becomes_dormant() {
        let rel_store = RelationshipStore::new(); // no relationships
        let mut entity_store = EntityStore::new();
        let eid = entity_store.mint_id();
        let snapshot = EntitySnapshot {
            members: vec![LocusId(1)],
            member_relationships: vec![],
            coherence: 1.0,
        };
        entity_store.insert(Entity::born(eid, BatchId(0), snapshot));

        let perspective = DefaultEmergencePerspective::default();
        let proposals = perspective.recognize(&rel_store, &entity_store, BatchId(1));

        let dormant = proposals
            .iter()
            .any(|p| matches!(p, EmergenceProposal::Dormant { entity } if *entity == eid));
        assert!(dormant, "orphaned active entity must become dormant: {proposals:?}");
    }
}
