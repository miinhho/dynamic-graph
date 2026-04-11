//! Subscriber registry: maps relationship IDs to the loci that want
//! notification when that relationship's state changes.
//!
//! The engine looks up subscribers here whenever a `ChangeSubject::Relationship`
//! change is committed. Each subscriber's locus receives the committed `Change`
//! in its inbox during the **same batch** — enabling meta-loci and event-loci to
//! react to relationship dynamics without a second program-dispatch path.
//!
//! Subscriptions are registered and cancelled via `StructuralProposal`
//! variants at end-of-batch, keeping the mechanism consistent with the
//! rest of topology mutation.

use graph_core::{BatchId, LocusId, RelationshipId};
use rustc_hash::{FxHashMap, FxHashSet};

/// A single subscription change event recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionEvent {
    /// The batch in which this subscription change was processed.
    pub batch: BatchId,
    /// The locus that subscribed or unsubscribed.
    pub subscriber: LocusId,
    /// The relationship being watched.
    pub rel_id: RelationshipId,
    /// `true` = subscribed, `false` = unsubscribed.
    pub subscribed: bool,
}

/// Tracks which loci have subscribed to which relationship state changes.
#[derive(Debug, Default, Clone)]
pub struct SubscriptionStore {
    /// rel_id → set of subscribing locus IDs.
    by_relationship: FxHashMap<RelationshipId, FxHashSet<LocusId>>,
    /// locus_id → set of watched relationship IDs (for cleanup on locus removal).
    by_locus: FxHashMap<LocusId, FxHashSet<RelationshipId>>,
    /// Monotonically incremented on every mutation. External consumers (e.g.
    /// storage layer) can compare against a previously saved value to detect
    /// whether the subscription set has changed since the last persist.
    generation: u64,
    /// Ordered log of every subscribe/unsubscribe event, tagged with the batch
    /// in which the proposal was applied. Used by `WorldDiff` to surface
    /// subscription topology changes in a given batch range.
    audit_log: Vec<SubscriptionEvent>,
}

