use graph_core::{Emission, Entity, EntityState, SignalVector, StateVector};

pub trait Stabilizer: Send + Sync {
    fn stabilize_emission(&self, source: &Entity, emission: Emission) -> Emission;
    fn stabilize_state(&self, source: &Entity, raw: EntityState) -> EntityState;
    fn relaxation_alpha(&self) -> f32;
}

pub trait StabilizationPolicy: Send + Sync {
    fn stabilize_emission(&self, source: &Entity, emission: Emission) -> Emission;
    fn stabilize_state(&self, source: &Entity, raw: EntityState) -> EntityState;
    fn relaxation_alpha(&self) -> f32;
}

pub struct PolicyStabilizer<P> {
    policy: P,
}

impl<P> PolicyStabilizer<P> {
    pub fn new(policy: P) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &P {
        &self.policy
    }

    pub fn into_policy(self) -> P {
        self.policy
    }
}

impl<P> Stabilizer for PolicyStabilizer<P>
where
    P: StabilizationPolicy,
{
    fn stabilize_emission(&self, source: &Entity, emission: Emission) -> Emission {
        self.policy.stabilize_emission(source, emission)
    }

    fn stabilize_state(&self, source: &Entity, raw: EntityState) -> EntityState {
        self.policy.stabilize_state(source, raw)
    }

    fn relaxation_alpha(&self) -> f32 {
        self.policy.relaxation_alpha()
    }
}

/// Component-wise saturation applied to emissions after the magnitude clamp.
///
/// `clamp_magnitude` already enforces a hard L2 ceiling on emitted signal,
/// but a hard ceiling can still feed unbounded local gain back through laws
/// during nonlinear interactions. A smooth saturation stage gives a softer
/// nonlinearity that bounds influence while preserving sign and shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SaturationMode {
    /// No additional saturation beyond the magnitude clamp.
    #[default]
    None,
    /// Component-wise `tanh`.
    Tanh,
    /// Component-wise `x / (1 + |x|)`.
    Softsign,
}

