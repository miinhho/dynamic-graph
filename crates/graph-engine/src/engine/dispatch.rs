use graph_core::{
    Channel, ChannelMode, Emission, EmissionOrigin, Entity, EntityId, EntityKindId, InteractionKind,
};
use graph_world::WorldSnapshot;

#[derive(Clone, Copy)]
pub struct EmissionRoute<'a> {
    pub world: WorldSnapshot<'a>,
    pub channel: &'a Channel,
    pub delivery_mode: ChannelMode,
}

pub struct CohortMessage<'a> {
    pub source: &'a Entity,
    pub channel: &'a Channel,
    pub target_kind: EntityKindId,
    pub interaction_kind: InteractionKind,
    pub template: Emission,
}

#[derive(Clone, Copy)]
pub enum FieldEvaluator {
    Flat,
    Linear { inverse_radius: Option<f32> },
    Gaussian { inverse_two_sigma_sq: f32 },
}

impl FieldEvaluator {
    pub fn from_channel(channel: &Channel) -> Self {
        match (&channel.field_kernel, channel.field_radius) {
            (graph_core::FieldKernel::Flat, _) => Self::Flat,
            (graph_core::FieldKernel::Linear, Some(radius)) => Self::Linear {
                inverse_radius: Some(1.0 / radius.max(f32::EPSILON)),
            },
            (graph_core::FieldKernel::Linear, None) => Self::Linear {
                inverse_radius: None,
            },
            (graph_core::FieldKernel::Gaussian { sigma }, _) => {
                let sigma = sigma.max(f32::EPSILON);
                Self::Gaussian {
                    inverse_two_sigma_sq: 1.0 / (2.0 * sigma * sigma),
                }
            }
        }
    }

    fn scale(self, distance: f32) -> f32 {
        match self {
            Self::Flat => 1.0,
            Self::Linear {
                inverse_radius: Some(inverse_radius),
            } => (1.0 - distance * inverse_radius).clamp(0.0, 1.0),
            Self::Linear {
                inverse_radius: None,
            } => 1.0 / (1.0 + distance),
            Self::Gaussian {
                inverse_two_sigma_sq,
            } => (-(distance * distance) * inverse_two_sigma_sq).exp(),
        }
    }
}

pub fn materialize_field_target(
    route: EmissionRoute<'_>,
    evaluator: FieldEvaluator,
    target: EntityId,
    interaction_kind: InteractionKind,
    template: &Emission,
    source: &Entity,
) -> Emission {
    debug_assert!(matches!(route.delivery_mode, ChannelMode::Field));
    let scale = emission_scale(route.world, evaluator, &source.position, target);
    let signal = template.signal.scaled(scale);
    Emission {
        signal: signal.clone(),
        magnitude: signal
            .l2_norm()
            .min(template.magnitude)
            .min(source.budget.max_signal_norm)
            .min(source.state.emitted.l2_norm()),
        cause: template.cause,
        origin: Some(EmissionOrigin {
            source: route.channel.source,
            target,
            channel: route.channel.id,
            law: route.channel.law,
            kind: interaction_kind,
        }),
    }
}

pub fn materialize_non_field_target(
    route: EmissionRoute<'_>,
    target: EntityId,
    interaction_kind: InteractionKind,
    template: &Emission,
    source: &Entity,
) -> Emission {
    Emission {
        signal: template.signal.clone(),
        magnitude: template
            .magnitude
            .min(source.budget.max_signal_norm)
            .min(source.state.emitted.l2_norm()),
        cause: template.cause,
        origin: Some(EmissionOrigin {
            source: route.channel.source,
            target,
            channel: route.channel.id,
            law: route.channel.law,
            kind: interaction_kind,
        }),
    }
}

pub fn materialize_cohort(message: CohortMessage<'_>) -> Emission {
    let signal = message
        .template
        .signal
        .clamp_magnitude(message.source.budget.max_signal_norm);
    Emission {
        signal: signal.clone(),
        magnitude: signal.l2_norm().min(message.template.magnitude),
        cause: message.template.cause,
        origin: Some(EmissionOrigin {
            source: message.channel.source,
            target: EntityId(message.target_kind.0),
            channel: message.channel.id,
            law: message.channel.law,
            kind: message.interaction_kind,
        }),
    }
}

pub fn retarget_emission(mut emission: Emission, entity_id: EntityId) -> Emission {
    if let Some(origin) = &mut emission.origin {
        origin.target = entity_id;
    }
    emission
}

pub fn sum_emissions(emissions: &[Emission]) -> graph_core::SignalVector {
    emissions
        .iter()
        .fold(graph_core::SignalVector::default(), |acc, emission| {
            acc.add(&emission.signal)
        })
}

fn emission_scale(
    world: WorldSnapshot<'_>,
    evaluator: FieldEvaluator,
    source_position: &graph_core::StateVector,
    target: EntityId,
) -> f32 {
    let Some(target_entity) = world.entity(target) else {
        return 0.0;
    };
    let distance = source_position.distance(&target_entity.position);
    evaluator.scale(distance)
}
