use graph_core::WorldEvent;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct TickEventCounts {
    pub(super) relationships_emerged: u32,
    pub(super) relationships_pruned: u32,
    pub(super) entities_born: u32,
    pub(super) entities_dormant: u32,
    pub(super) entities_revived: u32,
}

impl TickEventCounts {
    pub(super) fn from_events(events: &[WorldEvent]) -> Self {
        let mut counts = Self::default();
        for event in events {
            match event {
                WorldEvent::RelationshipEmerged { .. } => counts.relationships_emerged += 1,
                WorldEvent::RelationshipPruned { .. } => counts.relationships_pruned += 1,
                WorldEvent::EntityBorn { .. } => counts.entities_born += 1,
                WorldEvent::EntityDormant { .. } => counts.entities_dormant += 1,
                WorldEvent::EntityRevived { .. } => counts.entities_revived += 1,
                _ => {}
            }
        }
        counts
    }
}
