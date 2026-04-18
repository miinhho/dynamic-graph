use graph_core::{BatchId, EntityId};
use graph_world::World;

use super::{
    DecayRates, EmergenceReport, EmergenceSynergyReport, LeaveOneOutResult, PsiResult,
    PsiSynergyResult, coherence_dense_series_inner, pearson_autocorr, report,
};

pub fn coherence_stable_series(world: &World, entity_id: EntityId) -> Vec<(BatchId, f32)> {
    let entity = match world.entities().get(entity_id) {
        Some(entity) => entity,
        None => return Vec::new(),
    };

    let start_idx = entity
        .layers
        .iter()
        .rposition(super::is_lifecycle_transition)
        .map(|index| index + 1)
        .unwrap_or(0);

    entity.layers[start_idx..]
        .iter()
        .filter_map(|layer| super::layer_coherence(layer).map(|coherence| (layer.batch, coherence)))
        .collect()
}

pub fn coherence_dense_series(world: &World, entity_id: EntityId) -> Vec<(BatchId, f32)> {
    coherence_dense_series_inner(world, entity_id, None)
}

pub fn coherence_dense_series_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Vec<(BatchId, f32)> {
    coherence_dense_series_inner(world, entity_id, Some(decay_rates))
}

pub fn coherence_autocorrelation(world: &World, entity_id: EntityId, lag: usize) -> Option<f64> {
    let series: Vec<f32> = coherence_stable_series(world, entity_id)
        .into_iter()
        .map(|(_, coherence)| coherence)
        .collect();
    pearson_autocorr(&series, lag)
}

pub fn psi_scalar(world: &World, entity_id: EntityId) -> Option<PsiResult> {
    super::psi::psi_scalar_inner(world, entity_id, None)
}

pub fn psi_scalar_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Option<PsiResult> {
    super::psi::psi_scalar_inner(world, entity_id, Some(decay_rates))
}

pub fn psi_synergy(world: &World, entity_id: EntityId) -> Option<PsiSynergyResult> {
    super::psi::psi_synergy_inner(world, entity_id, None)
}

pub fn psi_synergy_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Option<PsiSynergyResult> {
    super::psi::psi_synergy_inner(world, entity_id, Some(decay_rates))
}

pub fn psi_synergy_leave_one_out(world: &World, entity_id: EntityId) -> Option<LeaveOneOutResult> {
    super::leave_one_out::psi_synergy_leave_one_out_inner(world, entity_id, None)
}

pub fn psi_synergy_leave_one_out_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Option<LeaveOneOutResult> {
    super::leave_one_out::psi_synergy_leave_one_out_inner(world, entity_id, Some(decay_rates))
}

pub fn emergence_report(world: &World) -> EmergenceReport {
    report::emergence_report_inner(world, None)
}

pub fn emergence_report_with_decay(world: &World, decay_rates: &DecayRates) -> EmergenceReport {
    report::emergence_report_inner(world, Some(decay_rates))
}

pub fn emergence_report_synergy(world: &World) -> EmergenceSynergyReport {
    report::emergence_report_synergy_inner(world, None)
}

pub fn emergence_report_synergy_with_decay(
    world: &World,
    decay_rates: &DecayRates,
) -> EmergenceSynergyReport {
    report::emergence_report_synergy_inner(world, Some(decay_rates))
}
