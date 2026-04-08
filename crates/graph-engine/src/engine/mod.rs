pub(crate) mod aggregation;
pub(crate) mod dispatch;
pub(crate) mod provenance;
pub(crate) mod routing;
pub(crate) mod source;
pub(crate) mod state;

use graph_core::{Emission, EntityId, Stimulus, TickId};
use graph_tx::TickTransaction;
use graph_world::World;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::{
    LawCatalog, NoopTraceSink, ProgramCatalog, Stabilizer, TickDiagnostics, TraceEvent, TraceSink,
};
pub use routing::{RoutingPolicy, RoutingStrategy};
use source::{dispatch_source, prepare_sources};
use state::{collect_affected_entities, compute_state_update, decay_cooldowns, merge_stimuli};

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub activation_threshold: f32,
    pub routing_policy: RoutingPolicy,
}

#[derive(Debug, Clone)]
pub struct TickResult {
    pub diagnostics: TickDiagnostics,
    pub transaction: TickTransaction,
}

pub struct Engine<S> {
    config: EngineConfig,
    stabilizer: S,
}

type EmissionBuffer = SmallVec<[Emission; 4]>;

struct RuntimeContext<'a, R, T> {
    routing: &'a R,
    trace_sink: &'a mut T,
}

impl<S> Engine<S>
where
    S: Stabilizer,
{
    pub fn new(config: EngineConfig, stabilizer: S) -> Self {
        Self { config, stabilizer }
    }

    pub fn stabilizer(&self) -> &S {
        &self.stabilizer
    }

    pub fn tick(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &impl ProgramCatalog,
        laws: &impl LawCatalog,
        stimuli: &[Stimulus],
    ) -> TickResult {
        let mut sink = NoopTraceSink;
        self.execute_tick(
            tick,
            world,
            programs,
            laws,
            stimuli,
            RuntimeContext {
                routing: &self.config.routing_policy,
                trace_sink: &mut sink,
            },
        )
    }

    pub fn tick_with_trace<T>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &impl ProgramCatalog,
        laws: &impl LawCatalog,
        stimuli: &[Stimulus],
        trace_sink: &mut T,
    ) -> TickResult
    where
        T: TraceSink,
    {
        self.execute_tick(
            tick,
            world,
            programs,
            laws,
            stimuli,
            RuntimeContext {
                routing: &self.config.routing_policy,
                trace_sink,
            },
        )
    }

    pub fn tick_with_routing<R>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &impl ProgramCatalog,
        laws: &impl LawCatalog,
        stimuli: &[Stimulus],
        routing: &R,
    ) -> TickResult
    where
        R: RoutingStrategy + Sync,
    {
        let mut sink = NoopTraceSink;
        self.execute_tick(
            tick,
            world,
            programs,
            laws,
            stimuli,
            RuntimeContext {
                routing,
                trace_sink: &mut sink,
            },
        )
    }

    fn execute_tick<T, R>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &impl ProgramCatalog,
        laws: &impl LawCatalog,
        stimuli: &[Stimulus],
        context: RuntimeContext<'_, R, T>,
    ) -> TickResult
    where
        T: TraceSink,
        R: RoutingStrategy + Sync,
    {
        let RuntimeContext {
            routing,
            trace_sink,
        } = context;
        let snapshot = world.snapshot();
        let read_version = snapshot.version();
        let mut transaction = TickTransaction::simulate(tick, read_version);
        let mut diagnostics = TickDiagnostics {
            tick,
            ..TickDiagnostics::default()
        };

        let merged_stimuli = merge_stimuli(stimuli);
        let prepared_sources =
            prepare_sources(snapshot, programs, self.config.activation_threshold);
        diagnostics.active_entities = prepared_sources
            .iter()
            .map(|source| source.entity_id)
            .collect();
        emit_trace(
            trace_sink,
            TraceEvent::TickStarted {
                tick,
                active_entities: diagnostics.active_entities.clone(),
            },
        );

        let dispatch_results = prepared_sources
            .par_iter()
            .map(|prepared| {
                dispatch_source(
                    self.config,
                    &self.stabilizer,
                    prepared,
                    snapshot,
                    laws,
                    routing,
                )
            })
            .collect::<Vec<_>>();

        let mut inbox: FxHashMap<EntityId, EmissionBuffer> = FxHashMap::default();
        let mut cohort_inbox: FxHashMap<graph_core::EntityKindId, EmissionBuffer> =
            FxHashMap::default();
        collect_dispatch_results(
            trace_sink,
            tick,
            &mut diagnostics,
            dispatch_results,
            &mut inbox,
            &mut cohort_inbox,
        );
        provenance::dedup_diagnostics(&mut diagnostics);

        let affected = collect_affected_entities(snapshot, &inbox, &cohort_inbox, &merged_stimuli);
        let mut updates = affected
            .par_iter()
            .filter_map(|entity_id| {
                compute_state_update(
                    *entity_id,
                    snapshot,
                    programs,
                    &inbox,
                    &cohort_inbox,
                    &merged_stimuli,
                    &self.stabilizer,
                )
            })
            .collect::<Vec<_>>();
        updates.sort_unstable_by_key(|update| update.entity_id.0);

        for update in &updates {
            emit_trace(
                trace_sink,
                TraceEvent::StateComputed {
                    tick,
                    entity: update.entity_id,
                },
            );
        }

        for update in &updates {
            transaction.record(
                update.entity_id,
                update.before.clone(),
                update.after.clone(),
                update.provenance.clone(),
            );
            emit_trace(
                trace_sink,
                TraceEvent::DeltaRecorded {
                    tick,
                    entity: update.entity_id,
                    source_entities: update.provenance.source_entities.clone(),
                    channel_ids: update.provenance.channel_ids.clone(),
                },
            );
        }

        let staged_states = updates
            .iter()
            .map(|update| (update.entity_id, update.after.clone()))
            .collect::<Vec<_>>();
        let committed = match world.commit_entity_states(read_version, &staged_states) {
            Ok(committed_version) => {
                transaction.mark_committed(committed_version);
                emit_trace(
                    trace_sink,
                    TraceEvent::TickCommitted {
                        tick,
                        committed_entities: staged_states
                            .iter()
                            .map(|(entity_id, _)| *entity_id)
                            .collect(),
                    },
                );
                true
            }
            Err(conflict) => {
                transaction.mark_conflict(conflict.expected, conflict.actual);
                emit_trace(trace_sink, TraceEvent::TickConflicted { tick });
                false
            }
        };

        if committed {
            decay_cooldowns(world);
        }

        TickResult {
            diagnostics,
            transaction,
        }
    }
}

