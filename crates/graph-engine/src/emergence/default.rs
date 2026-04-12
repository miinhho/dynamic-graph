//! Default connected-components perspective with sediment-aware
//! reconciliation, split, merge, and revive detection.
//!
//! Algorithm:
//! 1. Keep only relationships whose activity exceeds `min_activity_threshold`.
//! 2. Weighted label propagation to find communities.
//! 3. For each component, collect all entity matches above `overlap_threshold`.
//! 4. Classify: Merge / DepositLayer / Revive / Born.
//! 5. Active entities matched by 2+ components → Split.
//! 6. Active entities matched by 0 components → Dormant.

use graph_core::{
    BatchId, EmergenceProposal, Entity, EntityId, EntityLayer, EntitySnapshot,
    EntityStatus, LayerTransition, LifecycleCause, LocusId, Relationship,
    RelationshipId,
};
use graph_world::{EntityStore, RelationshipStore};

use super::EmergencePerspective;

/// Default perspective: activity-filtered connected components,
/// sediment-aware reconciliation with split, merge, and revive detection.
#[derive(Debug, Clone)]
pub struct DefaultEmergencePerspective {
    /// Minimum relationship activity score to include an edge in
    /// the clustering graph. Default: 0.1.
    pub min_activity_threshold: f32,
    /// Member-overlap ratio (Jaccard) required to match a component to an
    /// existing entity. Default: 0.5 (O4).
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
        let community = find_communities(relationships, self.min_activity_threshold);
        let components = community.components;
        let adj = &community.adj;
        let mut proposals: Vec<EmergenceProposal> = Vec::new();

        let component_sets: Vec<rustc_hash::FxHashSet<LocusId>> = components
            .iter()
            .map(|members| members.iter().copied().collect())
            .collect();

        let component_matches: Vec<Vec<(EntityId, f32, bool)>> = component_sets
            .iter()
            .map(|member_set| all_matches(member_set, existing, self.overlap_threshold))
            .collect();

        let mut entity_to_components: rustc_hash::FxHashMap<EntityId, Vec<usize>> =
            rustc_hash::FxHashMap::default();

        let mut claimed_active: rustc_hash::FxHashSet<EntityId> =
            rustc_hash::FxHashSet::default();

        for (comp_idx, ((members, member_set), matches)) in
            components.iter().zip(component_sets.iter()).zip(component_matches.iter()).enumerate()
        {
            let active_matches: Vec<(EntityId, f32)> = matches
                .iter()
                .filter(|(_, _, is_active)| *is_active)
                .map(|(id, score, _)| (*id, *score))
                .collect();

            if active_matches.len() >= 2 {
                let (coherence, member_rels) = component_stats(member_set, adj, self.min_activity_threshold);
                let (into_id, _) = *active_matches
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap();
                let absorbed: Vec<EntityId> = active_matches
                    .iter()
                    .filter(|(id, _)| *id != into_id)
                    .map(|(id, _)| *id)
                    .collect();
                for id in &absorbed {
                    claimed_active.insert(*id);
                }
                claimed_active.insert(into_id);
                proposals.push(EmergenceProposal::Merge {
                    absorbed: absorbed.clone(),
                    into: into_id,
                    new_members: members.clone(),
                    member_relationships: member_rels,
                    coherence,
                    cause: LifecycleCause::MergedFrom { absorbed },
                });
            } else if active_matches.len() == 1 {
                entity_to_components.entry(active_matches[0].0).or_default().push(comp_idx);
            } else {
                let (coherence, member_rels) = component_stats(member_set, adj, self.min_activity_threshold);
                let dormant_match = matches
                    .iter()
                    .filter(|(_, _, is_active)| !*is_active)
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                if let Some((dormant_id, _, _)) = dormant_match {
                    let cause = LifecycleCause::RelationshipCluster {
                        key_relationships: member_rels.clone(),
                    };
                    let snapshot = EntitySnapshot {
                        members: members.clone(),
                        member_relationships: member_rels,
                        coherence,
                    };
                    claimed_active.insert(*dormant_id);
                    proposals.push(EmergenceProposal::Revive {
                        entity: *dormant_id,
                        snapshot,
                        cause,
                    });
                } else {
                    let cause = LifecycleCause::RelationshipCluster {
                        key_relationships: member_rels.clone(),
                    };
                    proposals.push(EmergenceProposal::Born {
                        members: members.clone(),
                        member_relationships: member_rels,
                        coherence,
                        parents: Vec::new(),
                        cause,
                    });
                }
            }
        }

