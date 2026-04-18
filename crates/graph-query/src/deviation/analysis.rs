use graph_core::{BatchId, Entity};

use super::{EntityDiff, baseline::baseline_state, observed::observe_layers_since_baseline};

pub(super) fn diff_entity(entity: &Entity, baseline: BatchId) -> EntityDiff {
    let baseline_state = baseline_state(entity, baseline);
    let observed = observe_layers_since_baseline(entity, baseline);
    let coherence_now = entity.current.coherence;
    let member_count_now = entity.current.members.len() as i64;

    EntityDiff {
        entity_id: entity.id,
        born_after_baseline: observed.born_after_baseline,
        went_dormant: observed.went_dormant,
        revived: observed.revived,
        members_added: observed.members_added,
        members_removed: observed.members_removed,
        membership_event_count: observed.membership_event_count,
        coherence_at_baseline: baseline_state.coherence,
        coherence_now,
        coherence_delta: coherence_now - baseline_state.coherence,
        member_count_delta: member_count_now - baseline_state.member_count,
        latest_change_batch: observed.latest_change_batch,
    }
}