impl SubscriptionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `subscriber` to receive inbox entries whenever `rel_id` changes.
    /// Idempotent — subscribing twice is equivalent to once.
    /// If `batch` is `Some`, records a `SubscriptionEvent` in the audit log on
    /// a new (non-duplicate) subscription.
    pub fn subscribe(&mut self, subscriber: LocusId, rel_id: RelationshipId) {
        self.subscribe_at(subscriber, rel_id, None);
    }

    /// Like `subscribe` but tags the event with a batch id for audit purposes.
    pub fn subscribe_at(&mut self, subscriber: LocusId, rel_id: RelationshipId, batch: Option<BatchId>) {
        let inserted = self.by_relationship.entry(rel_id).or_default().insert(subscriber);
        self.by_locus.entry(subscriber).or_default().insert(rel_id);
        if inserted {
            self.generation += 1;
            if let Some(b) = batch {
                self.audit_log.push(SubscriptionEvent {
                    batch: b,
                    subscriber,
                    rel_id,
                    subscribed: true,
                });
            }
        }
    }

    /// Cancel a subscription. Idempotent — unsubscribing when not subscribed is a no-op.
    pub fn unsubscribe(&mut self, subscriber: LocusId, rel_id: RelationshipId) {
        self.unsubscribe_at(subscriber, rel_id, None);
    }

    /// Like `unsubscribe` but tags the event with a batch id for audit purposes.
    pub fn unsubscribe_at(&mut self, subscriber: LocusId, rel_id: RelationshipId, batch: Option<BatchId>) {
        let removed = self.by_relationship.get_mut(&rel_id)
            .map(|s| s.remove(&subscriber))
            .unwrap_or(false);
        if let Some(rels) = self.by_locus.get_mut(&subscriber) {
            rels.remove(&rel_id);
        }
        if removed {
            self.generation += 1;
            if let Some(b) = batch {
                self.audit_log.push(SubscriptionEvent {
                    batch: b,
                    subscriber,
                    rel_id,
                    subscribed: false,
                });
            }
        }
    }

    /// Iterate the loci subscribed to `rel_id`.
    ///
    /// Returns an empty iterator if no loci are watching this relationship.
    pub fn subscribers(&self, rel_id: RelationshipId) -> impl Iterator<Item = LocusId> + '_ {
        self.by_relationship
            .get(&rel_id)
            .into_iter()
            .flat_map(|set| set.iter().copied())
    }

    /// True if at least one locus is watching `rel_id`.
    ///
    /// Cheap O(1) check used by the batch loop to skip subscriber
    /// resolution for relationships that nobody cares about (cold path).
    pub fn has_subscribers(&self, rel_id: RelationshipId) -> bool {
        self.by_relationship
            .get(&rel_id)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Remove all subscriptions pointing at `rel_id`.
    ///
    /// Called automatically when a relationship is deleted — either via
    /// `StructuralProposal::DeleteRelationship` or auto-pruning in
    /// `flush_relationship_decay`.
    pub fn remove_relationship(&mut self, rel_id: RelationshipId) {
        if let Some(subs) = self.by_relationship.remove(&rel_id) {
            if !subs.is_empty() { self.generation += 1; }
            for locus in subs {
                if let Some(rels) = self.by_locus.get_mut(&locus) {
                    rels.remove(&rel_id);
                }
            }
        }
    }

    /// Remove all subscriptions registered by `locus`.
    ///
    /// Should be called when a locus is removed from the world to avoid
    /// delivering notifications to a dangling ID.
    pub fn remove_locus(&mut self, locus: LocusId) {
        if let Some(rels) = self.by_locus.remove(&locus) {
            if !rels.is_empty() { self.generation += 1; }
            for rel_id in rels {
                if let Some(subs) = self.by_relationship.get_mut(&rel_id) {
                    subs.remove(&locus);
                }
            }
        }
    }

    /// Monotonic generation counter — incremented on every mutation.
    ///
    /// Compare against a previously saved value to detect whether the
    /// subscription set has changed since the last persist. The initial
    /// value is `0` (no subscriptions, nothing to persist).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// All subscription events in `[from_batch, to_batch)`.
    ///
    /// O(N) in audit log length. Call `trim_audit_before` periodically to keep
    /// the log bounded; without trimming this degrades over long runs.
    pub fn events_in_range(&self, from: BatchId, to: BatchId) -> impl Iterator<Item = &SubscriptionEvent> {
        self.audit_log
            .iter()
            .filter(move |e| e.batch.0 >= from.0 && e.batch.0 < to.0)
    }

    /// Discard audit log entries older than `before_batch`.
    ///
    /// Call periodically alongside `ChangeLog::trim_before_batch` to keep
    /// `events_in_range` O(recent events) rather than O(all-time events).
    pub fn trim_audit_before(&mut self, before_batch: BatchId) {
        self.audit_log.retain(|e| e.batch.0 >= before_batch.0);
    }

    pub fn is_empty(&self) -> bool {
        self.by_relationship.is_empty()
    }

    /// Total number of (subscriber, rel_id) pairs currently registered.
    pub fn subscription_count(&self) -> usize {
        self.by_relationship.values().map(|s| s.len()).sum()
    }

    /// Iterate all (rel_id, subscriber) pairs in arbitrary order.
    ///
    /// Used by storage to persist the full subscription state.
    pub fn iter(&self) -> impl Iterator<Item = (RelationshipId, LocusId)> + '_ {
        self.by_relationship.iter().flat_map(|(&rel_id, subs)| {
            subs.iter().copied().map(move |locus_id| (rel_id, locus_id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_has_subscribers() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let rel = RelationshipId(10);
        assert!(!store.has_subscribers(rel));
        store.subscribe(locus, rel);
        assert!(store.has_subscribers(rel));
    }

    #[test]
    fn subscribe_is_idempotent() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let rel = RelationshipId(10);
        store.subscribe(locus, rel);
        store.subscribe(locus, rel);
        assert_eq!(store.subscription_count(), 1);
    }

    #[test]
    fn unsubscribe_removes_entry() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let rel = RelationshipId(10);
        store.subscribe(locus, rel);
        store.unsubscribe(locus, rel);
        assert!(!store.has_subscribers(rel));
        assert_eq!(store.subscription_count(), 0);
    }

    #[test]
    fn remove_relationship_cleans_both_indices() {
        let mut store = SubscriptionStore::new();
        let a = LocusId(1);
        let b = LocusId(2);
        let rel = RelationshipId(10);
        store.subscribe(a, rel);
        store.subscribe(b, rel);
        store.remove_relationship(rel);
        assert!(!store.has_subscribers(rel));
        // by_locus entries should be cleaned up too
        assert_eq!(store.subscription_count(), 0);
    }

    #[test]
    fn remove_locus_cleans_both_indices() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let rel_a = RelationshipId(10);
        let rel_b = RelationshipId(11);
        store.subscribe(locus, rel_a);
        store.subscribe(locus, rel_b);
        store.remove_locus(locus);
        assert!(!store.has_subscribers(rel_a));
        assert!(!store.has_subscribers(rel_b));
        assert_eq!(store.subscription_count(), 0);
    }

    #[test]
    fn multiple_subscribers_to_one_relationship() {
        let mut store = SubscriptionStore::new();
        let rel = RelationshipId(10);
        store.subscribe(LocusId(1), rel);
        store.subscribe(LocusId(2), rel);
        store.subscribe(LocusId(3), rel);
        let mut ids: Vec<LocusId> = store.subscribers(rel).collect();
        ids.sort_by_key(|l| l.0);
        assert_eq!(ids, vec![LocusId(1), LocusId(2), LocusId(3)]);
    }
}
