//! Subscriber registry: maps relationship IDs to the loci that want
//! notification when that relationship's state changes.
//!
//! The engine looks up subscribers here whenever a `ChangeSubject::Relationship`
//! change is committed. Each subscriber's locus receives the committed `Change`
//! in its inbox during the **same batch** — enabling meta-loci and event-loci to
//! react to relationship dynamics without a second program-dispatch path.
//!
//! ## Subscription scopes
//!
//! Three granularities are supported:
//!
//! - **Specific** — watch a single `RelationshipId`. The original per-edge mechanism.
//! - **AllOfKind** — watch every relationship of a given `InfluenceKindId`. A single
//!   registration covers all edges of that kind, present or future.
//! - **TouchingLocus** — watch relationships of a given kind that involve a specific
//!   anchor locus as either endpoint. Useful for "monitor all supply edges entering
//!   warehouse X" without enumerating them.
//!
//! ## Performance properties
//!
//! | Operation | Complexity |
//! |-----------|-----------|
//! | `has_subscribers` / `has_any_subscribers` | O(1) |
//! | `collect_subscribers` | O(k) — k = total matching subscribers |
//! | `subscribe_at` / `unsubscribe_at` | O(1) amortised |
//! | `events_in_range` | O(log N + k) — N = total audit batches |
//! | `trim_audit_before` | O(log N) |
//! | `subscription_count` | O(1) — cached |

use std::collections::BTreeMap;

use graph_core::{BatchId, InfluenceKindId, LocusId, RelationshipId};
use rustc_hash::{FxHashMap, FxHashSet};

/// A single subscription change event recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionEvent {
    /// The batch in which this subscription change was processed.
    pub batch: BatchId,
    /// The locus that subscribed or unsubscribed.
    pub subscriber: LocusId,
    /// The relationship being watched (Specific scope only).
    pub rel_id: RelationshipId,
    /// `true` = subscribed, `false` = unsubscribed.
    pub subscribed: bool,
}

/// Scope of a subscription — what class of relationships a subscriber watches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SubscriptionScope {
    /// Watch a specific relationship by ID. Equivalent to the legacy
    /// `subscribe(subscriber, rel_id)` call.
    Specific(RelationshipId),
    /// Watch **all** relationships of a given influence kind, regardless of
    /// endpoints. Covers relationships created in the future as well.
    AllOfKind(InfluenceKindId),
    /// Watch relationships of a given influence kind that involve `anchor` as
    /// either the `from` or `to` endpoint.
    TouchingLocus {
        anchor: LocusId,
        kind: InfluenceKindId,
    },
}

/// Tracks which loci have subscribed to which relationship state changes.
///
/// Internally maintains three independent indexes for the three subscription
/// scopes, plus bidirectional reverse indexes so all cleanup operations
/// (locus removal, relationship removal) remain O(k) — never O(total).
#[derive(Debug, Default, Clone)]
pub struct SubscriptionStore {
    // ── Specific: rel_id → subscriber set ────────────────────────────────
    by_relationship: FxHashMap<RelationshipId, FxHashSet<LocusId>>,
    /// Reverse: subscriber → watched rel_ids (for subscriber-locus cleanup).
    by_locus: FxHashMap<LocusId, FxHashSet<RelationshipId>>,

    // ── AllOfKind: kind → subscriber set ──────────────────────────────────
    by_kind: FxHashMap<InfluenceKindId, FxHashSet<LocusId>>,
    /// Reverse: subscriber → watched kinds (for subscriber-locus cleanup).
    kinds_by_subscriber: FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,

    // ── TouchingLocus: (anchor, kind) → subscriber set ────────────────────
    by_anchor_kind: FxHashMap<(LocusId, InfluenceKindId), FxHashSet<LocusId>>,
    /// Reverse: subscriber → {(anchor, kind)} (for subscriber-locus cleanup).
    anchor_kinds_by_subscriber: FxHashMap<LocusId, FxHashSet<(LocusId, InfluenceKindId)>>,
    /// Reverse: anchor → {kind} (for anchor-locus cleanup).
    kinds_by_anchor: FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,

    // ── Metadata ──────────────────────────────────────────────────────────
    /// Monotone mutation counter — incremented on every real change.
    generation: u64,
    /// Cached count of **Specific** subscriptions (not kind-level).
    /// O(1) to read.
    total_count: usize,

