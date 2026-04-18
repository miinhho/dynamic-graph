use graph_core::BatchId;

use super::{SubscriptionEvent, SubscriptionStore};

impl SubscriptionStore {
    pub fn events_in_range(
        &self,
        from: BatchId,
        to: BatchId,
    ) -> impl Iterator<Item = &SubscriptionEvent> {
        self.audit_log
            .range(from.0..to.0)
            .flat_map(|(_, events)| events.iter())
    }

    pub fn trim_audit_before(&mut self, before_batch: BatchId) {
        self.audit_log = self.audit_log.split_off(&before_batch.0);
    }
}
