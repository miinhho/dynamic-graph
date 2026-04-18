use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use graph_world::World;

use super::{Simulation, SimulationConfig, config};
use crate::engine::Engine;
use crate::regime::{AdaptiveGuardRail, BatchHistory, DefaultRegimeClassifier, DynamicsRegime};
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

#[cfg(feature = "storage")]
use graph_storage::Storage;

pub(super) fn with_config(
    world: World,
    loci: LocusKindRegistry,
    influences: InfluenceKindRegistry,
    config: SimulationConfig,
) -> Simulation {
    let guard_rail = build_guard_rail(&influences, &config);
    let prev_batch = world.current_batch();

    #[cfg(feature = "storage")]
    let auto_commit = config.auto_commit;

    #[cfg(feature = "storage")]
    let (storage, initial_error) = init_storage(&config);

    Simulation {
        world: Arc::new(RwLock::new(world)),
        loci,
        base_influences: influences,
        engine: Engine::new(config.engine),
        guard_rail,
        classifier: DefaultRegimeClassifier,
        history: BatchHistory::new(config::HISTORY_WINDOW),
        prev_batch,
        prev_regime: DynamicsRegime::Initializing,
        #[cfg(feature = "storage")]
        storage,
        #[cfg(feature = "storage")]
        last_storage_error: initial_error,
        #[cfg(feature = "storage")]
        last_flushed_batch: prev_batch,
        #[cfg(feature = "storage")]
        auto_commit,
        change_retention_batches: None,
        cold_relationship_threshold: None,
        cold_relationship_min_idle_batches: config::COLD_RELATIONSHIP_MIN_IDLE_BATCHES,
        pending_stimuli: Vec::new(),
        pending_stimuli_capacity: 0,
        backpressure_policy: config::BACKPRESSURE_POLICY,
        tick_count: 0,
        event_history: None,
        auto_weather_every_ticks: None,
        auto_weather_policy: None,
        locus_kind_names: FxHashMap::default(),
        influence_kind_names: FxHashMap::default(),
        default_influence: None,
        triggers: Vec::new(),
        observers: Vec::new(),
        plasticity_learners: None,
    }
}

pub(super) fn build_guard_rail(
    influences: &InfluenceKindRegistry,
    config: &SimulationConfig,
) -> AdaptiveGuardRail {
    let mut guard_rail = AdaptiveGuardRail::new(config.adaptive);
    for kind in influences.kinds() {
        guard_rail.register(kind);
    }
    guard_rail
}

#[cfg(feature = "storage")]
pub(super) fn init_storage(
    config: &SimulationConfig,
) -> (Option<Storage>, Option<graph_storage::StorageError>) {
    match config.storage_path {
        Some(ref path) => match Storage::open_or_reset(path) {
            Ok(storage) => (Some(storage), None),
            Err(error) => (None, Some(error)),
        },
        None => (None, None),
    }
}
