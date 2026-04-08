use graph_core::{
    ChannelId, ChannelMode, Emission, EntityId, EntityKindId, InteractionKind, LawId, SignalVector,
};
use graph_world::WorldSnapshot;

use crate::{LawCatalog, ProgramCatalog, Stabilizer};

use super::EngineConfig;
use super::dispatch::{self, CohortMessage};
use super::routing::{DispatchPlan, RoutingStrategy, SelectorCache, plan_channel_dispatch};

#[derive(Debug, Clone)]
pub(crate) struct PreparedSource {
    pub entity_id: EntityId,
    pub outbound_signal: SignalVector,
}

#[derive(Default)]
pub(crate) struct SourceDispatchResult {
    pub source_id: EntityId,
    pub direct_emissions: Vec<(EntityId, Emission)>,
    pub cohort_emissions: Vec<(EntityKindId, Emission)>,
    pub emitted_channels: Vec<ChannelId>,
    pub promoted_to_field: Vec<ChannelId>,
    pub promoted_to_cohort: Vec<ChannelId>,
    pub interaction_kinds: Vec<InteractionKind>,
    pub law_ids: Vec<LawId>,
    pub total_emissions: usize,
    pub fanout_capped: bool,
}

#[derive(Clone)]
struct DispatchContext<'a> {
    entity: &'a graph_core::Entity,
    channel: &'a graph_core::Channel,
    interaction_kind: InteractionKind,
    template: Emission,
}

struct DirectDispatch<'a> {
    world: WorldSnapshot<'a>,
    mode: ChannelMode,
    targets: &'a [EntityId],
    available: usize,
}

pub(crate) fn prepare_sources(
    world: WorldSnapshot<'_>,
    programs: &impl ProgramCatalog,
    activation_threshold: f32,
) -> Vec<PreparedSource> {
    let mut prepared = world
        .entities()
        .filter_map(|entity| {
            if entity.state.cooldown > 0 {
                return None;
            }

            let outbound_signal = programs
                .get(entity.kind)
                .map(|program| program.outbound_signal(&entity.state))
                .unwrap_or_else(|| entity.state.emitted.clone());
            if outbound_signal.l2_norm() < activation_threshold {
                return None;
            }

            Some(PreparedSource {
                entity_id: entity.id,
                outbound_signal,
            })
        })
        .collect::<Vec<_>>();
    prepared.sort_unstable_by_key(|source| source.entity_id.0);
    prepared
}

pub(crate) fn dispatch_source<S, R>(
    config: EngineConfig,
    stabilizer: &S,
    prepared: &PreparedSource,
    world: WorldSnapshot<'_>,
    laws: &impl LawCatalog,
    routing: &R,
) -> SourceDispatchResult
where
    S: Stabilizer,
    R: RoutingStrategy,
{
    let Some(entity) = world.entity(prepared.entity_id) else {
        return SourceDispatchResult::default();
    };

    let mut remaining_targets = entity.budget.max_targets_per_tick;
    let mut selector_cache = SelectorCache::default();
    let mut result = SourceDispatchResult {
        source_id: prepared.entity_id,
        ..SourceDispatchResult::default()
    };

    for channel in world.outbound_channels(prepared.entity_id) {
        if remaining_targets == 0 {
            result.fanout_capped = true;
            break;
        }
        if !channel.enabled {
            continue;
        }
        let Some(law) = laws.get(channel.law) else {
            continue;
        };

        let dispatch_plan = plan_channel_dispatch(
            world,
            channel,
            routing,
            remaining_targets,
            &mut selector_cache,
        );
        record_promoted_mode(channel.id, dispatch_plan.mode(), channel.kind, &mut result);

        let signal = law
            .project(&prepared.outbound_signal, &entity.state, channel)
            .clamp_magnitude(entity.budget.max_signal_norm);
        let signal_norm = signal.l2_norm();
        if signal_norm < config.activation_threshold {
            continue;
        }

        result.emitted_channels.push(channel.id);
        result.interaction_kinds.push(law.kind());
        result.law_ids.push(channel.law);

        let template = Emission {
            magnitude: signal_norm,
            signal,
            cause: law.cause(channel),
            origin: None,
        };

        match dispatch_plan {
            DispatchPlan::Cohort { kinds, available } => dispatch_cohort(
                stabilizer,
                DispatchContext {
                    entity,
                    channel,
                    interaction_kind: law.kind(),
                    template,
                },
                kinds,
                available,
                &mut remaining_targets,
                &mut result,
            ),
            DispatchPlan::Direct {
                targets,
                available,
                mode,
            } => dispatch_direct(
                stabilizer,
                DispatchContext {
                    entity,
                    channel,
                    interaction_kind: law.kind(),
                    template,
                },
                DirectDispatch {
                    world,
                    mode,
                    targets,
                    available,
                },
                &mut remaining_targets,
                &mut result,
            ),
        }
    }

    result
}