    // ── Audit log ─────────────────────────────────────────────────────────
    /// Ordered log of Specific-scope subscribe/unsubscribe events, keyed by
    /// `BatchId.0` for O(log N + k) range queries.
    ///
    /// Kind-level subscription changes are tracked via `generation` (for
    /// storage skip) but are not recorded in the batch-scoped audit log
    /// because they are not tied to a specific relationship ID.
    audit_log: BTreeMap<u64, Vec<SubscriptionEvent>>,
}

impl SubscriptionStore {
    pub fn new() -> Self {
        Self::default()
    }

    // =========================================================================
    // Specific-scope subscriptions (legacy API, unchanged semantics)
    // =========================================================================

    /// Register `subscriber` to receive inbox entries whenever `rel_id` changes.
    /// Idempotent — subscribing twice is equivalent to once.
    pub fn subscribe(&mut self, subscriber: LocusId, rel_id: RelationshipId) {
        self.subscribe_at(subscriber, rel_id, None);
    }

    /// Like `subscribe` but tags the audit event with a batch id.
    pub fn subscribe_at(
        &mut self,
        subscriber: LocusId,
        rel_id: RelationshipId,
        batch: Option<BatchId>,
    ) {
        let inserted = self
            .by_relationship
            .entry(rel_id)
            .or_default()
            .insert(subscriber);
        self.by_locus.entry(subscriber).or_default().insert(rel_id);
        if inserted {
            self.generation += 1;
            self.total_count += 1;
            if let Some(b) = batch {
                self.audit_log
                    .entry(b.0)
                    .or_default()
                    .push(SubscriptionEvent {
                        batch: b,
                        subscriber,
                        rel_id,
                        subscribed: true,
                    });
            }
        }
    }

    /// Cancel a specific subscription. Idempotent.
    pub fn unsubscribe(&mut self, subscriber: LocusId, rel_id: RelationshipId) {
        self.unsubscribe_at(subscriber, rel_id, None);
    }

    /// Like `unsubscribe` but tags the audit event with a batch id.
    pub fn unsubscribe_at(
        &mut self,
        subscriber: LocusId,
        rel_id: RelationshipId,
        batch: Option<BatchId>,
    ) {
        let removed = self
            .by_relationship
            .get_mut(&rel_id)
            .map(|s| s.remove(&subscriber))
            .unwrap_or(false);
        if let Some(rels) = self.by_locus.get_mut(&subscriber) {
            rels.remove(&rel_id);
        }
        if removed {
            self.generation += 1;
            self.total_count -= 1;
            if let Some(b) = batch {
                self.audit_log
                    .entry(b.0)
                    .or_default()
                    .push(SubscriptionEvent {
                        batch: b,
                        subscriber,
                        rel_id,
                        subscribed: false,
                    });
            }
        }
    }

    // =========================================================================
    // Kind-level subscriptions
    // =========================================================================

    /// Subscribe `subscriber` to all relationships of the given influence kind.
    ///
    /// The subscriber will receive inbox entries for every change committed to
    /// any relationship whose kind matches — including relationships that did
    /// not exist at subscription time.
    pub fn subscribe_to_kind(&mut self, subscriber: LocusId, kind: InfluenceKindId) {
        let inserted = self.by_kind.entry(kind).or_default().insert(subscriber);
        if inserted {
            self.generation += 1;
            self.kinds_by_subscriber
                .entry(subscriber)
                .or_default()
                .insert(kind);
        }
    }

    /// Cancel an AllOfKind subscription. Idempotent.
    pub fn unsubscribe_from_kind(&mut self, subscriber: LocusId, kind: InfluenceKindId) {
        let removed = self
            .by_kind
            .get_mut(&kind)
            .map(|s| s.remove(&subscriber))
            .unwrap_or(false);
        if removed {
            self.generation += 1;
            if let Some(kinds) = self.kinds_by_subscriber.get_mut(&subscriber) {
                kinds.remove(&kind);
            }
        }
    }

    /// Subscribe `subscriber` to all relationships of `kind` that touch `anchor`
    /// (i.e. `anchor` appears as either the `from` or `to` endpoint).
    pub fn subscribe_to_anchor_kind(
        &mut self,
        subscriber: LocusId,
        anchor: LocusId,
        kind: InfluenceKindId,
    ) {
        let key = (anchor, kind);
        let inserted = self
            .by_anchor_kind
            .entry(key)
            .or_default()
            .insert(subscriber);
        if inserted {
            self.generation += 1;
            self.anchor_kinds_by_subscriber
                .entry(subscriber)
                .or_default()
                .insert(key);
            self.kinds_by_anchor
                .entry(anchor)
                .or_default()
                .insert(kind);
        }
    }

