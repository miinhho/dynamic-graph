use graph_core::{EntityId, EntityKindId, Stimulus};
use graph_tx::DeltaProvenance;
use graph_world::{World, WorldSnapshot};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ProgramCatalog, Stabilizer};

use super::aggregation::aggregate_cohort_emissions;
use super::{EmissionBuffer, dispatch, provenance};

pub(crate) struct StateUpdate {
    pub entity_id: EntityId,
    pub before: graph_core::EntityState,
    pub after: graph_core::EntityState,
    pub provenance: DeltaProvenance,
}

pub(super) fn decay_cooldowns(world: &mut World) {
    let ids = world.entities().map(|entity| entity.id).collect::<Vec<_>>();
    for id in ids {
        if let Some(entity) = world.entity_mut(id) {
            if entity.state.cooldown > 0 {
                entity.state.cooldown -= 1;
            }
            if entity.state.emitted.l2_norm() > 0.0 && entity.refractory_period > 0 {
                entity.state.cooldown = entity.refractory_period;
            }
        }
    }
}

pub(crate) fn merge_stimuli(stimuli: &[Stimulus]) -> FxHashMap<EntityId, Stimulus> {
    let mut merged = FxHashMap::default();
    for stimulus in stimuli {
        merged
            .entry(stimulus.target)
            .and_modify(|current: &mut Stimulus| {
                current.signal = current.signal.add(&stimulus.signal);
            })
            .or_insert_with(|| stimulus.clone());
    }
    merged
}

pub(crate) fn compute_state_update<S>(
    entity_id: EntityId,
    world: WorldSnapshot<'_>,
    programs: &impl ProgramCatalog,
    inbox: &FxHashMap<EntityId, EmissionBuffer>,
    cohort_inbox: &FxHashMap<EntityKindId, EmissionBuffer>,
    stimuli: &FxHashMap<EntityId, Stimulus>,
    stabilizer: &S,
) -> Option<StateUpdate>
where
    S: Stabilizer,
{
    let entity = world.entity(entity_id).cloned()?;
    let program = programs.get(entity.kind)?;

    let mut entity_emissions = inbox.get(&entity_id).cloned().unwrap_or_default();
    if let Some(emissions) = cohort_inbox.get(&entity.kind) {
        entity_emissions.extend(
            aggregate_cohort_emissions(world, emissions)
                .into_iter()
                .map(|emission| dispatch::retarget_emission(emission, entity_id)),
        );
    }

    let inbox_signal = dispatch::sum_emissions(&entity_emissions);
    let stimulus = stimuli.get(&entity_id);
    let raw_state = program.next_state(&entity.state, &inbox_signal, stimulus);
    let next_state = stabilizer.stabilize_state(&entity, raw_state);
    let provenance =
        provenance::from_emissions(entity_id, &entity_emissions, stabilizer.relaxation_alpha());

    Some(StateUpdate {
        entity_id,
        before: entity.state,
        after: next_state,
        provenance,
    })
}

pub(crate) fn collect_affected_entities(
    snapshot: WorldSnapshot<'_>,
    inbox: &FxHashMap<EntityId, EmissionBuffer>,
    cohort_inbox: &FxHashMap<EntityKindId, EmissionBuffer>,
    stimuli: &FxHashMap<EntityId, Stimulus>,
) -> Vec<EntityId> {
    let mut affected = FxHashSet::with_capacity_and_hasher(
        inbox.len() + stimuli.len() + cohort_inbox.len(),
        Default::default(),
    );
    affected.extend(inbox.keys().copied());
    affected.extend(stimuli.keys().copied());
    for kind in cohort_inbox.keys() {
        affected.extend(snapshot.entity_ids_of_kind(*kind).iter().copied());
    }

    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable_by_key(|entity_id| entity_id.0);
    affected
}
