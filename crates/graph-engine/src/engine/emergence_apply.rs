use graph_core::{
    BatchId, ChangeId, InfluenceKindId, KindObservation, Relationship, RelationshipId,
    RelationshipLineage, StateVector,
};
use graph_world::RelationshipStore;

use super::batch::EmergenceResolution;

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
    match resolution {
        EmergenceResolution::Update { rel_id } => {
            let activity_decay = kind_cfg.map_or(1.0, |c| c.decay_per_batch);
            let weight_decay = kind_cfg.map_or(1.0, |c| c.plasticity.weight_decay);
            let activity_contribution = kind_cfg.map_or(1.0, |c| c.activity_contribution);
            let abs_signal = pre_signal.abs();

            if let Some(rel) = store.get_mut(rel_id) {
                let delta = batch.0.saturating_sub(rel.last_decayed_batch);
                if delta > 0 {
                    let act_factor = if delta == 1 {
                        activity_decay
                    } else {
                        activity_decay.powi(delta as i32)
                    };
                    let wt_factor = if delta == 1 {
                        weight_decay
                    } else {
                        weight_decay.powi(delta as i32)
                    };
                    let slots = rel.state.as_mut_slice();
                    if let Some(a) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
                        *a = *a * act_factor + activity_contribution * abs_signal;
                    }
                    if let Some(w) = slots.get_mut(Relationship::WEIGHT_SLOT) {
                        *w *= wt_factor;
                    }
                    for (i, slot_def) in resolved_slots.iter().enumerate() {
                        if let Some(factor) = slot_def.decay
                            && let Some(v) = slots.get_mut(2 + i)
                        {
                            *v *= if delta == 1 {
                                factor
                            } else {
                                factor.powi(delta as i32)
                            };
                        }
                    }
                    rel.last_decayed_batch = batch.0;
                } else if let Some(a) = rel
                    .state
                    .as_mut_slice()
                    .get_mut(Relationship::ACTIVITY_SLOT)
                {
                    *a += activity_contribution * abs_signal;
                }
                rel.lineage.last_touched_by = Some(change_id);
                rel.lineage.change_count += 1;
                rel.lineage.observe_kind(kind, batch);
            }
            Some((rel_id, false, None))
        }

        EmergenceResolution::Create {
            endpoints,
            kind: rel_kind,
            initial_state,
            pre_signal: create_pre_signal,
            activity_contribution,
        } => {
            let key = endpoints.key();
            if let Some(existing_id) = store.lookup(&key, rel_kind) {
                let rel = store.get_mut(existing_id).expect("indexed id must exist");
                if let Some(slot) = rel
                    .state
                    .as_mut_slice()
                    .get_mut(Relationship::ACTIVITY_SLOT)
                {
                    *slot += activity_contribution * create_pre_signal.abs();
                }
                rel.lineage.last_touched_by = Some(change_id);
                rel.lineage.change_count += 1;
                rel.lineage.observe_kind(rel_kind, batch);
                Some((existing_id, false, None))
            } else {
                let new_id = store.mint_id();
                store.insert(Relationship {
                    id: new_id,
                    kind: rel_kind,
                    endpoints,
                    state: initial_state.clone(),
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
                Some((new_id, true, Some(initial_state)))
            }
        }
    }
}