fn record_promoted_mode(
    channel_id: ChannelId,
    resolved_mode: ChannelMode,
    declared_mode: ChannelMode,
    result: &mut SourceDispatchResult,
) {
    match resolved_mode {
        ChannelMode::Field if !matches!(declared_mode, ChannelMode::Field) => {
            result.promoted_to_field.push(channel_id);
        }
        ChannelMode::Cohort if !matches!(declared_mode, ChannelMode::Cohort) => {
            result.promoted_to_cohort.push(channel_id);
        }
        _ => {}
    }
}

fn dispatch_direct<S>(
    stabilizer: &S,
    context: DispatchContext<'_>,
    direct: DirectDispatch<'_>,
    remaining_targets: &mut usize,
    result: &mut SourceDispatchResult,
) where
    S: Stabilizer,
{
    if direct.targets.is_empty() {
        return;
    }
    if direct.targets.len() < direct.available {
        result.fanout_capped = true;
    }

    let route = dispatch::EmissionRoute {
        world: direct.world,
        channel: context.channel,
        delivery_mode: direct.mode,
    };
    match direct.mode {
        ChannelMode::Field => match context.channel.field_kernel {
            graph_core::FieldKernel::Flat => {
                for &target in direct.targets {
                    let emission = dispatch::materialize_non_field_target(
                        route,
                        target,
                        context.interaction_kind,
                        &context.template,
                        context.entity,
                    );
                    result.direct_emissions.push((
                        target,
                        stabilizer.stabilize_emission(context.entity, emission),
                    ));
                    result.total_emissions += 1;
                    *remaining_targets = remaining_targets.saturating_sub(1);
                }
            }
            graph_core::FieldKernel::Linear | graph_core::FieldKernel::Gaussian { .. } => {
                let evaluator = dispatch::FieldEvaluator::from_channel(context.channel);
                for &target in direct.targets {
                    let emission = dispatch::materialize_field_target(
                        route,
                        evaluator,
                        target,
                        context.interaction_kind,
                        &context.template,
                        context.entity,
                    );
                    result.direct_emissions.push((
                        target,
                        stabilizer.stabilize_emission(context.entity, emission),
                    ));
                    result.total_emissions += 1;
                    *remaining_targets = remaining_targets.saturating_sub(1);
                }
            }
        },
        ChannelMode::Pairwise | ChannelMode::Broadcast => {
            for &target in direct.targets {
                let emission = dispatch::materialize_non_field_target(
                    route,
                    target,
                    context.interaction_kind,
                    &context.template,
                    context.entity,
                );
                result.direct_emissions.push((
                    target,
                    stabilizer.stabilize_emission(context.entity, emission),
                ));
                result.total_emissions += 1;
                *remaining_targets = remaining_targets.saturating_sub(1);
            }
        }
        ChannelMode::Cohort => {}
    }
}

fn dispatch_cohort<S>(
    stabilizer: &S,
    context: DispatchContext<'_>,
    kinds: &[EntityKindId],
    available: usize,
    remaining_targets: &mut usize,
    result: &mut SourceDispatchResult,
) where
    S: Stabilizer,
{
    if kinds.is_empty() {
        return;
    }
    if kinds.len() < available {
        result.fanout_capped = true;
    }

    for &kind in kinds {
        let emission = stabilizer.stabilize_emission(
            context.entity,
            dispatch::materialize_cohort(CohortMessage {
                source: context.entity,
                channel: context.channel,
                target_kind: kind,
                interaction_kind: context.interaction_kind,
                template: context.template.clone(),
            }),
        );
        result.cohort_emissions.push((kind, emission));
        result.total_emissions += 1;
        *remaining_targets = remaining_targets.saturating_sub(1);
    }
}