    /// Cancel a TouchingLocus subscription. Idempotent.
    pub fn unsubscribe_from_anchor_kind(
        &mut self,
        subscriber: LocusId,
        anchor: LocusId,
        kind: InfluenceKindId,
    ) {
        let key = (anchor, kind);
        let removed = self
            .by_anchor_kind
            .get_mut(&key)
            .map(|s| s.remove(&subscriber))
            .unwrap_or(false);
        if removed {
            self.generation += 1;
            if let Some(set) = self.anchor_kinds_by_subscriber.get_mut(&subscriber) {
                set.remove(&key);
            }
            // Clean up kinds_by_anchor if the set became empty.
            let anchor_set_empty = self
                .by_anchor_kind
                .get(&key)
                .map(|s| s.is_empty())
                .unwrap_or(true);
            if anchor_set_empty {
                if let Some(kinds) = self.kinds_by_anchor.get_mut(&anchor) {
                    kinds.remove(&kind);
                }
            }
        }
    }

    // =========================================================================
    // Scope-dispatched subscribe/unsubscribe
    // =========================================================================

    /// Subscribe using a typed scope. Idempotent.
    pub fn subscribe_scope(
        &mut self,
        subscriber: LocusId,
        scope: SubscriptionScope,
        batch: Option<BatchId>,
    ) {
        match scope {
            SubscriptionScope::Specific(rel_id) => {
                self.subscribe_at(subscriber, rel_id, batch)
            }
            SubscriptionScope::AllOfKind(kind) => self.subscribe_to_kind(subscriber, kind),
            SubscriptionScope::TouchingLocus { anchor, kind } => {
                self.subscribe_to_anchor_kind(subscriber, anchor, kind)
            }
        }
    }

    /// Unsubscribe using a typed scope. Idempotent.
    pub fn unsubscribe_scope(
        &mut self,
        subscriber: LocusId,
        scope: SubscriptionScope,
        batch: Option<BatchId>,
    ) {
        match scope {
            SubscriptionScope::Specific(rel_id) => {
                self.unsubscribe_at(subscriber, rel_id, batch)
            }
            SubscriptionScope::AllOfKind(kind) => {
                self.unsubscribe_from_kind(subscriber, kind)
            }
            SubscriptionScope::TouchingLocus { anchor, kind } => {
                self.unsubscribe_from_anchor_kind(subscriber, anchor, kind)
            }
        }
    }

    // =========================================================================
    // Lookup — used by the engine hot path
    // =========================================================================

