use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId, LocusId, ProposedChange,
    RelationshipId, RelationshipSlotDef, StateVector,
};
use graph_world::World;

use crate::registry::InfluenceKindRegistry;

use super::{
    BuiltChange, BuiltLocusChange, BuiltRelChange, ComputedChange, ComputedLocusChange,
    ComputedRelChange, CrossLocusPred, EmergenceResolution, PendingChange,
};

pub(crate) fn resolve_emergence(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: InfluenceKindId,
    cfg: Option<&crate::registry::InfluenceKindConfig>,
    resolved_slots: &[RelationshipSlotDef],
    pre_signal: f32,
) -> EmergenceResolution {
    let activity_contribution = cfg.map(|c| c.activity_contribution).unwrap_or(1.0);
    let endpoints = emergence_endpoints(cfg, from, to);
    let key = endpoints.key();

    debug_assert!(
        cfg.is_some(),
        "resolve_emergence: InfluenceKindId {kind:?} is not registered — call influence_registry.register() before ticking"
    );

    if let Some(rel_id) = world.relationships().lookup(&key, kind) {
        EmergenceResolution::Update { rel_id }
    } else {
        EmergenceResolution::Create {
            endpoints,
            kind,
            initial_state: emergence_initial_state(
                activity_contribution,
                pre_signal,
                resolved_slots,
            ),
            pre_signal,
            activity_contribution,
        }
    }
}

fn emergence_endpoints(
    cfg: Option<&crate::registry::InfluenceKindConfig>,
    from: LocusId,
    to: LocusId,
) -> Endpoints {
    if cfg.map(|c| c.symmetric).unwrap_or(false) {
        Endpoints::Symmetric { a: from, b: to }
    } else {
        Endpoints::Directed { from, to }
    }
}

fn emergence_initial_state(
    activity_contribution: f32,
    pre_signal: f32,
    resolved_slots: &[RelationshipSlotDef],
) -> StateVector {
    let initial_activity = activity_contribution * pre_signal.abs();
    let mut values = vec![initial_activity, 0.0f32];
    values.extend(resolved_slots.iter().map(|slot| slot.default));
    StateVector::from_slice(&values)
}

pub(crate) fn build_computed_change(
    idx: usize,
    base_id: ChangeId,
    computed: ComputedChange,
    batch: BatchId,
) -> BuiltChange {
    let id = ChangeId(base_id.0 + idx as u64);
    match computed {
        ComputedChange::Locus(c) => BuiltChange::Locus(build_locus_change(id, batch, c)),
        ComputedChange::Relationship(c) => {
            BuiltChange::Relationship(build_relationship_change(id, batch, c))
        }
        ComputedChange::Elided => unreachable!(
            "build_computed_change must not be called with Elided — filter before dispatch"
        ),
    }
}

fn build_locus_change(id: ChangeId, batch: BatchId, c: ComputedLocusChange) -> BuiltLocusChange {
    let change = Change {
        id,
        subject: ChangeSubject::Locus(c.locus_id),
        kind: c.kind,
        predecessors: c.predecessors,
        before: c.before,
        after: c.after.clone(),
        batch,
        wall_time: c.wall_time,
        metadata: c.metadata,
    };
    BuiltLocusChange {
        change,
        locus_id: c.locus_id,
        after: c.after,
        property_patch: c.property_patch,
        cross_locus_preds: c.cross_locus_preds,
        kind: c.kind,
        resolved_slots: c.resolved_slots,
        plasticity_active: c.plasticity_active,
        post_signal: c.post_signal,
    }
}

fn build_relationship_change(
    id: ChangeId,
    batch: BatchId,
    c: ComputedRelChange,
) -> BuiltRelChange {
    let change = Change {
        id,
        subject: ChangeSubject::Relationship(c.rel_id),
        kind: c.kind,
        predecessors: c.predecessors,
        before: c.before,
        after: c.after.clone(),
        batch,
        wall_time: c.wall_time,
        metadata: c.metadata,
    };
    BuiltRelChange {
        change,
        rel_id: c.rel_id,
        after: c.after,
        has_subscribers: c.has_subscribers,
        from: c.from,
        to: c.to,
        kind: c.kind,
    }
}

