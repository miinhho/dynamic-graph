use crate::World;

#[derive(Debug, Clone, Copy, Default)]
pub struct WorldMetrics {
    pub entity_count: usize,
}

pub fn metrics(world: &World) -> WorldMetrics {
    WorldMetrics {
        entity_count: world.entities().count(),
    }
}
