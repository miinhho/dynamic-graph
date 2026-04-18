use graph_core::{BatchId, Cohere};

use super::{CohereSnapshot, CohereStore};

pub(super) fn update(
    store: &mut CohereStore,
    perspective: impl Into<String>,
    coheres: Vec<Cohere>,
) {
    update_at(store, perspective, coheres, BatchId(0), false);
}

pub(super) fn update_at(
    store: &mut CohereStore,
    perspective: impl Into<String>,
    coheres: Vec<Cohere>,
    batch: BatchId,
    record_batch: bool,
) {
    let key = perspective.into();
    maybe_record_previous_snapshot(store, &key, batch, record_batch);
    store.by_perspective.insert(key, coheres);
}

fn maybe_record_previous_snapshot(
    store: &mut CohereStore,
    key: &str,
    batch: BatchId,
    record_batch: bool,
) {
    if store.max_history == 0 {
        return;
    }
    let Some(previous) = store.by_perspective.get(key) else {
        return;
    };
    if previous.is_empty() {
        return;
    }

    let snapshot_batch = if record_batch { batch } else { BatchId(0) };
    let ring = store.history.entry(key.to_string()).or_default();
    ring.push_back(CohereSnapshot {
        batch: snapshot_batch,
        coheres: previous.clone(),
    });
    while ring.len() > store.max_history {
        ring.pop_front();
    }
}