        // Resolve entity_to_components: split vs. continue.
        for (entity_id, comp_indices) in entity_to_components {
            claimed_active.insert(entity_id);
            if comp_indices.len() >= 2 {
                let offspring: Vec<(Vec<LocusId>, Vec<graph_core::RelationshipId>, f32)> = comp_indices
                    .iter()
                    .map(|&i| {
                        let members = components[i].clone();
                        let (coh, rels) = component_stats(&component_sets[i], adj, self.min_activity_threshold);
                        (members, rels, coh)
                    })
                    .collect();
                proposals.push(EmergenceProposal::Split {
                    source: entity_id,
                    offspring,
                    cause: LifecycleCause::ComponentSplit { weak_bridges: Vec::new() },
                });
            } else {
                let members = &components[comp_indices[0]];
                let member_set = &component_sets[comp_indices[0]];
                let (coherence, member_rels) =
                    component_stats(member_set, adj, self.min_activity_threshold);
                let snapshot = EntitySnapshot {
                    members: members.clone(),
                    member_relationships: member_rels,
                    coherence,
                };
                let entity = existing.get(entity_id).expect("entity_to_components only holds known ids");
                if snapshot_changed(entity, &snapshot) {
                    let transition = membership_delta(entity, &snapshot);
                    proposals.push(EmergenceProposal::DepositLayer {
                        entity: entity_id,
                        layer: EntityLayer::new(batch, snapshot, transition),
                    });
                }
            }
        }

        // Active entities with no component match become dormant.
        for entity in existing.active() {
            if !claimed_active.contains(&entity.id) {
                let decayed: Vec<graph_core::RelationshipId> = entity
                    .current
                    .member_relationships
                    .iter()
                    .copied()
                    .filter(|rid| {
                        relationships
                            .get(*rid)
                            .map(|r| r.activity() < self.min_activity_threshold)
                            .unwrap_or(true)
                    })
                    .collect();
                proposals.push(EmergenceProposal::Dormant {
                    entity: entity.id,
                    cause: LifecycleCause::RelationshipDecay {
                        decayed_relationships: decayed,
                    },
                });
            }
        }

        proposals
    }
}

// --- helpers ---------------------------------------------------------------

/// Adjacency entry: neighbor locus, relationship id, activity weight.
type AdjEntry = (LocusId, RelationshipId, f32);

/// Adjacency list built once per `find_communities` call, reused by
/// both label propagation and `component_stats`.
type AdjMap = rustc_hash::FxHashMap<LocusId, Vec<AdjEntry>>;

/// Result of `find_communities`: the communities plus the adjacency
/// list so `component_stats` can compute coherence + rel_ids without
/// re-scanning the RelationshipStore.
struct CommunityResult {
    components: Vec<Vec<LocusId>>,
    adj: AdjMap,
}