pub(crate) fn compute_pending_change(
    pending: PendingChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let PendingChange {
        proposed,
        derived_predecessors,
    } = pending;
    let mut predecessors = derived_predecessors;
    predecessors.extend(proposed.extra_predecessors.iter().copied());
    let kind = proposed.kind;

    match proposed.subject {
        ChangeSubject::Locus(locus_id) => compute_locus_change(
            locus_id,
            kind,
            predecessors,
            proposed,
            world,
            influence_registry,
        ),
        ChangeSubject::Relationship(rel_id) => compute_relationship_change(
            rel_id,
            kind,
            predecessors,
            proposed,
            world,
            influence_registry,
        ),
    }
}

fn compute_locus_change(
    locus_id: LocusId,
    kind: InfluenceKindId,
    predecessors: Vec<ChangeId>,
    proposed: ProposedChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let Some(locus) = world.locus(locus_id) else {
        return ComputedChange::Elided;
    };
    let before = locus.state.clone();
    let kind_cfg = influence_registry.get(kind);
    let resolved_slots = influence_registry.resolved_extra_slots(kind);
    let cross_locus_pairs = cross_locus_predecessors(world, &predecessors, locus_id);
    let stabilized_after = stabilize_locus_after(kind_cfg, &before, proposed.after);
    if !predecessors.is_empty()
        && cross_locus_pairs.is_empty()
        && stabilized_after == before
        && proposed.metadata.is_none()
        && proposed.property_patch.is_none()
    {
        return ComputedChange::Elided;
    }
    let locus_effect = locus_change_effect(
        world,
        locus_id,
        kind,
        kind_cfg,
        &resolved_slots,
        &stabilized_after,
        cross_locus_pairs,
    );
    ComputedChange::Locus(ComputedLocusChange {
        locus_id,
        kind,
        predecessors,
        before,
        after: stabilized_after,
        wall_time: proposed.wall_time,
        metadata: proposed.metadata,
        property_patch: proposed.property_patch,
        cross_locus_preds: locus_effect.cross_locus_preds,
        resolved_slots,
        plasticity_active: locus_effect.plasticity_active,
        post_signal: locus_effect.post_signal,
    })
}

struct CrossLocusPredecessor {
    from_locus: LocusId,
    pre_signal: f32,
    pred_batch: BatchId,
    pred_change_id: ChangeId,
}

struct LocusChangeEffect {
    cross_locus_preds: Vec<CrossLocusPred>,
    plasticity_active: bool,
    post_signal: f32,
}

fn cross_locus_predecessors(
    world: &World,
    predecessors: &[ChangeId],
    locus_id: LocusId,
) -> Vec<CrossLocusPredecessor> {
    predecessors
        .iter()
        .filter_map(|pid| world.log().get(*pid))
        .filter_map(|pred| match pred.subject {
            ChangeSubject::Locus(predecessor_locus)
                if predecessor_locus != locus_id && world.locus(predecessor_locus).is_some() =>
            {
                Some(CrossLocusPredecessor {
                    from_locus: predecessor_locus,
                    pre_signal: pred.after.as_slice().first().copied().unwrap_or(0.0),
                    pred_batch: pred.batch,
                    pred_change_id: pred.id,
                })
            }
            _ => None,
        })
        .collect()
}

fn stabilize_locus_after(
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    before: &StateVector,
    proposed_after: StateVector,
) -> StateVector {
    match kind_cfg {
        Some(cfg) => cfg.stabilization.stabilize(before, proposed_after),
        None => proposed_after,
    }
}

fn locus_change_effect(
    world: &World,
    locus_id: LocusId,
    kind: InfluenceKindId,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    resolved_slots: &[RelationshipSlotDef],
    stabilized_after: &StateVector,
    cross_locus_pairs: Vec<CrossLocusPredecessor>,
) -> LocusChangeEffect {
    LocusChangeEffect {
        cross_locus_preds: cross_locus_predictions(
            world,
            locus_id,
            kind,
            kind_cfg,
            resolved_slots,
            cross_locus_pairs,
        ),
        plasticity_active: kind_cfg
            .map(|cfg| cfg.plasticity.is_active())
            .unwrap_or(false),
        post_signal: stabilized_after.as_slice().first().copied().unwrap_or(0.0),
    }
}

