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
/// adopts the label with the highest total signed activity weight among
/// its neighbors.  Positive-activity edges act as **attraction** (they
/// pull nodes toward the same label), while negative-activity edges act
/// as **repulsion** (they push nodes toward different labels).
///
/// Repulsion is implemented by treating a negative-weight neighbor's
/// label as a negative vote: its contribution subtracts from the score
/// of that label rather than adding to it.  The propagation step still
/// picks the label with the highest net score — a node surrounded by
/// strong positive neighbors clusters with them; a node connected to
/// inhibitory edges tends to end up in a different community.
///
/// Converges when no labels change, or after `max_iter` rounds.
fn find_communities(
    store: &RelationshipStore,
    threshold: f32,
) -> CommunityResult {
    use rustc_hash::FxHashMap;

    let mut adj: AdjMap = FxHashMap::default();
    for rel in store.iter() {
        // Include any relationship whose absolute activity meets the
        // threshold — both excitatory (positive) and inhibitory (negative).
        if rel.activity().abs() < threshold {
            continue;
        }
        let (a, b) = endpoints_pair(rel);
        // Combine activity (instantaneous signal) and Hebbian weight
        // (accumulated co-activation history). Weight is non-negative and
        // zero by default, so this is a no-op when plasticity is disabled.
        // Sign is preserved from activity so inhibitory edges still repel.
        let w = rel.activity() + rel.weight();
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
    // LCG state for deterministic per-iteration shuffling.
    // Breaks the systematic small-ID-first bias without introducing
    // a PRNG crate dependency. Seed is fixed for reproducibility.
    let mut lcg_state: u64 = 0x517cc1b727220a95;

    const MAX_ITER: usize = 15;
    for _ in 0..MAX_ITER {
        // Shuffle node order each pass (Fisher-Yates with inline LCG).
        let n = all_loci.len();
        for i in (1..n).rev() {
            lcg_state = lcg_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (lcg_state >> 33) as usize % (i + 1);
            all_loci.swap(i, j);
        }

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
///
/// Coherence = `mean_activity × density`, where density uses a log-scaled
/// reference that avoids the O(n²) penalty of the fully-connected baseline.
///
/// **Why not `n*(n-1)/2`?**  Real-world graphs (biological connectomes,
/// social networks) are sparse: edge count grows as O(n) or O(n log n),
/// not O(n²). Dividing by `n*(n-1)/2` makes any large sparse cluster
/// score near 0 simply because it exists — which destroys the signal.
///
/// **Reference formula**: `n * ln(n+1) / 2`.
/// - Grows sub-quadratically (≈ O(n log n)), matching empirical sparse
///   graph densities.
/// - For n=2 fully-connected (1 edge): `density ≈ 1/ln(3) ≈ 0.91` — close
///   to 1.0, preserving the "tight pair" signal.
/// - For n=84 with 300 active edges (biological connectome density):
///   reference ≈ 186 → `density ≈ min(300/186, 1.0) = 1.0`.
/// - For n=27 with 30 edges: reference ≈ 45 → `density ≈ 0.67`.
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
                    // Only excitatory relationships contribute to coherence.
                    // Inhibitory edges (negative activity) are part of the
                    // topology but do not add to internal binding strength.
                    if activity >= threshold {
                        sum += activity;
                        active_count += 1;
                    }
                }
            }
        }
    }
    let mean_activity = if active_count == 0 {
        0.0
    } else {
        sum / active_count as f32
    };
    // Reference edge count: n * ln(n+1) / 2.
    // Sub-quadratic so large sparse graphs score proportionally, not near 0.
    let n = member_set.len();
    let reference = if n <= 1 {
        1.0f32
    } else {
        (n as f32) * ((n as f32 + 1.0).ln()) / 2.0
    };
    let density = (active_count as f32 / reference).min(1.0);
    let coherence = mean_activity * density;
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
///
/// Uses the `by_member` reverse index on `EntityStore` to build a candidate
/// set in O(|component|) before computing Jaccard scores, instead of
/// scanning every entity unconditionally.
fn all_matches(
    member_set: &rustc_hash::FxHashSet<LocusId>,
    existing: &EntityStore,
    threshold: f32,
) -> Vec<(EntityId, f32, bool)> {
    // Candidate set: entities that share at least one member with this
    // component.  Entities with zero overlap are excluded for free.
    let candidates = existing.candidates_for_members(member_set);
    let mut matches: Vec<(EntityId, f32, bool)> = candidates
        .iter()
        .filter_map(|&id| existing.get(id))
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
        Endpoints, InfluenceKindId, KindObservation, LocusId, RelationshipLineage, StateVector,
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
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(InfluenceKindId(1))],
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
    fn component_stats_triangle_graph() {
        // Triangle: nodes 1, 2, 3 with three bidirectional adj entries.
        // adj stores (neighbor, rel_id, signed_activity) per direction.
        let r0 = RelationshipId(0);
        let r1 = RelationshipId(1);
        let r2 = RelationshipId(2);
        let mut adj: AdjMap = rustc_hash::FxHashMap::default();
        adj.entry(LocusId(1)).or_default().extend([(LocusId(2), r0, 0.8), (LocusId(3), r2, 0.7)]);
        adj.entry(LocusId(2)).or_default().extend([(LocusId(1), r0, 0.8), (LocusId(3), r1, 0.6)]);
        adj.entry(LocusId(3)).or_default().extend([(LocusId(2), r1, 0.6), (LocusId(1), r2, 0.7)]);

        let member_set: rustc_hash::FxHashSet<LocusId> =
            [LocusId(1), LocusId(2), LocusId(3)].iter().copied().collect();

        let (coherence, rel_ids) = component_stats(&member_set, &adj, 0.1);

        // 3 edges above threshold (1-2, 1-3, 2-3); nb > locus dedup gives
        // visits for pairs (1,2), (1,3), (2,3) → active_count = 3.
        // mean_activity = (0.8 + 0.7 + 0.6) / 3 = 0.7
        // reference = 3 * ln(4) / 2 ≈ 2.079; density = 3/2.079 > 1.0 → 1.0
        // coherence = 0.7 * 1.0 = 0.7
        assert_eq!(rel_ids.len(), 3);
        assert!((coherence - 0.7).abs() < 1e-4, "coherence = {coherence}");
    }

    #[test]
    fn find_communities_two_disconnected_pairs() {
        let mut store = RelationshipStore::new();
        rel(&mut store, 1, 2, 1.0);
        rel(&mut store, 3, 4, 1.0);

        let result = find_communities(&store, 0.1);

        assert_eq!(result.components.len(), 2);
        for c in &result.components {
            assert_eq!(c.len(), 2);
        }
    }

    #[test]
    fn all_matches_returns_overlapping_entities_sorted_by_score() {
        let mut store = EntityStore::new();

        // Entity A: members {1, 2, 3}
        let ea = store.mint_id();
        store.insert(Entity::born(ea, BatchId(0), EntitySnapshot {
            members: vec![LocusId(1), LocusId(2), LocusId(3)],
            member_relationships: vec![],
            coherence: 1.0,
        }));

        // Entity B: members {3, 4, 5} — overlaps on node 3
        let eb = store.mint_id();
        store.insert(Entity::born(eb, BatchId(0), EntitySnapshot {
            members: vec![LocusId(3), LocusId(4), LocusId(5)],
            member_relationships: vec![],
            coherence: 1.0,
        }));

        // query set {1, 2, 3}: overlap with A = 3/3 = 1.0, with B = 1/5 = 0.2
        let query: rustc_hash::FxHashSet<LocusId> =
            [LocusId(1), LocusId(2), LocusId(3)].iter().copied().collect();

        // threshold 0.5 → only A qualifies
        let matches = all_matches(&query, &store, 0.5);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, ea);
        assert!((matches[0].1 - 1.0).abs() < 1e-6);

        // threshold 0.1 → both qualify, sorted by descending score
        let matches_all = all_matches(&query, &store, 0.1);
        assert_eq!(matches_all.len(), 2);
        assert_eq!(matches_all[0].0, ea, "highest score first");
        assert!(matches_all[0].1 > matches_all[1].1);
    }

    #[test]
    fn component_stats_high_activity_not_capped() {
        // Activity > 1.0 should flow through without being capped.
        let r0 = RelationshipId(0);
        let mut adj: AdjMap = rustc_hash::FxHashMap::default();
        adj.entry(LocusId(1)).or_default().push((LocusId(2), r0, 5.0));
        adj.entry(LocusId(2)).or_default().push((LocusId(1), r0, 5.0));
        let member_set: rustc_hash::FxHashSet<LocusId> =
            [LocusId(1), LocusId(2)].iter().copied().collect();

        let (coherence, _) = component_stats(&member_set, &adj, 0.1);

        // mean_activity = 5.0; density ≤ 1.0 so coherence = 5.0 * density
        assert!(coherence > 1.0, "coherence should exceed 1.0 when activity > 1.0, got {coherence}");
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
