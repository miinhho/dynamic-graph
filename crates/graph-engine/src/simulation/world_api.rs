use graph_core::WorldEvent;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::engine;

use super::Simulation;

impl Simulation {
    pub fn recognize_entities(
        &mut self,
        perspective: &dyn EmergencePerspective,
    ) -> Vec<WorldEvent> {
        engine::world_ops::recognize_entities(
            &mut self.world.write().unwrap(),
            &self.base_influences,
            perspective,
        )
    }

    pub fn extract_cohere(&mut self, perspective: &dyn CoherePerspective) {
        engine::world_ops::extract_cohere(
            &mut self.world.write().unwrap(),
            &self.base_influences,
            perspective,
        );
    }

    pub fn flush_relationship_decay(&mut self) {
        engine::world_ops::flush_relationship_decay(
            &mut self.world.write().unwrap(),
            &self.base_influences,
        );
    }

    pub fn weather_entities(&mut self, policy: &dyn graph_core::EntityWeatheringPolicy) {
        self.engine
            .weather_entities(&mut self.world.write().unwrap(), policy);
    }

    pub fn trim_change_log(&mut self, retention_batches: u64) -> usize {
        self.engine
            .trim_change_log(&mut self.world.write().unwrap(), retention_batches)
    }
}
