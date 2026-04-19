use graph_core::{
    BatchId, EmergenceProposal, Endpoints, Entity, EntityId, EntityLayer, EntitySnapshot,
    EntityStatus, LayerTransition, LifecycleCause, LocusId, RelationshipId,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{AdjMap, EntityDecision, MIN_SIGNIFICANT_BUCKET, ProposalContext, community};

/// Diagnostic counters for the exclusivity filter. Process-wide, reset-able
/// from tests. See `docs/hep-ph-finding.md §4` for the investigation these
/// counters support.
pub static EXCLUSIVITY_UNCHANGED: AtomicUsize = AtomicUsize::new(0);
pub static EXCLUSIVITY_FILTERED: AtomicUsize = AtomicUsize::new(0);
pub static EXCLUSIVITY_COLLAPSED: AtomicUsize = AtomicUsize::new(0);

pub fn reset_exclusivity_counters() {
    EXCLUSIVITY_UNCHANGED.store(0, Ordering::Relaxed);
    EXCLUSIVITY_FILTERED.store(0, Ordering::Relaxed);
    EXCLUSIVITY_COLLAPSED.store(0, Ordering::Relaxed);
}

pub fn exclusivity_counters() -> (usize, usize, usize) {
    (
        EXCLUSIVITY_UNCHANGED.load(Ordering::Relaxed),
        EXCLUSIVITY_FILTERED.load(Ordering::Relaxed),
        EXCLUSIVITY_COLLAPSED.load(Ordering::Relaxed),
    )
}

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
    // Single-perspective exclusivity (redesign §3.4, HEP-PH Finding 5):
    // loci already owned by a Claims-decision entity may not end up in a
    // different entity's membership. Accumulative hub-heavy graphs cause
    // explosion when this is unenforced.
    //
    // Born path ([]): exclude every owned locus.
    // Claim path ([entity]): exclude loci owned by OTHER Claims entities —
    //   the claimer's own members are protected.
    // Merge path ([_, _, ...]): not filtered yet — the merge routing will
    //   own all absorbed entities' members anyway.
    let (members_vec, component_set_ref, member_rels) = match claimers {
        [] => match apply_exclusivity_filter(members, &empty_set(), member_rels, context) {
            ExclusivityOutcome::Collapsed => return,
            ExclusivityOutcome::Unchanged(rels) => (members.to_vec(), None, rels),
            ExclusivityOutcome::Filtered { members, set, rels } => (members, Some(set), rels),
        },
        &[entity] => {
            let protected: FxHashSet<LocusId> = context
                .existing
                .get(entity)
                .map(|e| e.current.members.iter().copied().collect())
                .unwrap_or_default();
            match apply_exclusivity_filter(members, &protected, member_rels, context) {
                ExclusivityOutcome::Collapsed => return,
                ExclusivityOutcome::Unchanged(rels) => (members.to_vec(), None, rels),
                ExclusivityOutcome::Filtered { members, set, rels } => (members, Some(set), rels),
            }
        }
        _ => (members.to_vec(), None, member_rels),
    };

    let component_set_owned: FxHashSet<LocusId>;
    let effective_set: &FxHashSet<LocusId> = match &component_set_ref {
        Some(set) => set,
        None => {
            component_set_owned = component_set.clone();
            &component_set_owned
        }
    };

    let assembly = ComponentAssembly {
        members: members_vec,
        member_relationships: member_rels,
        coherence,
    };
    if let Some(verdict) =
        derive_component_proposal_verdict(batch, &assembly, effective_set, claimers, context)
    {
        context
            .proposals
            .push(assemble_component_proposal(assembly, verdict));
    }
}

enum ExclusivityOutcome {
    /// No owned loci in this component — proceed with original data.
    Unchanged(Vec<RelationshipId>),
    /// Filtered set still ≥ `MIN_SIGNIFICANT_BUCKET` members.
    Filtered {
        members: Vec<LocusId>,
        set: FxHashSet<LocusId>,
        rels: Vec<RelationshipId>,
    },
    /// Filtered set dropped below significance — do not emit a proposal.
    Collapsed,
}

fn empty_set() -> FxHashSet<LocusId> {
    FxHashSet::default()
}

/// Unified exclusivity filter. `protected` is the set of loci that may stay
/// in the proposed members even if they are `owned_loci` elsewhere — it is
/// empty on the Born path and equals the claimer's own prev members on the
/// Claim path.
fn apply_exclusivity_filter(
    members: &[LocusId],
    protected: &FxHashSet<LocusId>,
    member_rels: Vec<RelationshipId>,
    context: &ProposalContext<'_>,
) -> ExclusivityOutcome {
    let owned = context.owned_loci;
    let is_foreign = |l: &LocusId| owned.contains(l) && !protected.contains(l);

    if members.iter().all(|l| !is_foreign(l)) {
        EXCLUSIVITY_UNCHANGED.fetch_add(1, Ordering::Relaxed);
        return ExclusivityOutcome::Unchanged(member_rels);
    }

    let filtered_members: Vec<LocusId> =
        members.iter().copied().filter(|l| !is_foreign(l)).collect();
    if filtered_members.len() < MIN_SIGNIFICANT_BUCKET {
        EXCLUSIVITY_COLLAPSED.fetch_add(1, Ordering::Relaxed);
        return ExclusivityOutcome::Collapsed;
    }
    EXCLUSIVITY_FILTERED.fetch_add(1, Ordering::Relaxed);
    let filtered_set: FxHashSet<LocusId> = filtered_members.iter().copied().collect();

    let filtered_rels: Vec<RelationshipId> = member_rels
        .into_iter()
        .filter(|rid| {
            context
                .relationships
                .get(*rid)
                .map(|rel| {
                    let (a, b) = match rel.endpoints {
                        Endpoints::Directed { from, to } => (from, to),
                        Endpoints::Symmetric { a, b } => (a, b),
                    };
                    filtered_set.contains(&a) && filtered_set.contains(&b)
                })
                .unwrap_or(false)
        })
        .collect();

    ExclusivityOutcome::Filtered {
        members: filtered_members,
        set: filtered_set,
        rels: filtered_rels,
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
