mod adaptive;
mod client;
mod coordinator;
mod diagnostics;
mod dynamics;
mod engine;
mod inspection;
mod laws;
mod regime;
mod scheduled;
mod scheduler;
mod stabilizer;
mod trace;

pub use adaptive::{AdaptiveConfig, AdaptiveStabilizer};
pub use client::{ClientDriver, InspectableResult, RuntimeClient, TickRead, WorldRead};
pub use coordinator::{
    CommitPolicy, IdentityStimulusAdapter, RetryPolicy, RuntimeCoordinator, RuntimeTick,
    StimulusAdapter, TickDriver,
};
pub use diagnostics::TickDiagnostics;
pub use dynamics::{ProgramCatalog, ProgramRegistry};
pub use engine::{Engine, EngineConfig, RoutingPolicy, RoutingStrategy, TickResult};
pub use inspection::TickInspection;
pub use laws::{LawCatalog, LawRegistry};
pub use regime::{
    DefaultRegimeClassifier, DynamicsRegime, MetricsHistory, RegimeClassifier, TickMetrics,
};
pub use scheduled::{ScheduleConfig, ScheduledDriver, ScheduledTickResult};
pub use scheduler::{compute_scc_plan, SccPlan};
pub use stabilizer::{
    BasicStabilizer, PolicyStabilizer, SaturationMode, StabilizationPolicy, Stabilizer,
};
pub use trace::{NoopTraceSink, TraceEvent, TraceSink};

#[doc(hidden)]
pub mod __bench {
    use rustc_hash::FxHashMap;

    use crate::{EngineConfig, LawCatalog, ProgramCatalog, RoutingStrategy, Stabilizer};

    pub use crate::engine::aggregation::aggregate_cohort_emissions;
    pub use crate::engine::provenance::from_emissions;

    fn prepare_sources_internal(
        world: graph_world::WorldSnapshot<'_>,
        programs: &impl ProgramCatalog,
        activation_threshold: f32,
    ) -> Vec<crate::engine::source::PreparedSource> {
        crate::engine::source::prepare_sources(world, programs, activation_threshold)
    }

    pub fn dispatch_source_emission_count<S, R>(
        config: EngineConfig,
        stabilizer: &S,
        source_entity_id: graph_core::EntityId,
        world: graph_world::WorldSnapshot<'_>,
        laws: &impl LawCatalog,
        routing: &R,
        programs: &impl ProgramCatalog,
    ) -> usize
    where
        S: Stabilizer,
        R: RoutingStrategy,
    {
        let prepared = prepare_sources_internal(world, programs, config.activation_threshold);
        let Some(prepared_source) = prepared
            .iter()
            .find(|prepared| prepared.entity_id == source_entity_id)
        else {
            return 0;
        };

        crate::engine::source::dispatch_source(
            config,
            stabilizer,
            prepared_source,
            world,
            laws,
            routing,
        )
        .total_emissions
    }

    pub fn plan_channel_dispatch_target_count<R>(
        world: graph_world::WorldSnapshot<'_>,
        channel_id: graph_core::ChannelId,
        strategy: &R,
        remaining_targets: usize,
    ) -> usize
    where
        R: RoutingStrategy,
    {
        let Some(channel) = world.channel(channel_id) else {
            return 0;
        };
        let mut selector_cache = crate::engine::routing::SelectorCache::default();
        match crate::engine::routing::plan_channel_dispatch(
            world,
            channel,
            strategy,
            remaining_targets,
            &mut selector_cache,
        ) {
            crate::engine::routing::DispatchPlan::Direct { targets, .. } => targets.len(),
            crate::engine::routing::DispatchPlan::Cohort { kinds, .. } => kinds.len(),
        }
    }

    pub fn merge_stimuli(
        stimuli: &[graph_core::Stimulus],
    ) -> FxHashMap<graph_core::EntityId, graph_core::Stimulus> {
        crate::engine::state::merge_stimuli(stimuli)
    }

    pub fn compute_state_update_present<S>(
        entity_id: graph_core::EntityId,
        world: graph_world::WorldSnapshot<'_>,
        programs: &impl ProgramCatalog,
        inbox: &FxHashMap<graph_core::EntityId, smallvec::SmallVec<[graph_core::Emission; 4]>>,
        cohort_inbox: &FxHashMap<
            graph_core::EntityKindId,
            smallvec::SmallVec<[graph_core::Emission; 4]>,
        >,
        stimuli: &FxHashMap<graph_core::EntityId, graph_core::Stimulus>,
        stabilizer: &S,
    ) -> bool
    where
        S: Stabilizer,
    {
        crate::engine::state::compute_state_update(
            entity_id,
            world,
            programs,
            inbox,
            cohort_inbox,
            stimuli,
            stabilizer,
        )
        .is_some()
    }
}
