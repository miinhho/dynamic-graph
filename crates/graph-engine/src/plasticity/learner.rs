use std::sync::atomic::{AtomicU32, Ordering};

use graph_core::InfluenceKindId;
use rustc_hash::FxHashMap;

use super::objective::PlasticityObservation;

#[derive(Debug)]
struct PlasticityLearnerState {
    scale: AtomicU32,
    smoothed_signal: AtomicU32,
}

impl PlasticityLearnerState {
    fn new() -> Self {
        Self {
            scale: AtomicU32::new(PlasticityAdaptationPolicy::INITIAL_SCALE.to_bits()),
            smoothed_signal: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    fn current_scale(&self) -> f32 {
        f32::from_bits(self.scale.load(Ordering::Relaxed))
    }

    fn current_smoothed_signal(&self) -> f32 {
        f32::from_bits(self.smoothed_signal.load(Ordering::Relaxed))
    }

    fn reset(&self) {
        self.scale.store(
            PlasticityAdaptationPolicy::INITIAL_SCALE.to_bits(),
            Ordering::Relaxed,
        );
        self.smoothed_signal
            .store(0.0f32.to_bits(), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
struct PlasticityAssessment {
    signal: f32,
    confidence: f32,
}

impl PlasticityAssessment {
    fn from_observation(observation: PlasticityObservation) -> Self {
        Self {
            signal: observation.adaptation_signal(),
            confidence: observation.adaptation_confidence(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PlasticityStateTransition {
    next_signal: f32,
    next_scale: f32,
}

#[derive(Debug, Clone, Copy)]
struct PlasticityAdaptationPolicy {
    min_scale: f32,
    max_scale: f32,
    step_bound: f32,
    signal_gain: f32,
    min_signal_alpha: f32,
    max_signal_alpha: f32,
}

impl PlasticityAdaptationPolicy {
    const INITIAL_SCALE: f32 = 1.0;

    const DEFAULT: Self = Self {
        min_scale: 0.1,
        max_scale: 3.0,
        step_bound: 1.2,
        signal_gain: 0.4,
        min_signal_alpha: 0.15,
        max_signal_alpha: 0.55,
    };

    fn transition(
        self,
        current_scale: f32,
        previous_signal: f32,
        assessment: PlasticityAssessment,
    ) -> PlasticityStateTransition {
        let alpha = self.signal_alpha(assessment.confidence);
        let next_signal = previous_signal + alpha * (assessment.signal - previous_signal);
        let bounded_multiplier = (1.0 + self.signal_gain * assessment.confidence * next_signal)
            .clamp(1.0 / self.step_bound, self.step_bound);
        let next_scale = (current_scale * bounded_multiplier).clamp(self.min_scale, self.max_scale);

        PlasticityStateTransition {
            next_signal,
            next_scale,
        }
    }

    fn signal_alpha(self, confidence: f32) -> f32 {
        self.min_signal_alpha + (self.max_signal_alpha - self.min_signal_alpha) * confidence
    }
}

pub struct PlasticityLearners {
    states: FxHashMap<InfluenceKindId, PlasticityLearnerState>,
}

impl Default for PlasticityLearners {
    fn default() -> Self {
        Self::new()
    }
}

impl PlasticityLearners {
    pub fn new() -> Self {
        Self {
            states: FxHashMap::default(),
        }
    }

    pub fn register(&mut self, kind: InfluenceKindId) {
        self.states
            .entry(kind)
            .or_insert_with(PlasticityLearnerState::new);
    }

    pub fn observe(&self, kind: InfluenceKindId, observation: PlasticityObservation) {
        let Some(state) = self.states.get(&kind) else {
            return;
        };

        let transition = learner_transition(state, observation);

        state
            .smoothed_signal
            .store(transition.next_signal.to_bits(), Ordering::Relaxed);
        state
            .scale
            .store(transition.next_scale.to_bits(), Ordering::Relaxed);
    }

    pub fn current(&self, kind: InfluenceKindId) -> f32 {
        self.states
            .get(&kind)
            .map(PlasticityLearnerState::current_scale)
            .unwrap_or(PlasticityAdaptationPolicy::INITIAL_SCALE)
    }

    pub fn reset(&self, kind: InfluenceKindId) {
        if let Some(state) = self.states.get(&kind) {
            state.reset();
        }
    }

    pub fn reset_all(&self) {
        for state in self.states.values() {
            state.reset();
        }
    }
}

fn learner_transition(
    state: &PlasticityLearnerState,
    observation: PlasticityObservation,
) -> PlasticityStateTransition {
    PlasticityAdaptationPolicy::DEFAULT.transition(
        state.current_scale(),
        state.current_smoothed_signal(),
        PlasticityAssessment::from_observation(observation),
    )
}
