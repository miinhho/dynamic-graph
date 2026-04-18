use graph_core::{
    BatchId, EmergenceProposal, Entity, EntityId, EntityLayer, EntitySnapshot, EntityStatus,
    LayerTransition, LifecycleCause, LocusId, RelationshipId,
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{AdjMap, EntityDecision, MIN_SIGNIFICANT_BUCKET, ProposalContext, community};

struct ComponentAssembly {
    members: Vec<LocusId>,
    member_relationships: Vec<RelationshipId>,
    coherence: f32,
}

enum EntityProposalVerdict {
    Dormant {
        entity: EntityId,
        decayed_relationships: Vec<RelationshipId>,
    },
    Split {
        source: EntityId,
        offspring_components: Vec<usize>,
    },
    Claims,
}

enum ComponentProposalVerdict {
    Revive {
        entity: EntityId,
    },
    Born,
    DepositLayer {
        batch: BatchId,
        entity: EntityId,
        transition: LayerTransition,
    },
    Merge {
        into: EntityId,
        absorbed: Vec<EntityId>,
    },
}

pub(super) fn emit_entity_proposals(
    decisions: &FxHashMap<EntityId, EntityDecision>,
    components: &[Vec<LocusId>],
    component_sets: &[FxHashSet<LocusId>],
    adj: &AdjMap,
    context: &mut ProposalContext<'_>,
) -> FxHashSet<usize> {
    let mut split_offspring_components: FxHashSet<usize> = FxHashSet::default();

    for (&entity_id, decision) in decisions {
        match derive_entity_proposal_verdict(entity_id, decision, context) {
            EntityProposalVerdict::Dormant {
                entity,
                decayed_relationships,
            } => context
                .proposals
                .push(assemble_dormant_proposal(entity, decayed_relationships)),
            EntityProposalVerdict::Split {
                source,
                offspring_components,
            } => {
                split_offspring_components.extend(offspring_components.iter().copied());
                context.proposals.push(assemble_split_proposal(
                    source,
                    &offspring_components,
                    components,
                    component_sets,
                    adj,
                    context.threshold,
                ));
            }
            EntityProposalVerdict::Claims => {}
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
    let assembly = ComponentAssembly {
        members: members.to_vec(),
        member_relationships: member_rels,
        coherence,
    };
    if let Some(verdict) =
        derive_component_proposal_verdict(batch, &assembly, component_set, claimers, context)
    {
        context
            .proposals
            .push(assemble_component_proposal(assembly, verdict));
    }
}

fn derive_entity_proposal_verdict(
    entity_id: EntityId,
    decision: &EntityDecision,
    context: &ProposalContext<'_>,
) -> EntityProposalVerdict {
    match decision {
        EntityDecision::Dormant => EntityProposalVerdict::Dormant {
            entity: entity_id,
            decayed_relationships: collect_decayed_relationships(entity_id, context),
        },
        EntityDecision::Split(comp_idxs) => EntityProposalVerdict::Split {
            source: entity_id,
            offspring_components: comp_idxs.clone(),
        },
        EntityDecision::Claims(_) => EntityProposalVerdict::Claims,
    }
}

fn collect_decayed_relationships(
    entity_id: EntityId,
    context: &ProposalContext<'_>,
) -> Vec<RelationshipId> {
    context
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
        .unwrap_or_default()
}

fn assemble_dormant_proposal(
    entity: EntityId,
    decayed_relationships: Vec<RelationshipId>,
) -> EmergenceProposal {
    EmergenceProposal::Dormant {
        entity,
        cause: LifecycleCause::RelationshipDecay {
            decayed_relationships,
        },
    }
}

fn assemble_split_proposal(
    source: EntityId,
    offspring_components: &[usize],
    components: &[Vec<LocusId>],
    component_sets: &[FxHashSet<LocusId>],
    adj: &AdjMap,
    threshold: f32,
) -> EmergenceProposal {
    let offspring = offspring_components
        .iter()
        .map(|&component_idx| {
            let (coherence, member_relationships) =
                community::component_stats(&component_sets[component_idx], adj, threshold);
            (
                components[component_idx].clone(),
                member_relationships,
                coherence,
            )
        })
        .collect();

    EmergenceProposal::Split {
        source,
        offspring,
        cause: LifecycleCause::ComponentSplit {
            weak_bridges: Vec::new(),
        },
    }
}

fn derive_component_proposal_verdict(
    batch: BatchId,
    assembly: &ComponentAssembly,
    component_set: &FxHashSet<LocusId>,
    claimers: &[EntityId],
    context: &ProposalContext<'_>,
) -> Option<ComponentProposalVerdict> {
    match claimers {
        [] => Some(derive_unclaimed_component_verdict(component_set, context)),
        [entity] => derive_single_claimer_verdict(batch, assembly, *entity, context),
        _ => Some(derive_multi_claimer_verdict(claimers, context)),
    }
}

fn derive_unclaimed_component_verdict(
    component_set: &FxHashSet<LocusId>,
    context: &ProposalContext<'_>,
) -> ComponentProposalVerdict {
    find_dormant_match(component_set, context)
        .map(|entity| ComponentProposalVerdict::Revive { entity })
        .unwrap_or(ComponentProposalVerdict::Born)
}

fn find_dormant_match(
    component_set: &FxHashSet<LocusId>,
    context: &ProposalContext<'_>,
) -> Option<EntityId> {
    context
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
        .max_by_key(|&(_, overlap)| overlap)
        .map(|(entity_id, _)| entity_id)
}

fn derive_single_claimer_verdict(
    batch: BatchId,
    assembly: &ComponentAssembly,
    entity_id: EntityId,
    context: &ProposalContext<'_>,
) -> Option<ComponentProposalVerdict> {
    let entity = context
        .existing
        .get(entity_id)
        .expect("claimers contain only live active entity ids");
    let snapshot = build_snapshot(assembly);

    if snapshot_changed(entity, &snapshot) {
        Some(ComponentProposalVerdict::DepositLayer {
            batch,
            entity: entity_id,
            transition: membership_delta(entity, &snapshot),
        })
    } else {
        None
    }
}

fn derive_multi_claimer_verdict(
    claimers: &[EntityId],
    context: &ProposalContext<'_>,
) -> ComponentProposalVerdict {
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
    let absorbed = claimers.iter().copied().filter(|id| *id != into).collect();

    ComponentProposalVerdict::Merge { into, absorbed }
}

fn assemble_component_proposal(
    assembly: ComponentAssembly,
    verdict: ComponentProposalVerdict,
) -> EmergenceProposal {
    let snapshot = build_snapshot(&assembly);

    match verdict {
        ComponentProposalVerdict::Revive { entity } => EmergenceProposal::Revive {
            entity,
            snapshot,
            cause: LifecycleCause::RelationshipCluster {
                key_relationships: assembly.member_relationships,
            },
        },
        ComponentProposalVerdict::Born => EmergenceProposal::Born {
            members: assembly.members,
            member_relationships: assembly.member_relationships.clone(),
            coherence: assembly.coherence,
            parents: Vec::new(),
            cause: LifecycleCause::RelationshipCluster {
                key_relationships: assembly.member_relationships,
            },
        },
        ComponentProposalVerdict::DepositLayer {
            batch,
            entity,
            transition,
        } => EmergenceProposal::DepositLayer {
            entity,
            layer: EntityLayer::new(batch, snapshot, transition),
        },
        ComponentProposalVerdict::Merge { into, absorbed } => EmergenceProposal::Merge {
            absorbed: absorbed.clone(),
            into,
            new_members: assembly.members,
            member_relationships: assembly.member_relationships,
            coherence: assembly.coherence,
            cause: LifecycleCause::MergedFrom { absorbed },
        },
    }
}

fn build_snapshot(assembly: &ComponentAssembly) -> EntitySnapshot {
    EntitySnapshot {
        members: assembly.members.clone(),
        member_relationships: assembly.member_relationships.clone(),
        coherence: assembly.coherence,
    }
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
