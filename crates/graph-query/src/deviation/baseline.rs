use graph_core::{BatchId, CompressionLevel, Entity, EntityLayer};

pub(super) struct BaselineState {
    pub coherence: f32,
    pub member_count: i64,
}

pub(super) fn baseline_state(entity: &Entity, baseline: BatchId) -> BaselineState {
    let mut state = BaselineState {
        coherence: 0.0,
        member_count: 0,
    };

    for layer in &entity.layers {
        if layer.batch > baseline {
            break;
        }
        apply_baseline_layer(&mut state, layer);
    }

    state
}

fn apply_baseline_layer(state: &mut BaselineState, layer: &EntityLayer) {
    match &layer.compression {
        CompressionLevel::Full => {
            if let Some(snapshot) = &layer.snapshot {
                state.coherence = snapshot.coherence;
                state.member_count = snapshot.members.len() as i64;
            }
        }
        CompressionLevel::Compressed {
            coherence,
            member_count,
            ..
        }
        | CompressionLevel::Skeleton {
            coherence,
            member_count,
            ..
        } => {
            state.coherence = *coherence;
            state.member_count = i64::from(*member_count);
        }
    }
}
