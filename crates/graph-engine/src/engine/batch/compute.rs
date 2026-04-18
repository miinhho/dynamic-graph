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
    let endpoints = if cfg.map(|c| c.symmetric).unwrap_or(false) {
        Endpoints::Symmetric { a: from, b: to }
    } else {
        Endpoints::Directed { from, to }
    };
    let key = endpoints.key();
    let store = world.relationships();

    debug_assert!(
        cfg.is_some(),
        "resolve_emergence: InfluenceKindId {kind:?} is not registered — call influence_registry.register() before ticking"
    );

    if let Some(rel_id) = store.lookup(&key, kind) {
        EmergenceResolution::Update { rel_id }
    } else {
        let initial_activity = activity_contribution * pre_signal.abs();
        let mut values = vec![initial_activity, 0.0f32];
        for slot in resolved_slots {
            values.push(slot.default);
        }
        EmergenceResolution::Create {
            endpoints,
            kind,
            initial_state: StateVector::from_slice(&values),
            pre_signal,
            activity_contribution,
        }
    }
}

pub(crate) fn build_computed_change(
    idx: usize,
    base_id: ChangeId,
    computed: ComputedChange,
    batch: BatchId,
) -> BuiltChange {
    let id = ChangeId(base_id.0 + idx as u64);
    match computed {
        ComputedChange::Locus(c) => {
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
            BuiltChange::Locus(BuiltLocusChange {
                change,
                locus_id: c.locus_id,
                after: c.after,
                property_patch: c.property_patch,
                cross_locus_preds: c.cross_locus_preds,
                kind: c.kind,
                resolved_slots: c.resolved_slots,
                plasticity_active: c.plasticity_active,
                post_signal: c.post_signal,
            })
        }
        ComputedChange::Relationship(c) => {
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
            BuiltChange::Relationship(BuiltRelChange {
                change,
                rel_id: c.rel_id,
                after: c.after,
                has_subscribers: c.has_subscribers,
                from: c.from,
                to: c.to,
                kind: c.kind,
            })
        }
        ComputedChange::Elided => unreachable!(
            "build_computed_change must not be called with Elided — filter before dispatch"
        ),
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
    let cross_locus_pairs: Vec<(LocusId, f32, BatchId, ChangeId)> = predecessors
        .iter()
        .filter_map(|pid| world.log().get(*pid))
        .filter_map(|pred| match pred.subject {
            ChangeSubject::Locus(pl) if pl != locus_id && world.locus(pl).is_some() => {
                let pre = pred.after.as_slice().first().copied().unwrap_or(0.0);
                Some((pl, pre, pred.batch, pred.id))
            }
            _ => None,
        })
        .collect();
    let kind_cfg = influence_registry.get(kind);
    let resolved_slots = influence_registry.resolved_extra_slots(kind);
    let stabilized_after = match kind_cfg {
        Some(cfg) => cfg.stabilization.stabilize(&before, proposed.after),
        None => proposed.after,
    };
    if !predecessors.is_empty()
        && cross_locus_pairs.is_empty()
        && stabilized_after == before
        && proposed.metadata.is_none()
        && proposed.property_patch.is_none()
    {
        return ComputedChange::Elided;
    }
    let post_signal = stabilized_after.as_slice().first().copied().unwrap_or(0.0);
    let plasticity_active = kind_cfg
        .map(|cfg| cfg.plasticity.is_active())
        .unwrap_or(false);
    let cross_locus_preds: Vec<CrossLocusPred> = cross_locus_pairs
        .into_iter()
        .map(|(from_locus, pre_signal, pred_batch, pred_change_id)| {
            let schema_violation = if let Some(cfg) = kind_cfg
                && !cfg.applies_between.is_empty()
            {
                let from_kind = world.locus(from_locus).map(|l| l.kind);
                let to_kind = world.locus(locus_id).map(|l| l.kind);
                match (from_kind, to_kind) {
                    (Some(fk), Some(tk)) if !cfg.allows_endpoint_kinds(fk, tk) => Some((fk, tk)),
                    _ => None,
                }
            } else {
                None
            };
            let emergence = resolve_emergence(
                world,
                from_locus,
                locus_id,
                kind,
                kind_cfg,
                &resolved_slots,
                pre_signal,
            );
            CrossLocusPred {
                from_locus,
                pre_signal,
                pred_batch,
                pred_change_id,
                is_feedback: false,
                schema_violation,
                emergence,
            }
        })
        .collect();
    ComputedChange::Locus(ComputedLocusChange {
        locus_id,
        kind,
        predecessors,
        before,
        after: stabilized_after,
        wall_time: proposed.wall_time,
        metadata: proposed.metadata,
        property_patch: proposed.property_patch,
        cross_locus_preds,
        resolved_slots,
        plasticity_active,
        post_signal,
    })
}

fn compute_relationship_change(
    rel_id: RelationshipId,
    kind: InfluenceKindId,
    predecessors: Vec<ChangeId>,
    proposed: ProposedChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let (before, from, to) = match world.relationships().get(rel_id) {
        Some(r) => {
            let (f, t) = match r.endpoints {
                Endpoints::Directed { from, to } => (from, to),
                Endpoints::Symmetric { a, b } => (a, b),
            };
            (r.state.clone(), f, t)
        }
        None => return ComputedChange::Elided,
    };
    let raw_after = match proposed.slot_patches {
        Some(patches) => patches.into_iter().fold(before.clone(), |s, (idx, delta)| {
            s.with_slot_delta(idx, delta)
        }),
        None => proposed.after,
    };
    let stabilized_after = match influence_registry.get(kind) {
        Some(cfg) => cfg.stabilization.stabilize(&before, raw_after),
        None => raw_after,
    };
    let has_subscribers = world
        .subscriptions()
        .has_any_subscribers(rel_id, kind, from, to);
    ComputedChange::Relationship(ComputedRelChange {
        rel_id,
        kind,
        predecessors,
        before,
        after: stabilized_after,
        wall_time: proposed.wall_time,
        metadata: proposed.metadata,
        from,
        to,
        has_subscribers,
    })
}
