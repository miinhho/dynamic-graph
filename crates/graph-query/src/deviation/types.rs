use graph_core::{BatchId, EntityId, LocusId};

#[derive(Debug, Clone)]
pub struct EntityDiff {
    pub entity_id: EntityId,
    pub born_after_baseline: bool,
    pub went_dormant: bool,
    pub revived: bool,
    pub members_added: Vec<LocusId>,
    pub members_removed: Vec<LocusId>,
    pub membership_event_count: u32,
    pub coherence_at_baseline: f32,
    pub coherence_now: f32,
    pub coherence_delta: f32,
    pub member_count_delta: i64,
    pub latest_change_batch: Option<BatchId>,
}

impl EntityDiff {
    pub fn has_changes(&self) -> bool {
        self.born_after_baseline
            || self.went_dormant
            || self.revived
            || !self.members_added.is_empty()
            || !self.members_removed.is_empty()
            || self.membership_event_count > 0
            || self.coherence_delta.abs() > 1e-6
    }
}
