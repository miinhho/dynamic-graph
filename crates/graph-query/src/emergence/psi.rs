use graph_core::EntityId;
use graph_world::World;

use super::{
    DecayRates, PsiResult, PsiSynergyResult, coherence_dense_series_inner, gaussian_joint_mi,
    gaussian_mi_from_series, synergy,
};

pub(super) fn psi_scalar_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Option<PsiResult> {
    let entity = world.entities().get(entity_id)?;
    let prepared = PreparedWindow::build(world, entity_id, decay_rates)?;
    let components =
        collect_scalar_components(world, &entity.current.member_relationships, &prepared);
    assemble_scalar_result(prepared, components)
}

pub(super) fn psi_synergy_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Option<PsiSynergyResult> {
    let entity = world.entities().get(entity_id)?;
    let prepared = PreparedWindow::build(world, entity_id, decay_rates)?;
    let components = synergy::build_component_series(
        world,
        &entity.current.member_relationships,
        &prepared.window,
        &prepared.v_t1,
    );
    assemble_synergy_result(prepared, components)
}

struct PreparedWindow {
    window: Vec<(graph_core::BatchId, f32)>,
    v_t1: Vec<f64>,
    i_self: f64,
    n_pairs: usize,
}

struct ScalarComponents {
    individual_mi: Vec<f64>,
}

impl PreparedWindow {
    fn build(world: &World, entity_id: EntityId, decay_rates: Option<&DecayRates>) -> Option<Self> {
        let window = coherence_dense_series_inner(world, entity_id, decay_rates);
        let n = window.len();
        if n < 3 {
            return None;
        }

        let v_t: Vec<f64> = window[..n - 1]
            .iter()
            .map(|(_, coherence)| *coherence as f64)
            .collect();
        let v_t1: Vec<f64> = window[1..]
            .iter()
            .map(|(_, coherence)| *coherence as f64)
            .collect();
        let i_self = gaussian_mi_from_series(&v_t, &v_t1)?;

        Some(Self {
            window,
            v_t1,
            i_self,
            n_pairs: n - 1,
        })
    }
}

fn collect_scalar_components(
    world: &World,
    member_relationships: &[graph_core::RelationshipId],
    prepared: &PreparedWindow,
) -> ScalarComponents {
    let individual_mi = member_relationships
        .iter()
        .filter_map(|relationship_id| {
            let component_series = component_series(world, *relationship_id, prepared);
            gaussian_mi_from_series(&component_series, &prepared.v_t1)
        })
        .collect();

    ScalarComponents { individual_mi }
}

fn component_series(
    world: &World,
    relationship_id: graph_core::RelationshipId,
    prepared: &PreparedWindow,
) -> Vec<f64> {
    prepared
        .window
        .iter()
        .take(prepared.n_pairs)
        .map(|(batch, _)| super::rel_weight_at(*batch, relationship_id, world))
        .collect()
}

fn assemble_scalar_result(
    prepared: PreparedWindow,
    components: ScalarComponents,
) -> Option<PsiResult> {
    let n_components = components.individual_mi.len();
    if n_components == 0 {
        return None;
    }

    let component_sum: f64 = components.individual_mi.iter().sum();
    Some(PsiResult {
        psi: prepared.i_self - component_sum,
        i_self: prepared.i_self,
        i_sum_components: component_sum,
        n_samples: prepared.n_pairs,
        n_components,
    })
}

fn assemble_synergy_result(
    prepared: PreparedWindow,
    components: synergy::ComponentSeries,
) -> Option<PsiSynergyResult> {
    let n_components = components.x_series.len();
    if n_components < 2 || prepared.n_pairs < n_components + 2 {
        return None;
    }

    let i_sum: f64 = components.individual_mi.iter().sum();
    let i_joint = gaussian_joint_mi(&components.x_series, &prepared.v_t1)?;
    let pair_summary = synergy::summarize_pairs(
        &components.x_series,
        &components.rel_ids,
        &components.individual_mi,
        &prepared.v_t1,
    );

    Some(PsiSynergyResult {
        i_self: prepared.i_self,
        i_sum_components: i_sum,
        i_joint_components: i_joint,
        psi_naive: prepared.i_self - i_sum,
        psi_corrected: prepared.i_self - i_joint,
        top_pairs: pair_summary.top_pairs,
        n_samples: prepared.n_pairs,
        n_components,
        n_pairs_evaluated: pair_summary.n_pairs_evaluated,
        total_pair_synergy: pair_summary.total_pair_synergy,
        total_pair_redundancy: pair_summary.total_pair_redundancy,
        mean_pair_synergy: pair_summary.mean_pair_synergy,
        psi_pair_top3: prepared.i_self - pair_summary.top3_joint_sum,
    })
}