impl SaturationMode {
    fn apply(self, signal: SignalVector) -> SignalVector {
        match self {
            SaturationMode::None => signal,
            SaturationMode::Tanh => signal.saturated_tanh(),
            SaturationMode::Softsign => signal.saturated_softsign(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BasicStabilizer {
    /// Under-relaxation blend between previous and raw state. `0 < alpha <= 1`.
    pub alpha: f32,
    /// Multiplicative leak applied to the internal state per tick. `0 <= decay <= 1`.
    pub decay: f32,
    /// Optional smooth saturation applied to emitted signals.
    pub saturation: SaturationMode,
    /// Per-tick trust region: max L2 magnitude allowed for the change in
    /// internal state. `None` disables the trust region.
    pub trust_region: Option<f32>,
}

impl BasicStabilizer {
    /// Construct a stabilizer with only the legacy `alpha`/`decay` knobs set.
    /// Saturation is `None` and the trust region is disabled.
    pub fn new(alpha: f32, decay: f32) -> Self {
        Self {
            alpha,
            decay,
            saturation: SaturationMode::None,
            trust_region: None,
        }
    }

    /// Apply the per-tick trust region (if configured) to a candidate next
    /// internal state, returning the clamped value.
    fn apply_trust_region(&self, prev: &StateVector, candidate: StateVector) -> StateVector {
        let Some(max_delta) = self.trust_region else {
            return candidate;
        };
        if !(max_delta.is_finite() && max_delta > 0.0) {
            return candidate;
        }
        let delta = candidate.sub(prev);
        let norm = delta.l2_norm();
        if norm <= max_delta || norm == 0.0 {
            return candidate;
        }
        prev.add(&delta.scaled(max_delta / norm))
    }
}

impl Default for BasicStabilizer {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            decay: 1.0,
            saturation: SaturationMode::None,
            trust_region: None,
        }
    }
}

impl StabilizationPolicy for BasicStabilizer {
    fn stabilize_emission(&self, source: &Entity, mut emission: Emission) -> Emission {
        let clamped = emission
            .signal
            .clamp_magnitude(source.budget.max_signal_norm);
        emission.signal = self.saturation.apply(clamped);
        emission.magnitude = emission.signal.l2_norm();
        emission
    }

    fn stabilize_state(&self, source: &Entity, raw: EntityState) -> EntityState {
        let alpha = self.alpha.clamp(0.0, 1.0);
        let blended_internal = source
            .state
            .internal
            .scaled(1.0 - alpha)
            .add(&raw.internal.scaled(alpha))
            .scaled(self.decay);
        let bounded_internal = self.apply_trust_region(&source.state.internal, blended_internal);
        EntityState {
            internal: bounded_internal,
            emitted: source
                .state
                .emitted
                .scaled(1.0 - alpha)
                .add(&raw.emitted.scaled(alpha))
                .clamp_magnitude(source.budget.max_signal_norm),
            cooldown: raw.cooldown,
        }
    }

    fn relaxation_alpha(&self) -> f32 {
        self.alpha
    }
}

impl Stabilizer for BasicStabilizer {
    fn stabilize_emission(&self, source: &Entity, emission: Emission) -> Emission {
        <Self as StabilizationPolicy>::stabilize_emission(self, source, emission)
    }

    fn stabilize_state(&self, source: &Entity, raw: EntityState) -> EntityState {
        <Self as StabilizationPolicy>::stabilize_state(self, source, raw)
    }

    fn relaxation_alpha(&self) -> f32 {
        <Self as StabilizationPolicy>::relaxation_alpha(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{EmissionBudget, EntityKindId};

    fn entity_with_internal(values: Vec<f32>, max_signal_norm: f32) -> Entity {
        Entity {
            id: graph_core::EntityId(1),
            kind: EntityKindId(0),
            position: StateVector::default(),
            state: EntityState {
                internal: StateVector::new(values),
                emitted: SignalVector::default(),
                cooldown: 0,
            },
            refractory_period: 0,
            budget: EmissionBudget {
                max_targets_per_tick: usize::MAX,
                max_signal_norm,
            },
        }
    }

    #[test]
    fn trust_region_clamps_state_delta() {
        let stabilizer = BasicStabilizer {
            alpha: 1.0,
            decay: 1.0,
            saturation: SaturationMode::None,
            trust_region: Some(0.5),
        };
        let entity = entity_with_internal(vec![0.0], f32::MAX);
        let raw = EntityState {
            internal: StateVector::new(vec![10.0]),
            emitted: SignalVector::default(),
            cooldown: 0,
        };
        let stabilized = Stabilizer::stabilize_state(&stabilizer, &entity, raw);
        let value = stabilized.internal.values()[0];
        assert!((value - 0.5).abs() < 1e-6, "got {value}");
    }

    #[test]
    fn trust_region_passes_small_deltas() {
        let stabilizer = BasicStabilizer {
            alpha: 1.0,
            decay: 1.0,
            saturation: SaturationMode::None,
            trust_region: Some(2.0),
        };
        let entity = entity_with_internal(vec![0.0], f32::MAX);
        let raw = EntityState {
            internal: StateVector::new(vec![1.0]),
            emitted: SignalVector::default(),
            cooldown: 0,
        };
        let stabilized = Stabilizer::stabilize_state(&stabilizer, &entity, raw);
        assert!((stabilized.internal.values()[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tanh_saturation_bounds_emission_components() {
        let stabilizer = BasicStabilizer {
            alpha: 1.0,
            decay: 1.0,
            saturation: SaturationMode::Tanh,
            trust_region: None,
        };
        let entity = entity_with_internal(vec![0.0], f32::MAX);
        let emission = Emission {
            signal: SignalVector::new(vec![100.0, -100.0]),
            ..Emission::default()
        };
        let result = Stabilizer::stabilize_emission(&stabilizer, &entity, emission);
        for &value in result.signal.values() {
            assert!(value.abs() <= 1.0 + 1e-6, "value {value} out of bounds");
        }
    }

    #[test]
    fn default_matches_legacy_alpha_decay_one() {
        let stabilizer = BasicStabilizer::default();
        assert_eq!(stabilizer.alpha, 1.0);
        assert_eq!(stabilizer.decay, 1.0);
        assert_eq!(stabilizer.saturation, SaturationMode::None);
        assert!(stabilizer.trust_region.is_none());
    }
}
