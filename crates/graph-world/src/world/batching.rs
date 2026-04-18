use graph_core::{BatchId, Change, ChangeId, RelationshipId};

use super::World;

impl World {
    pub fn current_batch(&self) -> BatchId {
        self.current_batch
    }

    pub fn last_committed_batch(&self) -> Option<BatchId> {
        let current = self.current_batch.0;
        if current == 0 {
            None
        } else {
            Some(BatchId(current - 1))
        }
    }

    pub fn mint_change_id(&mut self) -> ChangeId {
        let id = ChangeId(self.next_change_id);
        self.next_change_id += 1;
        id
    }

    pub fn reserve_change_ids(&mut self, n: usize) -> ChangeId {
        let base = ChangeId(self.next_change_id);
        self.next_change_id += n as u64;
        base
    }

    pub fn append_change(&mut self, change: Change) -> ChangeId {
        self.log.append(change)
    }

    pub fn extend_batch_changes(&mut self, changes: Vec<Change>) {
        self.log.extend_batch(changes);
    }

    pub fn advance_batch(&mut self) -> BatchId {
        self.current_batch = BatchId(self.current_batch.0 + 1);
        self.current_batch
    }

    pub fn record_pruned(&mut self, rel_id: RelationshipId) {
        self.pruned_log.push((rel_id, self.current_batch));
    }

    pub fn pruned_log(&self) -> &[(RelationshipId, BatchId)] {
        &self.pruned_log
    }

    pub fn trim_pruned_log_before(&mut self, batch: BatchId) {
        self.pruned_log
            .retain(|(_, entry_batch)| entry_batch.0 >= batch.0);
    }
}
