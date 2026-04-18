use graph_core::{BatchId, InfluenceKindId};

use crate::plasticity::{PairPredictionObjective, PlasticityLearners, PlasticityObservation};

use super::Simulation;

impl Simulation {
    pub fn set_plasticity_learners(&mut self, mut learners: PlasticityLearners) {
        for kind in self.base_influences.kinds() {
            learners.register(kind);
        }
        self.plasticity_learners = Some(learners);
    }

    pub fn plasticity_learners(&self) -> Option<&PlasticityLearners> {
        self.plasticity_learners.as_ref()
    }

    pub fn current_plasticity_scale(&self, kind: InfluenceKindId) -> f32 {
        self.plasticity_learners
            .as_ref()
            .map(|learners| learners.current(kind))
            .unwrap_or(1.0)
    }

    pub fn evaluate_pair_prediction(
        &self,
        objective: &PairPredictionObjective,
        future_events: &[Vec<Vec<u64>>],
        from_batch: BatchId,
        to_batch: BatchId,
    ) -> PlasticityObservation {
        let world = self.world();
        objective.score_window(&world, future_events, from_batch, to_batch)
    }

    pub fn observe_plasticity_objective(
        &self,
        kind: InfluenceKindId,
        obs: PlasticityObservation,
    ) -> bool {
        let Some(learners) = self.plasticity_learners.as_ref() else {
            return false;
        };
        learners.observe(kind, obs);
        true
    }

    pub fn evaluate_and_observe_pair_prediction(
        &self,
        objective: &PairPredictionObjective,
        future_events: &[Vec<Vec<u64>>],
        from_batch: BatchId,
        to_batch: BatchId,
    ) -> Option<PlasticityObservation> {
        let obs = self.evaluate_pair_prediction(objective, future_events, from_batch, to_batch);
        self.observe_plasticity_objective(objective.kind, obs)
            .then_some(obs)
    }
}