fn collect_dispatch_results<T>(
    trace_sink: &mut T,
    tick: TickId,
    diagnostics: &mut TickDiagnostics,
    dispatch_results: Vec<source::SourceDispatchResult>,
    inbox: &mut FxHashMap<EntityId, EmissionBuffer>,
    cohort_inbox: &mut FxHashMap<graph_core::EntityKindId, EmissionBuffer>,
) where
    T: TraceSink,
{
    for result in dispatch_results {
        let emitted_channels = result.emitted_channels.clone();
        diagnostics.emitted_channels.extend(result.emitted_channels);
        diagnostics
            .promoted_to_field
            .extend(result.promoted_to_field);
        diagnostics
            .promoted_to_cohort
            .extend(result.promoted_to_cohort);
        diagnostics
            .interaction_kinds
            .extend(result.interaction_kinds);
        diagnostics.law_ids.extend(result.law_ids);
        diagnostics.total_emissions += result.total_emissions;
        if result.fanout_capped {
            diagnostics.fanout_capped_entities.push(result.source_id);
        }
        emit_trace(
            trace_sink,
            TraceEvent::SourceDispatched {
                tick,
                source: result.source_id,
                emitted_channels,
                total_emissions: result.total_emissions,
            },
        );

        for (target, emission) in result.direct_emissions {
            inbox.entry(target).or_default().push(emission);
        }
        for (kind, emission) in result.cohort_emissions {
            cohort_inbox.entry(kind).or_default().push(emission);
        }
    }
}

fn emit_trace<T>(trace_sink: &mut T, event: TraceEvent)
where
    T: TraceSink,
{
    if trace_sink.should_record(&event) {
        trace_sink.record(event);
    }
}
