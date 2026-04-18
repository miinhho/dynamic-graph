use rustc_hash::FxHashMap;
use std::sync::{Arc, RwLock};

use graph_core::{BatchId, ProposedChange};
use graph_world::World;

use crate::engine::Engine;
use crate::plasticity::PlasticityLearners;
use crate::regime::{AdaptiveGuardRail, BatchHistory, DefaultRegimeClassifier, DynamicsRegime};
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};
use crate::simulation::{config, observability::EventHistory, watch};

#[cfg(feature = "storage")]
use graph_storage::Storage;

/// Bundles the world, registries, engine, regime classifier, and
/// adaptive guard rail into a single step-by-step interface.
pub struct Simulation {
    pub(crate) world: Arc<RwLock<World>>,
    pub(crate) loci: LocusKindRegistry,
    pub(crate) base_influences: InfluenceKindRegistry,
    pub(crate) engine: Engine,
    pub(crate) guard_rail: AdaptiveGuardRail,
    pub(crate) classifier: DefaultRegimeClassifier,
    pub(crate) history: BatchHistory,
    pub(crate) prev_batch: BatchId,
    pub(crate) prev_regime: DynamicsRegime,
    #[cfg(feature = "storage")]
    pub(crate) storage: Option<Storage>,
    #[cfg(feature = "storage")]
    pub(crate) last_storage_error: Option<graph_storage::StorageError>,
    #[cfg(feature = "storage")]
    pub(crate) last_flushed_batch: BatchId,
    #[cfg(feature = "storage")]
    pub(crate) auto_commit: bool,
    pub(crate) change_retention_batches: Option<u64>,
    pub(crate) cold_relationship_threshold: Option<f32>,
    pub(crate) cold_relationship_min_idle_batches: u64,
    pub(crate) pending_stimuli: Vec<ProposedChange>,
    pub(crate) pending_stimuli_capacity: usize,
    pub(crate) backpressure_policy: config::BackpressurePolicy,
    pub(crate) tick_count: u64,
    pub(crate) event_history: Option<EventHistory>,
    pub(crate) auto_weather_every_ticks: Option<u32>,
    pub(crate) auto_weather_policy: Option<Box<dyn graph_core::EntityWeatheringPolicy>>,
    pub(crate) locus_kind_names: FxHashMap<String, graph_core::LocusKindId>,
    pub(crate) influence_kind_names: FxHashMap<String, graph_core::InfluenceKindId>,
    pub(crate) default_influence: Option<graph_core::InfluenceKindId>,
    pub(crate) triggers: Vec<watch::TriggerEntry>,
    pub(crate) observers: Vec<watch::ObserverEntry>,
    pub(crate) plasticity_learners: Option<PlasticityLearners>,
}
