use graph_core::{BatchId, InfluenceKindId, ProposedChange};
use graph_world::World;

use crate::engine::{self, Engine};
use crate::regime::{BatchMetrics, RegimeClassifier};
use crate::registry::InfluenceKindRegistry;

use super::{Simulation, observability::TickSummary};

pub(super) struct TickStage {
    pub(super) tick: engine::TickResult,
    pub(super) batch: BatchId,
}

pub(super) struct MetricsStage {
    pub(super) tick: engine::TickResult,
    pub(super) batch: BatchId,
    pub(super) metrics: BatchMetrics,
}

pub(super) struct RecordedStage {
    pub(super) tick: engine::TickResult,
    pub(super) batch: BatchId,
}

pub(super) struct StepExecution {
    pub(super) tick: engine::TickResult,
    pub(super) relationships: usize,
    pub(super) active_entities: usize,
    pub(super) summary: TickSummary,
}

pub(super) struct LifecycleMaintenanceConfig<'a> {
    pub(super) engine: &'a Engine,
    pub(super) base_influences: &'a InfluenceKindRegistry,
    pub(super) change_retention_batches: Option<u64>,
    pub(super) cold_relationship_threshold: Option<f32>,
    pub(super) cold_relationship_min_idle_batches: u64,
    pub(super) auto_weather_every_ticks: Option<u32>,
    pub(super) auto_weather_policy: Option<&'a dyn graph_core::EntityWeatheringPolicy>,
}

pub(super) fn apply_lifecycle_maintenance(
    world: &mut World,
    tick_count: &mut u64,
    current_batch: BatchId,
    config: LifecycleMaintenanceConfig<'_>,
) {
    if let Some(retention) = config.change_retention_batches
        && current_batch.0 > retention
    {
        let cutoff = BatchId(current_batch.0 - retention);
        config.engine.trim_change_log_to(world, cutoff);
        world.subscriptions_mut().trim_audit_before(cutoff);
        world.trim_pruned_log_before(cutoff);
    }

    if let Some(threshold) = config.cold_relationship_threshold {
        world.evict_cold_relationships(
            threshold,
            config.cold_relationship_min_idle_batches,
            current_batch,
        );
    }

    crate::engine::world_ops::apply_demotion_policies(world, config.base_influences, current_batch);

    *tick_count += 1;
    if let Some(interval) = config.auto_weather_every_ticks
        && tick_count.is_multiple_of(interval as u64)
        && let Some(policy) = config.auto_weather_policy
    {
        config.engine.weather_entities(world, policy);
    }
}

impl Simulation {
    pub(super) fn run_step_world_mutation(
        &mut self,
        stimuli: Vec<ProposedChange>,
        effective: &mut InfluenceKindRegistry,
        kinds: &[InfluenceKindId],
        prev_batch: BatchId,
    ) -> StepExecution {
        let world_handle = self.world_handle();
        let mut world = world_handle.write().unwrap();
        let tick_stage = self.execute_step_tick(&mut world, effective, stimuli);
        let metrics_stage = self.compute_step_metrics(&world, prev_batch, tick_stage);
        let recorded_stage = self.record_step_metrics(metrics_stage, kinds);

        #[cfg(feature = "storage")]
        self.persist_step_batches(&world, recorded_stage.batch);

        apply_lifecycle_maintenance(
            &mut world,
            &mut self.tick_count,
            recorded_stage.batch,
            LifecycleMaintenanceConfig {
                engine: &self.engine,
                base_influences: &self.base_influences,
                change_retention_batches: self.change_retention_batches,
                cold_relationship_threshold: self.cold_relationship_threshold,
                cold_relationship_min_idle_batches: self.cold_relationship_min_idle_batches,
                auto_weather_every_ticks: self.auto_weather_every_ticks,
                auto_weather_policy: self.auto_weather_policy.as_deref(),
            },
        );

        self.finalize_step_execution(&world, prev_batch, recorded_stage)
    }

    fn execute_step_tick(
        &self,
        world: &mut World,
        effective: &mut InfluenceKindRegistry,
        stimuli: Vec<ProposedChange>,
    ) -> TickStage {
        let tick = self.engine.tick(world, &self.loci, effective, stimuli);
        TickStage {
            tick,
            batch: world.current_batch(),
        }
    }

    fn compute_step_metrics(
        &self,
        world: &World,
        prev_batch: BatchId,
        tick_stage: TickStage,
    ) -> MetricsStage {
        let changes =
            (prev_batch.0 + 1..=tick_stage.batch.0).flat_map(|b| world.log().batch(BatchId(b)));
        MetricsStage {
            tick: tick_stage.tick,
            batch: tick_stage.batch,
            metrics: BatchMetrics::from_changes(changes),
        }
    }

    fn record_step_metrics(
        &mut self,
        metrics_stage: MetricsStage,
        kinds: &[InfluenceKindId],
    ) -> RecordedStage {
        self.prev_batch = metrics_stage.batch;
        self.history.push(metrics_stage.metrics);
        let regime = self.classifier.classify(&self.history);
        for &kind in kinds {
            self.guard_rail.observe(kind, regime);
        }
        RecordedStage {
            tick: metrics_stage.tick,
            batch: metrics_stage.batch,
        }
    }

    fn finalize_step_execution(
        &self,
        world: &World,
        prev_batch: BatchId,
        recorded_stage: RecordedStage,
    ) -> StepExecution {
        let summary = TickSummary::compute(
            self.tick_count,
            &recorded_stage.tick,
            prev_batch,
            recorded_stage.batch,
            world,
            &[],
        );
        StepExecution {
            tick: recorded_stage.tick,
            relationships: world.relationships().len(),
            active_entities: world.entities().active_count(),
            summary,
        }
    }
}