    /// Iterate the loci subscribed to `rel_id` via the **Specific** scope.
    ///
    /// Returns an empty iterator if no loci are watching this relationship.
    pub fn subscribers(&self, rel_id: RelationshipId) -> impl Iterator<Item = LocusId> + '_ {
        self.by_relationship
            .get(&rel_id)
            .into_iter()
            .flat_map(|set| set.iter().copied())
    }

    /// `true` when at least one locus has a **Specific** subscription to `rel_id`.
    ///
    /// O(1). Called in the engine's inner commit loop for every relationship
    /// change — must stay fast.
    pub fn has_subscribers(&self, rel_id: RelationshipId) -> bool {
        self.by_relationship
            .get(&rel_id)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// `true` when at least one locus has an **AllOfKind** subscription for `kind`.
    pub fn has_kind_subscribers(&self, kind: InfluenceKindId) -> bool {
        self.by_kind
            .get(&kind)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Iterate loci subscribed to `kind` via the AllOfKind scope.
    pub fn kind_subscribers(
        &self,
        kind: InfluenceKindId,
    ) -> impl Iterator<Item = LocusId> + '_ {
        self.by_kind
            .get(&kind)
            .into_iter()
            .flat_map(|s| s.iter().copied())
    }

    /// `true` when at least one locus watches relationships of `kind` touching `anchor`.
    pub fn has_anchor_kind_subscribers(&self, anchor: LocusId, kind: InfluenceKindId) -> bool {
        self.by_anchor_kind
            .get(&(anchor, kind))
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Iterate loci subscribed to the `(anchor, kind)` scope.
    pub fn anchor_kind_subscribers(
        &self,
        anchor: LocusId,
        kind: InfluenceKindId,
    ) -> impl Iterator<Item = LocusId> + '_ {
        self.by_anchor_kind
            .get(&(anchor, kind))
            .into_iter()
            .flat_map(|s| s.iter().copied())
    }

    /// Fast check: does **any** scope have at least one subscriber for a
    /// relationship change on (`rel_id`, `kind`, `from` → `to`)?
    ///
    /// Called in the engine's inner loop (line 312 equivalent). O(1).
    pub fn has_any_subscribers(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        from: LocusId,
        to: LocusId,
    ) -> bool {
        self.has_subscribers(rel_id)
            || self.has_kind_subscribers(kind)
            || self.has_anchor_kind_subscribers(from, kind)
            || (from != to && self.has_anchor_kind_subscribers(to, kind))
    }

    /// Collect the deduplicated set of all subscribers for a relationship
    /// change on (`rel_id`, `kind`, `from` → `to`), across all three scopes.
    ///
    /// Allocates a `Vec` — call only when `has_any_subscribers` returned `true`.
    pub fn collect_subscribers(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        from: LocusId,
        to: LocusId,
    ) -> Vec<LocusId> {
        let mut seen: FxHashSet<LocusId> = FxHashSet::default();
        let mut out = Vec::new();
        for locus in self
            .subscribers(rel_id)
            .chain(self.kind_subscribers(kind))
            .chain(self.anchor_kind_subscribers(from, kind))
            .chain(self.anchor_kind_subscribers(to, kind))
        {
            if seen.insert(locus) {
                out.push(locus);
            }
        }
        out
    }

    // =========================================================================
    // Cleanup — called when loci or relationships are removed
    // =========================================================================

    /// Remove all **Specific** subscriptions pointing at `rel_id`.
    ///
    /// Called automatically when a relationship is deleted — either via
    /// `StructuralProposal::DeleteRelationship` or auto-pruning in
    /// `flush_relationship_decay`.
    ///
    /// Returns the set of loci that were watching `rel_id` so the caller
    /// (engine) can emit tombstone notifications before the relationship is
    /// actually removed from the store.
    pub fn remove_relationship(&mut self, rel_id: RelationshipId) -> Vec<LocusId> {
        if let Some(subs) = self.by_relationship.remove(&rel_id) {
            if !subs.is_empty() {
                self.generation += 1;
                self.total_count -= subs.len();
            }
            let notified: Vec<LocusId> = subs.iter().copied().collect();
            for locus in &subs {
                if let Some(rels) = self.by_locus.get_mut(locus) {
                    rels.remove(&rel_id);
                }
            }
            notified
        } else {
            Vec::new()
        }
    }

    /// Remove all subscriptions registered **by** `locus` (as a subscriber),
    /// across all three scopes.
    ///
    /// Call when a locus is deleted to avoid delivering notifications to a
    /// dangling ID.
    pub fn remove_locus(&mut self, locus: LocusId) {
        let mut changed = false;

        // Specific scope.
        if let Some(rels) = self.by_locus.remove(&locus) {
            if !rels.is_empty() {
                changed = true;
                self.total_count -= rels.len();
            }
            for rel_id in rels {
                if let Some(subs) = self.by_relationship.get_mut(&rel_id) {
                    subs.remove(&locus);
                }
            }
        }

        // AllOfKind scope.
        if let Some(kinds) = self.kinds_by_subscriber.remove(&locus) {
            if !kinds.is_empty() {
                changed = true;
            }
            for kind in kinds {
                if let Some(subs) = self.by_kind.get_mut(&kind) {
                    subs.remove(&locus);
                }
            }
        }

        // TouchingLocus scope.
        if let Some(keys) = self.anchor_kinds_by_subscriber.remove(&locus) {
            if !keys.is_empty() {
                changed = true;
            }
            for key @ (anchor, kind) in keys {
                if let Some(subs) = self.by_anchor_kind.get_mut(&key) {
                    subs.remove(&locus);
                }
                // Clean up kinds_by_anchor if set is now empty.
                let anchor_set_empty = self
                    .by_anchor_kind
                    .get(&key)
                    .map(|s| s.is_empty())
                    .unwrap_or(true);
                if anchor_set_empty {
                    if let Some(ks) = self.kinds_by_anchor.get_mut(&anchor) {
                        ks.remove(&kind);
                    }
                }
            }
        }

        if changed {
            self.generation += 1;
        }
    }

    /// Remove all TouchingLocus subscriptions where `anchor` was the anchor
    /// locus. Called when the anchor locus is deleted from the world.
    ///
    /// Note: this does NOT remove subscribers themselves — a subscriber that
    /// watched `(anchor, kind)` might still have other subscriptions. Only
    /// the entries keyed on `anchor` are cleaned.
    pub fn remove_anchor_locus(&mut self, anchor: LocusId) {
        let Some(kinds) = self.kinds_by_anchor.remove(&anchor) else {
            return;
        };
        if kinds.is_empty() {
            return;
        }
        self.generation += 1;
        for kind in kinds {
            let key = (anchor, kind);
            if let Some(subs) = self.by_anchor_kind.remove(&key) {
                for sub in subs {
                    if let Some(set) = self.anchor_kinds_by_subscriber.get_mut(&sub) {
                        set.remove(&key);
                    }
                }
            }
        }
    }

    // =========================================================================
    // Metadata
    // =========================================================================

    /// Monotonic generation counter — incremented on every mutation.
    ///
    /// Compare against a previously saved value to detect whether the
    /// subscription set has changed since the last persist. The initial
    /// value is `0`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `true` when no specific subscriptions are registered.
    pub fn is_empty(&self) -> bool {
        self.by_relationship.is_empty()
            && self.by_kind.is_empty()
            && self.by_anchor_kind.is_empty()
    }

    /// Number of **Specific**-scope (subscriber, rel_id) pairs currently registered.
    ///
    /// O(1) — cached.
    pub fn subscription_count(&self) -> usize {
        self.total_count
    }

    /// Number of **AllOfKind** (subscriber, kind) pairs currently registered.
    pub fn kind_subscription_count(&self) -> usize {
        self.by_kind.values().map(|s| s.len()).sum()
    }

    /// Iterate all (rel_id, subscriber) pairs for the Specific scope.
    ///
    /// Used by storage to persist the full subscription state.
    pub fn iter(&self) -> impl Iterator<Item = (RelationshipId, LocusId)> + '_ {
        self.by_relationship.iter().flat_map(|(&rel_id, subs)| {
            subs.iter().copied().map(move |locus_id| (rel_id, locus_id))
        })
    }

    // =========================================================================
    // Audit log
    // =========================================================================

    /// All Specific-scope subscription events in `[from_batch, to_batch)`.
    ///
    /// O(log N + k) — N = number of distinct batch keys in the audit log,
    /// k = number of matching events. Significantly faster than the previous
    /// O(N-total) linear scan.
    pub fn events_in_range(
        &self,
        from: BatchId,
        to: BatchId,
    ) -> impl Iterator<Item = &SubscriptionEvent> {
        self.audit_log
            .range(from.0..to.0)
            .flat_map(|(_, events)| events.iter())
    }

    /// Discard audit log entries older than `before_batch`.
    ///
    /// O(log N). Call alongside `ChangeLog::trim_before_batch` to keep
    /// `events_in_range` fast over long runs.
    pub fn trim_audit_before(&mut self, before_batch: BatchId) {
        self.audit_log = self.audit_log.split_off(&before_batch.0);
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
        let notified = store.remove_relationship(rel);
        assert!(!store.has_subscribers(rel));
        assert_eq!(store.subscription_count(), 0);
        // Both subscribers are returned for tombstone dispatch.
        let mut ids = notified;
        ids.sort_by_key(|l| l.0);
        assert_eq!(ids, vec![LocusId(1), LocusId(2)]);
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

    #[test]
    fn total_count_stays_correct_after_operations() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let r1 = RelationshipId(10);
        let r2 = RelationshipId(11);
        store.subscribe(locus, r1);
        store.subscribe(locus, r2);
        store.subscribe(LocusId(2), r1);
        assert_eq!(store.subscription_count(), 3);
        store.unsubscribe(locus, r1);
        assert_eq!(store.subscription_count(), 2);
        store.remove_relationship(r1);
        assert_eq!(store.subscription_count(), 1);
        store.remove_locus(locus);
        assert_eq!(store.subscription_count(), 0);
    }

    #[test]
    fn audit_log_btreemap_range_query() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let r1 = RelationshipId(10);
        let r2 = RelationshipId(11);
        store.subscribe_at(locus, r1, Some(BatchId(5)));
        store.subscribe_at(locus, r2, Some(BatchId(10)));
        store.subscribe_at(LocusId(2), r1, Some(BatchId(15)));
        // Range [5, 12) should return r1@5 and r2@10.
        let events: Vec<_> = store.events_in_range(BatchId(5), BatchId(12)).collect();
        assert_eq!(events.len(), 2);
        // Range [10, 20) should return r2@10 and the r1@15.
        let events2: Vec<_> = store.events_in_range(BatchId(10), BatchId(20)).collect();
        assert_eq!(events2.len(), 2);
    }

    #[test]
    fn trim_audit_before_splits_btreemap() {
        let mut store = SubscriptionStore::new();
        let locus = LocusId(1);
        let r = RelationshipId(10);
        store.subscribe_at(locus, r, Some(BatchId(3)));
        store.unsubscribe_at(locus, r, Some(BatchId(7)));
        store.subscribe_at(locus, r, Some(BatchId(12)));
        // Trim before batch 7: only the event at 3 is removed.
        store.trim_audit_before(BatchId(7));
        let after: Vec<_> = store.events_in_range(BatchId(0), BatchId(20)).collect();
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|e| e.batch.0 >= 7));
    }

    // ── Kind-level subscription tests ─────────────────────────────────────────

    #[test]
    fn subscribe_to_kind_and_has_kind_subscribers() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(1);
        let kind = InfluenceKindId(42);
        assert!(!store.has_kind_subscribers(kind));
        store.subscribe_to_kind(sub, kind);
        assert!(store.has_kind_subscribers(kind));
        let subs: Vec<LocusId> = store.kind_subscribers(kind).collect();
        assert_eq!(subs, vec![sub]);
    }

    #[test]
    fn subscribe_to_kind_is_idempotent() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(1);
        let kind = InfluenceKindId(42);
        store.subscribe_to_kind(sub, kind);
        let gen1 = store.generation();
        store.subscribe_to_kind(sub, kind);
        assert_eq!(store.generation(), gen1, "idempotent — no mutation");
    }

    #[test]
    fn unsubscribe_from_kind_works() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(1);
        let kind = InfluenceKindId(42);
        store.subscribe_to_kind(sub, kind);
        store.unsubscribe_from_kind(sub, kind);
        assert!(!store.has_kind_subscribers(kind));
    }

    #[test]
    fn subscribe_to_anchor_kind_and_lookup() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(99);
        let anchor = LocusId(5);
        let kind = InfluenceKindId(3);
        store.subscribe_to_anchor_kind(sub, anchor, kind);
        assert!(store.has_anchor_kind_subscribers(anchor, kind));
        assert!(!store.has_anchor_kind_subscribers(LocusId(6), kind));
        let subs: Vec<LocusId> = store.anchor_kind_subscribers(anchor, kind).collect();
        assert_eq!(subs, vec![sub]);
    }

    #[test]
    fn collect_subscribers_deduplicates() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(1);
        let rel = RelationshipId(10);
        let kind = InfluenceKindId(2);
        let from = LocusId(20);
        let to = LocusId(30);
        // Subscribe the same locus via two paths.
        store.subscribe(sub, rel);
        store.subscribe_to_kind(sub, kind);
        let subs = store.collect_subscribers(rel, kind, from, to);
        // Despite matching two scopes, sub appears only once.
        assert_eq!(subs, vec![sub]);
    }

    #[test]
    fn has_any_subscribers_covers_all_scopes() {
        let mut store = SubscriptionStore::new();
        let rel = RelationshipId(10);
        let kind = InfluenceKindId(2);
        let from = LocusId(20);
        let to = LocusId(30);
        assert!(!store.has_any_subscribers(rel, kind, from, to));
        // Kind subscriber only.
        store.subscribe_to_kind(LocusId(99), kind);
        assert!(store.has_any_subscribers(rel, kind, from, to));
    }

    #[test]
    fn remove_locus_cleans_kind_subscriptions() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(1);
        let kind = InfluenceKindId(7);
        let anchor = LocusId(5);
        store.subscribe_to_kind(sub, kind);
        store.subscribe_to_anchor_kind(sub, anchor, kind);
        store.remove_locus(sub);
        assert!(!store.has_kind_subscribers(kind));
        assert!(!store.has_anchor_kind_subscribers(anchor, kind));
    }

    #[test]
    fn remove_anchor_locus_cleans_anchor_entries() {
        let mut store = SubscriptionStore::new();
        let sub = LocusId(99);
        let anchor = LocusId(5);
        let kind = InfluenceKindId(3);
        store.subscribe_to_anchor_kind(sub, anchor, kind);
        store.remove_anchor_locus(anchor);
        assert!(!store.has_anchor_kind_subscribers(anchor, kind));
        // The subscriber itself is still alive — only the anchor entry is gone.
        assert!(store
            .anchor_kinds_by_subscriber
            .get(&sub)
            .map(|s| s.is_empty())
            .unwrap_or(true));
    }
}