/// Weighted label propagation over the relationship graph.
///
/// Each node starts with its own label. In each iteration, a node
/// adopts the label with the highest total activity weight among its
/// neighbors. Converges when no labels change, or after `max_iter`
/// rounds.
fn find_communities(
    store: &RelationshipStore,
    threshold: f32,
) -> CommunityResult {
    use rustc_hash::FxHashMap;

    let mut adj: AdjMap = FxHashMap::default();
    for rel in store.iter() {
        if rel.activity() < threshold {
            continue;
        }
        let (a, b) = endpoints_pair(rel);
        let w = rel.activity();
        adj.entry(a).or_default().push((b, rel.id, w));
        adj.entry(b).or_default().push((a, rel.id, w));
    }

    if adj.is_empty() {
        return CommunityResult { components: Vec::new(), adj };
    }

    let mut labels: FxHashMap<LocusId, LocusId> = adj
        .keys()
        .map(|&id| (id, id))
        .collect();

    let mut all_loci: Vec<LocusId> = adj.keys().copied().collect();
    all_loci.sort();

    let mut label_weight: FxHashMap<LocusId, f32> = FxHashMap::default();

    const MAX_ITER: usize = 15;
    for _ in 0..MAX_ITER {
        let mut changed = false;
        for &node in &all_loci {
            let neighbors = match adj.get(&node) {
                Some(ns) => ns,
                None => continue,
            };
            label_weight.clear();
            for &(nb, _, w) in neighbors {
                let nb_label = labels[&nb];
                *label_weight.entry(nb_label).or_default() += w;
            }
            if let Some((&best_label, _)) = label_weight
                .iter()
                .max_by(|(la, wa), (lb, wb)| {
                    wa.partial_cmp(wb)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| lb.0.cmp(&la.0))
                })
            {
                let current = labels.get_mut(&node).unwrap();
                if *current != best_label {
                    *current = best_label;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut groups: rustc_hash::FxHashMap<LocusId, Vec<LocusId>> = FxHashMap::default();
    for (&node, &label) in &labels {
        groups.entry(label).or_default().push(node);
    }
    let mut components: Vec<Vec<LocusId>> = groups.into_values().collect();
    for c in &mut components {
        c.sort();
    }
    components.sort_by(|a, b| a[0].0.cmp(&b[0].0));
    CommunityResult { components, adj }
}

fn endpoints_pair(rel: &Relationship) -> (LocusId, LocusId) {
    use graph_core::Endpoints;
    match &rel.endpoints {
        Endpoints::Directed { from, to } => (*from, *to),
        Endpoints::Symmetric { a, b } => (*a, *b),
    }
}

/// Compute coherence and member relationship ids for a component using
/// the pre-built adjacency list.
fn component_stats(
    member_set: &rustc_hash::FxHashSet<LocusId>,
    adj: &AdjMap,
    threshold: f32,
) -> (f32, Vec<RelationshipId>) {
    let mut sum = 0.0f32;
    let mut active_count = 0usize;
    let mut rel_ids = Vec::new();
    for &locus in member_set {
        if let Some(neighbors) = adj.get(&locus) {
            for &(nb, rel_id, activity) in neighbors {
                if nb > locus && member_set.contains(&nb) {
                    rel_ids.push(rel_id);
                    if activity >= threshold {
                        sum += activity;
                        active_count += 1;
                    }
                }
            }
        }
    }
    let coherence = if active_count == 0 { 0.0 } else { (sum / active_count as f32).min(1.0) };
    (coherence, rel_ids)
}

/// Jaccard-like overlap: |intersection| / |union| of member sets.
fn overlap(a: &rustc_hash::FxHashSet<LocusId>, b: &[LocusId]) -> f32 {
    let intersection = b.iter().filter(|x| a.contains(x)).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 { 1.0 } else { intersection as f32 / union as f32 }
}

/// Collect all entities (active and dormant) whose member overlap with
/// `members` is at least `threshold`.
fn all_matches(
    member_set: &rustc_hash::FxHashSet<LocusId>,
    existing: &EntityStore,
    threshold: f32,
) -> Vec<(EntityId, f32, bool)> {
    let mut matches: Vec<(EntityId, f32, bool)> = existing
        .iter()
        .map(|e| {
            let score = overlap(member_set, &e.current.members);
            let active = e.status == EntityStatus::Active;
            (e.id, score, active)
        })
        .filter(|(_, score, _)| *score >= threshold)
        .collect();
    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    matches
}

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
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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

        let born_count = proposals
            .iter()
            .filter(|p| matches!(p, EmergenceProposal::Born { .. }))
            .count();
        assert_eq!(born_count, 2, "{proposals:?}");
    }

    #[test]
    fn low_activity_edge_excluded_from_clustering() {
        let mut store = RelationshipStore::new();
        rel(&mut store, 1, 2, 0.05);
        let perspective = DefaultEmergencePerspective::default();
        let entities = EntityStore::new();
        let proposals = perspective.recognize(&store, &entities, BatchId(0));
        assert!(proposals.is_empty());
    }

    #[test]
    fn continuation_produces_deposit_layer_not_new_born() {
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
        assert_eq!(born_count, 0, "{proposals:?}");
    }

    #[test]
    fn active_entity_with_no_component_becomes_dormant() {
        let rel_store = RelationshipStore::new();
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
            .any(|p| matches!(p, EmergenceProposal::Dormant { entity, .. } if *entity == eid));
        assert!(dormant, "{proposals:?}");
    }
}
