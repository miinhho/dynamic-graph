use graph_core::{
    BatchId, EmergenceProposal, Entity, EntityId, EntityLayer, EntitySnapshot, EntityStatus,
    LayerTransition, LifecycleCause, LocusId, RelationshipId,
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{AdjMap, EntityDecision, MIN_SIGNIFICANT_BUCKET, ProposalContext, community};

pub(super) fn emit_entity_proposals(
    decisions: &FxHashMap<EntityId, EntityDecision>,
    components: &[Vec<LocusId>],
    component_sets: &[FxHashSet<LocusId>],
    adj: &AdjMap,
    context: &mut ProposalContext<'_>,
) -> FxHashSet<usize> {
    let mut split_offspring_components: FxHashSet<usize> = FxHashSet::default();

    for (&entity_id, decision) in decisions {
        match decision {
            EntityDecision::Dormant => {
                let decayed: Vec<RelationshipId> = context
                    .existing
                    .get(entity_id)
                    .map(|e| {
                        e.current
                            .member_relationships
                            .iter()
                            .copied()
                            .filter(|rid| {
                                context
                                    .relationships
                                    .get(*rid)
                                    .map(|r| r.activity() < context.threshold)
                                    .unwrap_or(true)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                context.proposals.push(EmergenceProposal::Dormant {
                    entity: entity_id,
                    cause: LifecycleCause::RelationshipDecay {
                        decayed_relationships: decayed,
                    },
                });
            }
            EntityDecision::Split(comp_idxs) => {
                let mut offspring = Vec::with_capacity(comp_idxs.len());
                for &i in comp_idxs {
                    let (coh, rels) =
                        community::component_stats(&component_sets[i], adj, context.threshold);
                    offspring.push((components[i].clone(), rels, coh));
                    split_offspring_components.insert(i);
                }
                context.proposals.push(EmergenceProposal::Split {
                    source: entity_id,
                    offspring,
                    cause: LifecycleCause::ComponentSplit {
                        weak_bridges: Vec::new(),
                    },
                });
            }
            EntityDecision::Claims => {}
        }
    }

    split_offspring_components
}

pub(super) fn resolve_component_proposal(
    batch: BatchId,
    members: &[LocusId],
    component_set: &FxHashSet<LocusId>,
    claimers: &[EntityId],
    member_rels: Vec<RelationshipId>,
    coherence: f32,
    context: &mut ProposalContext<'_>,
) {
    match claimers.len() {
        0 => resolve_unclaimed_component(members, component_set, member_rels, coherence, context),
        1 => resolve_single_claimer(batch, members, claimers[0], member_rels, coherence, context),
        _ => resolve_multi_claimer(members, claimers, member_rels, coherence, context),
    }
}

fn resolve_unclaimed_component(
    members: &[LocusId],
    component_set: &FxHashSet<LocusId>,
    member_rels: Vec<RelationshipId>,
    coherence: f32,
    context: &mut ProposalContext<'_>,
) {
    let dormant_match = context
        .existing
        .iter()
        .filter(|e| e.status == EntityStatus::Dormant)
        .filter_map(|e| {
            let overlap = e
                .current
                .members
                .iter()
                .filter(|l| component_set.contains(l))
                .count();
            if overlap >= MIN_SIGNIFICANT_BUCKET && overlap * 2 >= e.current.members.len() {
                Some((e.id, overlap))
            } else {
                None
            }
        })
        .max_by_key(|&(_, overlap)| overlap);

    if let Some((dormant_id, _)) = dormant_match {
        let snapshot = EntitySnapshot {
            members: members.to_vec(),
            member_relationships: member_rels.clone(),
            coherence,
        };
        context.proposals.push(EmergenceProposal::Revive {
            entity: dormant_id,
            snapshot,
            cause: LifecycleCause::RelationshipCluster {
                key_relationships: member_rels,
            },
        });
    } else {
        context.proposals.push(EmergenceProposal::Born {
            members: members.to_vec(),
            member_relationships: member_rels.clone(),
            coherence,
            parents: Vec::new(),
            cause: LifecycleCause::RelationshipCluster {
                key_relationships: member_rels,
            },
        });
    }
}

fn resolve_single_claimer(
    batch: BatchId,
    members: &[LocusId],
    entity_id: EntityId,
    member_rels: Vec<RelationshipId>,
    coherence: f32,
    context: &mut ProposalContext<'_>,
) {
    let entity = context
        .existing
        .get(entity_id)
        .expect("claimers contain only live active entity ids");
    let snapshot = EntitySnapshot {
        members: members.to_vec(),
        member_relationships: member_rels,
        coherence,
    };
    if snapshot_changed(entity, &snapshot) {
        let transition = membership_delta(entity, &snapshot);
        context.proposals.push(EmergenceProposal::DepositLayer {
            entity: entity_id,
            layer: EntityLayer::new(batch, snapshot, transition),
        });
    }
}

fn resolve_multi_claimer(
    members: &[LocusId],
    claimers: &[EntityId],
    member_rels: Vec<RelationshipId>,
    coherence: f32,
    context: &mut ProposalContext<'_>,
) {
    let into = *claimers
        .iter()
        .max_by_key(|id| {
            context
                .existing
                .get(**id)
                .map(|e| e.current.members.len())
                .unwrap_or(0)
        })
        .expect("claimers non-empty in ≥2 branch");
    let absorbed: Vec<EntityId> = claimers.iter().copied().filter(|id| *id != into).collect();
    context.proposals.push(EmergenceProposal::Merge {
        absorbed: absorbed.clone(),
        into,
        new_members: members.to_vec(),
        member_relationships: member_rels,
        coherence,
        cause: LifecycleCause::MergedFrom { absorbed },
    });
}

fn snapshot_changed(entity: &Entity, new: &EntitySnapshot) -> bool {
    if entity.current.members != new.members {
        return true;
    }
    (entity.current.coherence - new.coherence).abs() > 0.05
}

fn membership_delta(entity: &Entity, new: &EntitySnapshot) -> LayerTransition {
    let old = &entity.current.members;
    let added: Vec<LocusId> = new
        .members
        .iter()
        .filter(|m| !old.contains(m))
        .copied()
        .collect();
    let removed: Vec<LocusId> = old
        .iter()
        .filter(|m| !new.members.contains(m))
        .copied()
        .collect();
    if added.is_empty() && removed.is_empty() {
        LayerTransition::CoherenceShift {
            from: entity.current.coherence,
            to: new.coherence,
        }
    } else {
        LayerTransition::MembershipDelta { added, removed }
    }
}
