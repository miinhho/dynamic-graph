use graph_core::EntityId;
use graph_world::World;

use super::{DecayRates, LeaveOneOutResult, coherence_dense_series_inner, psi, synergy};

pub(super) fn psi_synergy_leave_one_out_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Option<LeaveOneOutResult> {
    let baseline = psi::psi_synergy_inner(world, entity_id, decay_rates)?;
    let entity = world.entities().get(entity_id)?;
    let window = coherence_dense_series_inner(world, entity_id, decay_rates);
    let v_t1: Vec<f64> = window[1..]
        .iter()
        .map(|(_, coherence)| *coherence as f64)
        .collect();
    let components = synergy::build_component_series(
        world,
        &entity.current.member_relationships,
        &window,
        &v_t1,
    );
    if components.x_series.len() < 3 {
        return None;
    }

    let drops = synergy::leave_one_out_drops(
        &components.x_series,
        &components.rel_ids,
        &components.individual_mi,
        &v_t1,
        &baseline,
    );

    Some(LeaveOneOutResult {
        entity: entity_id,
        baseline,
        drops,
    })
}
