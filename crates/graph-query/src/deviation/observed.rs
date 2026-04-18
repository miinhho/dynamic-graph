use graph_core::{
    BatchId, CompressedTransition, CompressionLevel, Entity, EntityLayer, LayerTransition, LocusId,
};

pub(super) struct ObservedTransitions {
    pub born_after_baseline: bool,
    pub went_dormant: bool,
    pub revived: bool,
    pub members_added: Vec<LocusId>,
    pub members_removed: Vec<LocusId>,
    pub membership_event_count: u32,
    pub latest_change_batch: Option<BatchId>,
}

pub(super) fn observe_layers_since_baseline(
    entity: &Entity,
    baseline: BatchId,
) -> ObservedTransitions {
    let mut observed = ObservedTransitions {
        born_after_baseline: false,
        went_dormant: false,
        revived: false,
        members_added: Vec::new(),
        members_removed: Vec::new(),
        membership_event_count: 0,
        latest_change_batch: None,
    };

    for layer in entity.layers.iter().filter(|layer| layer.batch > baseline) {
        observed.observe_layer(layer);
    }

    observed
}

impl ObservedTransitions {
    fn observe_layer(&mut self, layer: &EntityLayer) {
        self.record_latest_change(layer.batch);
        match &layer.compression {
            CompressionLevel::Full => self.observe_full_transition(layer),
            CompressionLevel::Compressed {
                transition_kind, ..
            }
            | CompressionLevel::Skeleton {
                transition_kind, ..
            } => self.observe_compressed_transition(*transition_kind),
        }
    }

    fn record_latest_change(&mut self, batch: BatchId) {
        match self.latest_change_batch {
            Some(previous) if previous >= batch => {}
            _ => self.latest_change_batch = Some(batch),
        }
    }

    fn observe_full_transition(&mut self, layer: &EntityLayer) {
        match &layer.transition {
            LayerTransition::Born => {
                self.born_after_baseline = true;
            }
            LayerTransition::BecameDormant => {
                self.went_dormant = true;
            }
            LayerTransition::Revived => {
                self.revived = true;
            }
            LayerTransition::MembershipDelta { added, removed } => {
                self.membership_event_count += 1;
                self.members_added.extend_from_slice(added);
                self.members_removed.extend_from_slice(removed);
            }
            LayerTransition::CoherenceShift { .. } => {}
            LayerTransition::Split { .. } | LayerTransition::Merged { .. } => {
                self.membership_event_count += 1;
            }
        }
    }

    fn observe_compressed_transition(&mut self, kind: CompressedTransition) {
        match kind {
            CompressedTransition::Born => self.born_after_baseline = true,
            CompressedTransition::BecameDormant => self.went_dormant = true,
            CompressedTransition::Revived => self.revived = true,
            CompressedTransition::MembershipDelta
            | CompressedTransition::Split
            | CompressedTransition::Merged => self.membership_event_count += 1,
            CompressedTransition::CoherenceShift => {}
        }
    }
}
