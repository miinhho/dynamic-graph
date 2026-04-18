use graph_core::{EntityId, EntityStatus};
use graph_world::World;

use super::NameMap;

#[derive(Debug, Clone)]
pub struct EntitySummary {
    pub entity_id: EntityId,
    pub display_name: String,
    pub coherence: f32,
    pub member_names: Vec<String>,
    pub status: String,
    pub layer_count: usize,
}

impl std::fmt::Display for EntitySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Entity#{} [{}]  coherence={:.3}  status={}  layers={}  members=[{}]",
            self.entity_id.0,
            self.display_name,
            self.coherence,
            self.status,
            self.layer_count,
            self.member_names.join(", "),
        )
    }
}

pub fn entity_summary(
    world: &World,
    entity_id: EntityId,
    names: &NameMap,
) -> Option<EntitySummary> {
    let entity = world.entities().get(entity_id)?;
    Some(make_summary(entity, names))
}

pub fn entities_summary(world: &World, names: &NameMap) -> Vec<EntitySummary> {
    let mut summaries: Vec<EntitySummary> = world
        .entities()
        .iter()
        .map(|entity| make_summary(entity, names))
        .collect();
    summaries.sort_by(|a, b| {
        b.coherence
            .partial_cmp(&a.coherence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    summaries
}

pub(crate) fn make_summary(entity: &graph_core::Entity, names: &NameMap) -> EntitySummary {
    let member_names: Vec<String> = entity
        .current
        .members
        .iter()
        .map(|&id| names.name(id))
        .collect();

    EntitySummary {
        entity_id: entity.id,
        display_name: display_name(entity, &member_names),
        coherence: entity.current.coherence,
        member_names,
        status: status_label(entity.status),
        layer_count: entity.layers.len(),
    }
}

fn display_name(entity: &graph_core::Entity, member_names: &[String]) -> String {
    let first_three: Vec<&str> = member_names
        .iter()
        .take(3)
        .map(|name| name.as_str())
        .collect();
    if entity.current.members.len() > 3 {
        format!("{}…", first_three.join(", "))
    } else {
        first_three.join(", ")
    }
}

fn status_label(status: EntityStatus) -> String {
    match status {
        EntityStatus::Active => "active".to_owned(),
        EntityStatus::Dormant => "dormant".to_owned(),
    }
}
