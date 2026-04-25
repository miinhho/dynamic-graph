use graph_core::{
    BatchId, ChangeId, InfluenceKindId, KindObservation, Relationship, RelationshipId,
    RelationshipLineage, StateVector,
};
use graph_world::RelationshipStore;

use super::batch::{signed_activity_contribution, EmergenceResolution};

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_emergence(
    store: &mut RelationshipStore,
    resolution: EmergenceResolution,
    change_id: ChangeId,
    batch: BatchId,
    kind: InfluenceKindId,
    pre_signal: f32,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) -> Option<(RelationshipId, bool, Option<StateVector>)> {
    let update_context = EmergenceUpdateContext {
        change_id,
        batch,
        kind,
        pre_signal,
        kind_cfg,
        resolved_slots,
    };

    match resolution {
        EmergenceResolution::Update { rel_id } => {
            apply_emergence_update(store, rel_id, update_context)
        }
        EmergenceResolution::Create {
            endpoints,
            kind: rel_kind,
            initial_state,
            pre_signal: create_pre_signal,
            activity_contribution,
        } => apply_emergence_create(
            store,
            EmergenceCreateContext {
                change_id,
                batch,
                endpoints,
                rel_kind,
                initial_state,
                create_pre_signal,
                activity_contribution,
            },
        ),
    }
}

#[derive(Clone, Copy)]
struct EmergenceUpdateContext<'a> {
    change_id: ChangeId,
    batch: BatchId,
    kind: InfluenceKindId,
    pre_signal: f32,
    kind_cfg: Option<&'a crate::registry::InfluenceKindConfig>,
    resolved_slots: &'a [graph_core::RelationshipSlotDef],
}

struct EmergenceCreateContext {
    change_id: ChangeId,
    batch: BatchId,
    endpoints: graph_core::Endpoints,
    rel_kind: InfluenceKindId,
    initial_state: StateVector,
    create_pre_signal: f32,
    activity_contribution: f32,
}

fn apply_emergence_update(
    store: &mut RelationshipStore,
    rel_id: RelationshipId,
    context: EmergenceUpdateContext<'_>,
) -> Option<(RelationshipId, bool, Option<StateVector>)> {
    let decay = emergence_decay(context.kind_cfg, context.pre_signal);
    if let Some(rel) = store.get_mut(rel_id) {
        apply_emergence_update_slots(rel, context.batch, decay, context.resolved_slots);
        touch_emergent_relationship(rel, context.change_id, context.kind, context.batch);
    }
    Some((rel_id, false, None))
}

fn apply_emergence_create(
    store: &mut RelationshipStore,
    context: EmergenceCreateContext,
) -> Option<(RelationshipId, bool, Option<StateVector>)> {
    let key = context.endpoints.key();
    if let Some(existing_id) = store.lookup(&key, context.rel_kind) {
        apply_existing_emergent_relationship(
            store,
            existing_id,
            context.change_id,
            context.batch,
            context.rel_kind,
            context.create_pre_signal,
            context.activity_contribution,
        );
        Some((existing_id, false, None))
    } else {
        let new_id = insert_emergent_relationship(
            store,
            context.change_id,
            context.batch,
            context.endpoints,
            context.rel_kind,
            context.initial_state.clone(),
        );
        Some((new_id, true, Some(context.initial_state)))
    }
}

#[derive(Clone, Copy)]
struct EmergenceDecay {
    activity_decay: f32,
    weight_decay: f32,
    /// Pre-computed activity contribution (`activity_contribution × |pre_signal|`).
    /// Centralised through `signed_activity_contribution` so Phase 1
    /// of the trigger-axis roadmap has one place to revisit signed semantics.
    activity_input: f32,
}

fn emergence_decay(
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    pre_signal: f32,
) -> EmergenceDecay {
    let activity_contribution = kind_cfg.map_or(1.0, |cfg| cfg.activity_contribution);
    EmergenceDecay {
        activity_decay: kind_cfg.map_or(1.0, |cfg| cfg.decay_per_batch),
        weight_decay: kind_cfg.map_or(1.0, |cfg| cfg.plasticity.weight_decay),
        activity_input: signed_activity_contribution(activity_contribution, pre_signal),
    }
}

fn apply_emergence_update_slots(
    rel: &mut Relationship,
    batch: BatchId,
    decay: EmergenceDecay,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) {
    let delta = batch.0.saturating_sub(rel.last_decayed_batch);
    if delta > 0 {
        apply_decay_step(rel, delta, decay, resolved_slots);
        rel.last_decayed_batch = batch.0;
    } else if let Some(activity) = rel
        .state
        .as_mut_slice()
        .get_mut(Relationship::ACTIVITY_SLOT)
    {
        *activity += decay.activity_input;
    }
}

fn apply_decay_step(
    rel: &mut Relationship,
    delta: u64,
    decay: EmergenceDecay,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) {
    let activity_factor = pow_decay(decay.activity_decay, delta);
    let weight_factor = pow_decay(decay.weight_decay, delta);
    let slots = rel.state.as_mut_slice();
    if let Some(activity) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
        *activity = *activity * activity_factor + decay.activity_input;
    }
    if let Some(weight) = slots.get_mut(Relationship::WEIGHT_SLOT) {
        *weight *= weight_factor;
    }
    apply_resolved_slot_decay(slots, delta, resolved_slots);
}

fn apply_resolved_slot_decay(
    slots: &mut [f32],
    delta: u64,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) {
    for (index, slot_def) in resolved_slots.iter().enumerate() {
        if let Some(factor) = slot_def.decay
            && let Some(value) = slots.get_mut(2 + index)
        {
            *value *= pow_decay(factor, delta);
        }
    }
}

fn pow_decay(factor: f32, delta: u64) -> f32 {
    if delta == 1 {
        factor
    } else {
        factor.powi(delta as i32)
    }
}

fn touch_emergent_relationship(
    rel: &mut Relationship,
    change_id: ChangeId,
    kind: InfluenceKindId,
    batch: BatchId,
) {
    rel.lineage.last_touched_by = Some(change_id);
    rel.lineage.change_count += 1;
    rel.lineage.observe_kind(kind, batch);
}

fn apply_existing_emergent_relationship(
    store: &mut RelationshipStore,
    rel_id: RelationshipId,
    change_id: ChangeId,
    batch: BatchId,
    rel_kind: InfluenceKindId,
    create_pre_signal: f32,
    activity_contribution: f32,
) {
    let rel = store.get_mut(rel_id).expect("indexed id must exist");
    if let Some(activity) = rel
        .state
        .as_mut_slice()
        .get_mut(Relationship::ACTIVITY_SLOT)
    {
        *activity += signed_activity_contribution(activity_contribution, create_pre_signal);
    }
    touch_emergent_relationship(rel, change_id, rel_kind, batch);
}

fn insert_emergent_relationship(
    store: &mut RelationshipStore,
    change_id: ChangeId,
    batch: BatchId,
    endpoints: graph_core::Endpoints,
    rel_kind: InfluenceKindId,
    initial_state: StateVector,
) -> RelationshipId {
    let new_id = store.mint_id();
    store.insert(Relationship {
        id: new_id,
        kind: rel_kind,
        endpoints,
        state: initial_state,
        lineage: RelationshipLineage {
            created_by: Some(change_id),
            last_touched_by: Some(change_id),
            change_count: 1,
            kinds_observed: smallvec::smallvec![KindObservation::once(rel_kind, batch)],
        },
        created_batch: batch,
        last_decayed_batch: batch.0,
        metadata: None,
    });
    new_id
}