fn cross_locus_predictions(
    world: &World,
    locus_id: LocusId,
    kind: InfluenceKindId,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    resolved_slots: &[RelationshipSlotDef],
    cross_locus_pairs: Vec<CrossLocusPredecessor>,
) -> Vec<CrossLocusPred> {
    cross_locus_pairs
        .into_iter()
        .map(|predecessor| CrossLocusPred {
            from_locus: predecessor.from_locus,
            pre_signal: predecessor.pre_signal,
            pred_batch: predecessor.pred_batch,
            pred_change_id: predecessor.pred_change_id,
            is_feedback: false,
            schema_violation: cross_locus_schema_violation(
                world,
                kind_cfg,
                predecessor.from_locus,
                locus_id,
            ),
            emergence: resolve_emergence(
                world,
                predecessor.from_locus,
                locus_id,
                kind,
                kind_cfg,
                resolved_slots,
                predecessor.pre_signal,
            ),
        })
        .collect()
}

fn cross_locus_schema_violation(
    world: &World,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    from_locus: LocusId,
    to_locus: LocusId,
) -> Option<(graph_core::LocusKindId, graph_core::LocusKindId)> {
    let cfg = kind_cfg?;
    if cfg.applies_between.is_empty() {
        return None;
    }
    let from_kind = world.locus(from_locus).map(|l| l.kind)?;
    let to_kind = world.locus(to_locus).map(|l| l.kind)?;
    (!cfg.allows_endpoint_kinds(from_kind, to_kind)).then_some((from_kind, to_kind))
}

fn compute_relationship_change(
    rel_id: RelationshipId,
    kind: InfluenceKindId,
    predecessors: Vec<ChangeId>,
    proposed: ProposedChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let Some((before, from, to)) = relationship_snapshot(world, rel_id) else {
        return ComputedChange::Elided;
    };
    let raw_after = relationship_raw_after(&before, proposed.slot_patches, proposed.after);
    let after = stabilize_relationship_after(influence_registry, kind, &before, raw_after);
    let has_subscribers = relationship_has_subscribers(world, rel_id, kind, from, to);
    ComputedChange::Relationship(ComputedRelChange {
        rel_id,
        kind,
        predecessors,
        before,
        after,
        wall_time: proposed.wall_time,
        metadata: proposed.metadata,
        from,
        to,
        has_subscribers,
    })
}

fn relationship_snapshot(
    world: &World,
    rel_id: RelationshipId,
) -> Option<(StateVector, LocusId, LocusId)> {
    let rel = world.relationships().get(rel_id)?;
    let (from, to) = match rel.endpoints {
        Endpoints::Directed { from, to } => (from, to),
        Endpoints::Symmetric { a, b } => (a, b),
    };
    Some((rel.state.clone(), from, to))
}

fn relationship_raw_after(
    before: &StateVector,
    slot_patches: Option<Vec<(usize, f32)>>,
    proposed_after: StateVector,
) -> StateVector {
    match slot_patches {
        Some(patches) => patches
            .into_iter()
            .fold(before.clone(), |state, (idx, delta)| {
                state.with_slot_delta(idx, delta)
            }),
        None => proposed_after,
    }
}

fn stabilize_relationship_after(
    influence_registry: &InfluenceKindRegistry,
    kind: InfluenceKindId,
    before: &StateVector,
    raw_after: StateVector,
) -> StateVector {
    match influence_registry.get(kind) {
        Some(cfg) => cfg.stabilization.stabilize(before, raw_after),
        None => raw_after,
    }
}

fn relationship_has_subscribers(
    world: &World,
    rel_id: RelationshipId,
    kind: InfluenceKindId,
    from: LocusId,
    to: LocusId,
) -> bool {
    world
        .subscriptions()
        .has_any_subscribers(rel_id, kind, from, to)
}
