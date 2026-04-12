//! State- and property-based filtering of loci and relationships.
//!
//! All functions take `&World` and return `Vec` of references valid for
//! the lifetime of the world borrow. They are intentionally simple:
//! no builder pattern, no lazy iterators — just composable free functions
//! that can be chained by the caller.

use graph_core::{BatchId, Entity, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship, RelationshipId};
use graph_world::World;

// ─── ID-to-ref lookup helpers ─────────────────────────────────────────────────
//
// Traversal functions return `Vec<LocusId>` (structure-only, no allocation of
// locus data). Filter functions return `Vec<&Locus>` (full objects). When you
// need to bridge the two — e.g. filter a traversal result by state — use these
// lookup helpers rather than writing `ids.iter().filter_map(|id| world.locus(*id))`.

/// Resolve a slice of `LocusId`s to `&Locus` references, skipping any IDs not
/// present in the world (e.g. stale IDs from a previous snapshot).
///
/// Preserves the order of the input slice.
pub fn lookup_loci<'w>(world: &'w World, ids: &[LocusId]) -> Vec<&'w Locus> {
    ids.iter().filter_map(|&id| world.locus(id)).collect()
}

/// Resolve a slice of `RelationshipId`s to `&Relationship` references, skipping
/// any IDs not present in the world (e.g. cold-evicted or deleted relationships).
///
/// Preserves the order of the input slice.
pub fn lookup_relationships<'w>(world: &'w World, ids: &[RelationshipId]) -> Vec<&'w Relationship> {
    ids.iter()
        .filter_map(|&id| world.relationships().get(id))
        .collect()
}

// ─── Locus degree metrics ─────────────────────────────────────────────────────

/// Total relationship degree of `locus` — number of edges in any direction.
///
/// Counts both `Directed` and `Symmetric` edges. A `Directed(A→B)` edge
/// counts once toward both A's and B's degree.
///
/// Returns 0 for loci with no relationships (including loci that don't exist).
pub fn locus_degree(world: &World, locus: LocusId) -> usize {
    world.degree(locus)
}

/// Number of directed edges **arriving at** `locus` (`Directed { to == locus }`).
///
/// Symmetric edges are not counted.
pub fn locus_in_degree(world: &World, locus: LocusId) -> usize {
    world.in_degree(locus)
}

/// Number of directed edges **leaving** `locus` (`Directed { from == locus }`).
///
/// Symmetric edges are not counted.
pub fn locus_out_degree(world: &World, locus: LocusId) -> usize {
    world.out_degree(locus)
}

/// The top-N loci ranked by total relationship degree (descending).
///
/// Loci with degree 0 are excluded. Returns fewer than `n` entries when
/// fewer than `n` loci have at least one edge.
pub fn most_connected_loci(world: &World, n: usize) -> Vec<LocusId> {
    most_connected_loci_with_degree(world, n)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// Like `most_connected_loci`, but returns `(LocusId, degree)` pairs so the
/// caller doesn't need a second `locus_degree` call per entry.
///
/// Sorted by degree descending. Loci with degree 0 are excluded.
pub fn most_connected_loci_with_degree(world: &World, n: usize) -> Vec<(LocusId, usize)> {
    if n == 0 {
        return Vec::new();
    }
    let mut by_degree: Vec<(LocusId, usize)> = world.degree_iter().collect();
    by_degree.sort_unstable_by_key(|&(_, d)| std::cmp::Reverse(d));
    by_degree.truncate(n);
    by_degree
}

/// The top-N loci ranked by `state[slot]` in descending order.
///
/// Loci whose state vector is shorter than `slot + 1` are excluded.
/// Useful for finding the "most active", "most convinced", or otherwise
/// highest-valued loci for any numeric state slot.
pub fn loci_top_n_by_state(world: &World, slot: usize, n: usize) -> Vec<&Locus> {
    if n == 0 {
        return Vec::new();
    }
    let mut loci: Vec<&Locus> = world
        .loci()
        .iter()
        .filter(|l| l.state.as_slice().len() > slot)
        .collect();
    loci.sort_unstable_by(|a, b| {
        let va = a.state.as_slice()[slot];
        let vb = b.state.as_slice()[slot];
        vb.total_cmp(&va)
    });
    loci.truncate(n);
    loci
}

// ─── Locus filters ────────────────────────────────────────────────────────────

/// All loci of a specific kind.
pub fn loci_of_kind(world: &World, kind: LocusKindId) -> Vec<&Locus> {
    world.loci().iter().filter(|l| l.kind == kind).collect()
}

/// All loci whose `state[slot]` satisfies `pred`.
///
/// Loci with a state vector shorter than `slot + 1` are excluded.
pub fn loci_with_state<F>(world: &World, slot: usize, pred: F) -> Vec<&Locus>
where
    F: Fn(f32) -> bool,
{
    world
        .loci()
        .iter()
        .filter(|l| l.state.as_slice().get(slot).is_some_and(|&v| pred(v)))
        .collect()
}

/// All loci that have a string property `key` satisfying `pred`.
pub fn loci_with_str_property<'w, F>(world: &'w World, key: &str, pred: F) -> Vec<&'w Locus>
where
    F: Fn(&str) -> bool,
{
    world
        .loci()
        .iter()
        .filter(|l| {
            world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_str(key))
                .is_some_and(&pred)
        })
        .collect()
}

/// All loci that have a numeric (f64) property `key` satisfying `pred`.
pub fn loci_with_f64_property<'w, F>(world: &'w World, key: &str, pred: F) -> Vec<&'w Locus>
where
    F: Fn(f64) -> bool,
{
    world
        .loci()
        .iter()
        .filter(|l| {
            world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_f64(key))
                .is_some_and(&pred)
        })
        .collect()
}

/// All loci matching a custom predicate over the `Locus` itself.
///
/// Use this when none of the typed helpers cover your case.
pub fn loci_matching<F>(world: &World, pred: F) -> Vec<&Locus>
where
    F: Fn(&Locus) -> bool,
{
    world.loci().iter().filter(|l| pred(l)).collect()
}

// ─── Relationship filters ─────────────────────────────────────────────────────

/// All relationships of a specific influence kind.
pub fn relationships_of_kind(world: &World, kind: InfluenceKindId) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .collect()
}

/// All relationships whose activity (`state[0]`) satisfies `pred`.
pub fn relationships_with_activity<F>(world: &World, pred: F) -> Vec<&Relationship>
where
    F: Fn(f32) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| pred(r.activity()))
        .collect()
}

/// All relationships whose Hebbian weight (`state[1]`) satisfies `pred`.
pub fn relationships_with_weight<F>(world: &World, pred: F) -> Vec<&Relationship>
where
    F: Fn(f32) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| pred(r.weight()))
        .collect()
}

/// All relationships whose extra slot at `slot_idx` satisfies `pred`.
///
/// Slot index 2 onwards are user-defined extra slots. Relationships with
/// a state vector shorter than `slot_idx + 1` are excluded.
pub fn relationships_with_slot<F>(world: &World, slot_idx: usize, pred: F) -> Vec<&Relationship>
where
    F: Fn(f32) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| {
            r.state
                .as_slice()
                .get(slot_idx)
                .is_some_and(|&v| pred(v))
        })
        .collect()
}

/// All relationships matching a custom predicate.
pub fn relationships_matching<F>(world: &World, pred: F) -> Vec<&Relationship>
where
    F: Fn(&Relationship) -> bool,
{
    world.relationships().iter().filter(|r| pred(r)).collect()
}

/// All relationships that have a string metadata property `key` satisfying `pred`.
///
/// Relationships without `metadata`, or where `key` is absent or not a string,
/// are silently excluded.
///
/// # Example
/// ```ignore
/// let trusted = relationships_with_str_property(world, "type", |v| v == "trust");
/// ```
pub fn relationships_with_str_property<'w, F>(
    world: &'w World,
    key: &str,
    pred: F,
) -> Vec<&'w Relationship>
where
    F: Fn(&str) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| r.get_str_property(key).is_some_and(&pred))
        .collect()
}

/// All relationships that have a numeric (f64) metadata property `key` satisfying `pred`.
///
/// Relationships without `metadata`, or where `key` is absent or not numeric,
/// are silently excluded.
///
/// # Example
/// ```ignore
/// let confident = relationships_with_f64_property(world, "confidence", |v| v >= 0.8);
/// ```
pub fn relationships_with_f64_property<'w, F>(
    world: &'w World,
    key: &str,
    pred: F,
) -> Vec<&'w Relationship>
where
    F: Fn(f64) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| r.get_f64_property(key).is_some_and(&pred))
        .collect()
}

// ─── Lifecycle filters (created_batch-based) ────────────────────────────────

/// All relationships created at or after `from_batch` and at or before `to_batch`.
///
/// Uses `Relationship::created_batch` directly — does not depend on the change log
/// and survives log trimming. Structurally-created relationships are included.
pub fn relationships_created_in<'w>(
    world: &'w World,
    from: BatchId,
    to: BatchId,
) -> Vec<&'w Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.created_batch >= from && r.created_batch <= to)
        .collect()
}

/// All relationships whose age (`current_batch − created_batch`) is at least `min_batches`.
pub fn relationships_older_than(
    world: &World,
    current_batch: BatchId,
    min_batches: u64,
) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.age_in_batches(current_batch) >= min_batches)
        .collect()
}

// ─── Influence aggregation ────────────────────────────────────────────────────

/// Sum of `activity` across all directed relationships arriving **at** `locus`.
///
/// Represents the total causal pressure flowing into this locus.
/// Symmetric edges are excluded (they have no directionality).
pub fn incoming_activity_sum(world: &World, locus: LocusId) -> f32 {
    world.relationships_to(locus).map(|r| r.activity()).sum()
}

/// Sum of `activity` across all directed relationships leaving `locus`.
///
/// Represents the total causal signal this locus is emitting.
/// Symmetric edges are excluded.
pub fn outgoing_activity_sum(world: &World, locus: LocusId) -> f32 {
    world.relationships_from(locus).map(|r| r.activity()).sum()
}

/// Net influence balance: `outgoing_activity_sum − incoming_activity_sum`.
///
/// Positive → net sender (more outgoing influence than incoming).
/// Negative → net receiver.
/// Zero → balanced or isolated.
pub fn net_influence_balance(world: &World, locus: LocusId) -> f32 {
    outgoing_activity_sum(world, locus) - incoming_activity_sum(world, locus)
}

// ─── Change-count / velocity filters ─────────────────────────────────────────

/// All relationships whose cumulative `change_count` is at least `min_count`.
///
/// `change_count` counts how many times the engine has touched (incremented
/// activity on) this relationship. High counts indicate structurally stable,
/// frequently-reinforced edges.
pub fn relationships_by_change_count(world: &World, min_count: u64) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.lineage.change_count >= min_count)
        .collect()
}

/// The top-N relationships ranked by `change_count` in descending order.
pub fn most_changed_relationships(world: &World, n: usize) -> Vec<&Relationship> {
    if n == 0 {
        return Vec::new();
    }
    let mut all: Vec<&Relationship> = world.relationships().iter().collect();
    all.sort_unstable_by_key(|r| std::cmp::Reverse(r.lineage.change_count));
    all.truncate(n);
    all
}

/// All relationships whose combined strength (`activity + weight`) exceeds `threshold`.
pub fn relationships_above_strength(world: &World, threshold: f32) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.strength() > threshold)
        .collect()
}

/// The top-N relationships ranked by strength (`activity + weight`) in descending order.
///
/// Returns fewer than `n` entries when the world has fewer than `n` relationships.
pub fn relationships_top_n_by_strength(world: &World, n: usize) -> Vec<&Relationship> {
    if n == 0 {
        return Vec::new();
    }
    let mut all: Vec<&Relationship> = world.relationships().iter().collect();
    all.sort_unstable_by(|a, b| b.strength().total_cmp(&a.strength()));
    all.truncate(n);
    all
}

/// Average number of engine touches per batch for a relationship, measured over
/// its entire lifetime.
///
/// Computed as `change_count / age_in_batches`. Returns `0.0` when the
/// relationship is 0 batches old (just created this batch) or not found.
///
/// Unlike `relationship_volatility`, this metric is meaningful for
/// **auto-emerged** relationships (which have no `ChangeSubject::Relationship`
/// log entries) because it is derived entirely from `Relationship::lineage.change_count`
/// and `created_batch` — both present on every relationship regardless of how
/// it was created.
///
/// - High rate → relationship is being actively reinforced every batch.
/// - Low rate → relationship was born from a burst and has since gone quiet.
pub fn relationship_touch_rate(world: &World, rel_id: RelationshipId, current_batch: BatchId) -> f32 {
    let Some(rel) = world.relationships().get(rel_id) else { return 0.0 };
    let age = rel.age_in_batches(current_batch);
    if age == 0 { return 0.0; }
    rel.lineage.change_count as f32 / age as f32
}

/// Relationships that have not been touched (decayed) for at least `min_idle_batches`
/// batches relative to `current_batch`.
///
/// A relationship is "idle" when `current_batch - last_decayed_batch >= min_idle_batches`.
/// These are candidates for cold eviction.
pub fn relationships_idle_for(
    world: &World,
    current_batch: BatchId,
    min_idle_batches: u64,
) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| {
            current_batch.0.saturating_sub(r.last_decayed_batch) >= min_idle_batches
        })
        .collect()
}

/// All directed relationships that originate **from** `locus`
/// (`Directed { from == locus }`). Symmetric edges are excluded.
pub fn relationships_from(world: &World, locus: LocusId) -> Vec<&Relationship> {
    world.relationships_from(locus).collect()
}

/// Directed outgoing relationships of a specific kind from `locus`.
/// Symmetric edges are excluded.
pub fn relationships_from_of_kind(world: &World, locus: LocusId, kind: InfluenceKindId) -> Vec<&Relationship> {
    world.relationships_from_of_kind(locus, kind).collect()
}

/// All directed relationships that arrive **at** `locus`
/// (`Directed { to == locus }`). Symmetric edges are excluded.
pub fn relationships_to(world: &World, locus: LocusId) -> Vec<&Relationship> {
    world.relationships_to(locus).collect()
}

/// Directed incoming relationships of a specific kind to `locus`.
/// Symmetric edges are excluded.
pub fn relationships_to_of_kind(world: &World, locus: LocusId, kind: InfluenceKindId) -> Vec<&Relationship> {
    world.relationships_to_of_kind(locus, kind).collect()
}

/// All relationships whose endpoints include both `a` and `b`,
/// across all kinds and directions. O(k_a).
pub fn relationships_between(world: &World, a: LocusId, b: LocusId) -> Vec<&Relationship> {
    world.relationships_between(a, b).collect()
}

/// All relationships between `a` and `b` of a specific influence kind.
pub fn relationships_between_of_kind(
    world: &World,
    a: LocusId,
    b: LocusId,
    kind: InfluenceKindId,
) -> Vec<&Relationship> {
    world.relationships_between_of_kind(a, b, kind).collect()
}

// ─── Entity filters ───────────────────────────────────────────────────────────

/// All currently active entities.
pub fn active_entities(world: &World) -> Vec<&Entity> {
    world.entities().active().collect()
}

/// All active entities whose current member set contains `locus`.
pub fn entities_with_member(world: &World, locus: LocusId) -> Vec<&Entity> {
    world
        .entities()
        .active()
        .filter(|e| e.current.members.contains(&locus))
        .collect()
}

/// All active entities whose coherence score satisfies `pred`.
pub fn entities_with_coherence<F>(world: &World, pred: F) -> Vec<&Entity>
where
    F: Fn(f32) -> bool,
{
    world
        .entities()
        .active()
        .filter(|e| pred(e.current.coherence))
        .collect()
}

/// All active entities matching a custom predicate over the `Entity` itself.
pub fn entities_matching<F>(world: &World, pred: F) -> Vec<&Entity>
where
    F: Fn(&Entity) -> bool,
{
    world.entities().active().filter(|e| pred(e)).collect()
}

// ─── Cross-layer queries ──────────────────────────────────────────────────────
//
// Bridge Layer 0 (Locus) ↔ Layer 3 (Entity). These are the natural seams
// where the emergent ontology connects: entities are coherent bundles of loci,
// and a locus can be a member of multiple active entities.

/// Resolve the member loci of an entity, returning `&Locus` references.
///
/// Returns only loci that still exist in the world (stale member IDs are
/// silently skipped). The ordering matches `entity.current.members`.
pub fn entity_member_loci<'w>(world: &'w World, entity: &graph_core::Entity) -> Vec<&'w Locus> {
    entity.current.members.iter()
        .filter_map(|&id| world.locus(id))
        .collect()
}

/// All active entities that currently contain `locus` as a member.
///
/// A locus can belong to more than one active entity when two overlapping
/// bundles both claim it.
pub fn locus_entities<'w>(world: &'w World, locus: LocusId) -> Vec<&'w graph_core::Entity> {
    world.entities().active()
        .filter(|e| e.current.members.contains(&locus))
        .collect()
}

/// All member loci of the top-N entities ranked by coherence (descending).
///
/// Returns a flat `Vec<&Locus>` deduplicated by locus ID. Useful for
/// identifying the most structurally significant loci in the world.
pub fn top_entity_members(world: &World, n: usize) -> Vec<&Locus> {
    let mut entities: Vec<&graph_core::Entity> = world.entities().active().collect();
    entities.sort_unstable_by(|a, b| b.current.coherence.partial_cmp(&a.current.coherence).unwrap_or(std::cmp::Ordering::Equal));

    let mut seen = rustc_hash::FxHashSet::default();
    let mut result = Vec::new();
    for entity in entities.into_iter().take(n) {
        for &locus_id in &entity.current.members {
            if seen.insert(locus_id) {
                if let Some(l) = world.locus(locus_id) {
                    result.push(l);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, Locus, LocusKindId, Relationship, RelationshipKindId,
        RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn make_world() -> World {
        let lk_a = LocusKindId(1);
        let lk_b = LocusKindId(2);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();

        w.insert_locus(Locus::new(graph_core::LocusId(0), lk_a, StateVector::from_slice(&[0.9])));
        w.insert_locus(Locus::new(graph_core::LocusId(1), lk_a, StateVector::from_slice(&[0.3])));
        w.insert_locus(Locus::new(graph_core::LocusId(2), lk_b, StateVector::from_slice(&[0.7])));

        w.properties_mut().insert(graph_core::LocusId(0), graph_core::props! {
            "type" => "ORG",
            "score" => 0.9_f64,
        });
        w.properties_mut().insert(graph_core::LocusId(1), graph_core::props! {
            "type" => "PERSON",
            "score" => 0.3_f64,
        });

        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Directed {
                from: graph_core::LocusId(0),
                to: graph_core::LocusId(1),
            },
            state: StateVector::from_slice(&[0.8, 0.5, 0.2]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: vec![rk],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        w
    }

    #[test]
    fn loci_of_kind_filters() {
        let w = make_world();
        let kind_a = loci_of_kind(&w, LocusKindId(1));
        assert_eq!(kind_a.len(), 2);
        let kind_b = loci_of_kind(&w, LocusKindId(2));
        assert_eq!(kind_b.len(), 1);
    }

    #[test]
    fn loci_with_state_filters_by_slot() {
        let w = make_world();
        let high = loci_with_state(&w, 0, |v| v > 0.5);
        assert_eq!(high.len(), 2); // 0.9 and 0.7
        let low = loci_with_state(&w, 0, |v| v < 0.5);
        assert_eq!(low.len(), 1); // 0.3
    }

    #[test]
    fn loci_with_str_property_filters() {
        let w = make_world();
        let orgs = loci_with_str_property(&w, "type", |v| v == "ORG");
        assert_eq!(orgs.len(), 1);
        assert_eq!(orgs[0].id, graph_core::LocusId(0));
    }

    #[test]
    fn loci_with_f64_property_filters() {
        let w = make_world();
        let high_score = loci_with_f64_property(&w, "score", |v| v > 0.5);
        assert_eq!(high_score.len(), 1);
    }

    #[test]
    fn relationships_with_activity_filters() {
        let w = make_world();
        let active = relationships_with_activity(&w, |a| a > 0.5);
        assert_eq!(active.len(), 1);
        let none = relationships_with_activity(&w, |a| a > 0.9);
        assert!(none.is_empty());
    }

    #[test]
    fn relationships_with_slot_filters_extra_slot() {
        let w = make_world();
        // slot_idx=2 is the first extra slot, value 0.2
        let found = relationships_with_slot(&w, 2, |v| v > 0.1);
        assert_eq!(found.len(), 1);
        let not_found = relationships_with_slot(&w, 2, |v| v > 0.5);
        assert!(not_found.is_empty());
    }

    #[test]
    fn relationships_of_kind_filters() {
        let w = make_world();
        let rk = InfluenceKindId(1);
        assert_eq!(relationships_of_kind(&w, rk).len(), 1);
        assert!(relationships_of_kind(&w, InfluenceKindId(99)).is_empty());
    }

    // ── Entity filter tests ──────────────────────────────────────────────

    fn make_world_with_entities() -> World {
        use graph_core::{BatchId, Entity, EntityId, EntitySnapshot, LocusId};
        let mut w = World::new();
        let lk = LocusKindId(1);
        w.insert_locus(Locus::new(LocusId(0), lk, StateVector::from_slice(&[0.5])));
        w.insert_locus(Locus::new(LocusId(1), lk, StateVector::from_slice(&[0.8])));
        w.insert_locus(Locus::new(LocusId(2), lk, StateVector::from_slice(&[0.3])));

        let e0 = w.entities_mut().mint_id();
        w.entities_mut().insert(Entity::born(
            e0,
            BatchId(1),
            EntitySnapshot { members: vec![LocusId(0), LocusId(1)], member_relationships: vec![], coherence: 0.8 },
        ));
        let e1 = w.entities_mut().mint_id();
        w.entities_mut().insert(Entity::born(
            e1,
            BatchId(2),
            EntitySnapshot { members: vec![LocusId(2)], member_relationships: vec![], coherence: 0.2 },
        ));
        w
    }

    #[test]
    fn active_entities_returns_all() {
        let w = make_world_with_entities();
        assert_eq!(active_entities(&w).len(), 2);
    }

    #[test]
    fn entities_with_member_finds_correct_entity() {
        let w = make_world_with_entities();
        use graph_core::LocusId;
        let found = entities_with_member(&w, LocusId(1));
        assert_eq!(found.len(), 1);
        assert!(found[0].current.members.contains(&LocusId(1)));

        let not_found = entities_with_member(&w, LocusId(99));
        assert!(not_found.is_empty());
    }

    #[test]
    fn entities_with_coherence_filters() {
        let w = make_world_with_entities();
        let high = entities_with_coherence(&w, |c| c > 0.5);
        assert_eq!(high.len(), 1);
        assert!((high[0].current.coherence - 0.8).abs() < 1e-5);

        let low = entities_with_coherence(&w, |c| c <= 0.5);
        assert_eq!(low.len(), 1);
    }

    #[test]
    fn entities_matching_custom_pred() {
        let w = make_world_with_entities();
        let large = entities_matching(&w, |e| e.current.members.len() >= 2);
        assert_eq!(large.len(), 1);
        assert_eq!(large[0].current.members.len(), 2);
    }

    // ── Directed relationship filter tests ──────────────────────────────────

    fn directed_world() -> World {
        use graph_core::{Endpoints, LocusId};
        let mut w = World::new();
        // L0→L1 kind 1, L2→L1 kind 1, L0→L2 kind 2
        w.add_relationship(Endpoints::directed(LocusId(0), LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0, 0.0]));
        w.add_relationship(Endpoints::directed(LocusId(2), LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0, 0.0]));
        w.add_relationship(Endpoints::directed(LocusId(0), LocusId(2)), InfluenceKindId(2), StateVector::from_slice(&[1.0, 0.0]));
        w
    }

    #[test]
    fn relationships_from_returns_outgoing_only() {
        use graph_core::LocusId;
        let w = directed_world();
        // L0 has two outgoing edges (to L1 and L2)
        let out = relationships_from(&w, LocusId(0));
        assert_eq!(out.len(), 2);
        // L1 has no outgoing edges
        let none = relationships_from(&w, LocusId(1));
        assert!(none.is_empty());
    }

    #[test]
    fn relationships_to_returns_incoming_only() {
        use graph_core::LocusId;
        let w = directed_world();
        // L1 has two incoming edges
        let inc = relationships_to(&w, LocusId(1));
        assert_eq!(inc.len(), 2);
        // L0 has no incoming edges
        let none = relationships_to(&w, LocusId(0));
        assert!(none.is_empty());
    }

    #[test]
    fn relationships_between_returns_all_kinds() {
        use graph_core::LocusId;
        let w = directed_world();
        // L0→L1 (kind 1): one edge between L0 and L1
        let between01 = relationships_between(&w, LocusId(0), LocusId(1));
        assert_eq!(between01.len(), 1);
        // L0→L2 (kind 2): one edge between L0 and L2
        let between02 = relationships_between(&w, LocusId(0), LocusId(2));
        assert_eq!(between02.len(), 1);
        // L2→L1 (kind 1): one edge between L1 and L2 (direction-agnostic)
        let between12 = relationships_between(&w, LocusId(1), LocusId(2));
        assert_eq!(between12.len(), 1);
        // No edge between L3 and L0
        let none = relationships_between(&w, LocusId(0), LocusId(99));
        assert!(none.is_empty());
    }

    #[test]
    fn relationships_between_of_kind_filters_kind() {
        use graph_core::LocusId;
        // Add a second edge between L0→L1 of kind 2
        let mut w = directed_world();
        w.add_relationship(
            graph_core::Endpoints::directed(LocusId(0), LocusId(1)),
            InfluenceKindId(2),
            StateVector::from_slice(&[1.0, 0.0]),
        );
        let kind1 = relationships_between_of_kind(&w, LocusId(0), LocusId(1), InfluenceKindId(1));
        assert_eq!(kind1.len(), 1);
        let kind2 = relationships_between_of_kind(&w, LocusId(0), LocusId(1), InfluenceKindId(2));
        assert_eq!(kind2.len(), 1);
        let kind99 = relationships_between_of_kind(&w, LocusId(0), LocusId(1), InfluenceKindId(99));
        assert!(kind99.is_empty());
    }

    // ── Strength / lifecycle filter tests ────────────────────────────────────

    fn strength_world() -> World {
        use graph_core::{Endpoints, LocusId};
        let mut w = World::new();
        // Three relationships: (activity=0.8, weight=0.2) → strength=1.0
        //                      (activity=0.3, weight=0.1) → strength=0.4
        //                      (activity=0.5, weight=0.6) → strength=1.1
        let rk = InfluenceKindId(1);
        for i in 0u64..4 {
            w.insert_locus(graph_core::Locus::new(
                LocusId(i),
                LocusKindId(1),
                StateVector::from_slice(&[0.5]),
            ));
        }
        w.add_relationship(Endpoints::directed(LocusId(0), LocusId(1)), rk, StateVector::from_slice(&[0.8, 0.2]));
        w.add_relationship(Endpoints::directed(LocusId(1), LocusId(2)), rk, StateVector::from_slice(&[0.3, 0.1]));
        w.add_relationship(Endpoints::directed(LocusId(2), LocusId(3)), rk, StateVector::from_slice(&[0.5, 0.6]));
        w
    }

    #[test]
    fn relationships_above_strength_filters() {
        let w = strength_world();
        let above = relationships_above_strength(&w, 1.0);
        assert_eq!(above.len(), 1); // only strength=1.1
        assert!((above[0].strength() - 1.1).abs() < 1e-5);

        let none = relationships_above_strength(&w, 2.0);
        assert!(none.is_empty());
    }

    #[test]
    fn relationships_top_n_by_strength_is_sorted() {
        let w = strength_world();
        let top2 = relationships_top_n_by_strength(&w, 2);
        assert_eq!(top2.len(), 2);
        // Descending: 1.1, 1.0
        assert!(top2[0].strength() >= top2[1].strength());
        assert!((top2[0].strength() - 1.1).abs() < 1e-5);
        assert!((top2[1].strength() - 1.0).abs() < 1e-5);

        let all = relationships_top_n_by_strength(&w, 100);
        assert_eq!(all.len(), 3);

        let zero = relationships_top_n_by_strength(&w, 0);
        assert!(zero.is_empty());
    }

    #[test]
    fn relationships_idle_for_filters_by_last_decayed_batch() {
        use graph_core::BatchId;
        let w = strength_world();
        // All relationships have last_decayed_batch = 0 (default). At batch 10,
        // idle_for >= 5 should catch all three.
        let idle = relationships_idle_for(&w, BatchId(10), 5);
        assert_eq!(idle.len(), 3);
        // idle_for >= 11 should catch none (10 - 0 = 10, not >= 11).
        let none = relationships_idle_for(&w, BatchId(10), 11);
        assert!(none.is_empty());
    }

    // ── lookup_loci / lookup_relationships ──────────────────────────────────

    #[test]
    fn lookup_loci_resolves_ids_to_references() {
        use graph_core::LocusId;
        let w = make_world();
        let ids = vec![LocusId(0), LocusId(2), LocusId(99)]; // 99 doesn't exist
        let loci = lookup_loci(&w, &ids);
        assert_eq!(loci.len(), 2); // 99 skipped
        assert_eq!(loci[0].id, LocusId(0));
        assert_eq!(loci[1].id, LocusId(2));
    }

    #[test]
    fn lookup_relationships_resolves_ids() {
        use graph_core::{RelationshipId, LocusId};
        let w = directed_world();
        // directed_world has 3 relationships; get all IDs then look them up
        let ids: Vec<RelationshipId> = w.relationships().iter().map(|r| r.id).collect();
        let rels = lookup_relationships(&w, &ids);
        assert_eq!(rels.len(), ids.len());
        // Non-existent ID is skipped
        let with_bad = lookup_relationships(&w, &[RelationshipId(999)]);
        assert!(with_bad.is_empty());
    }

    // ── relationship_touch_rate ──────────────────────────────────────────────

    #[test]
    fn relationship_touch_rate_is_zero_for_new_relationship() {
        use graph_core::{BatchId, LocusId};
        let w = directed_world();
        let rel = w.relationships().iter().next().unwrap();
        // Created at batch 0, current_batch = 0 → age = 0
        assert_eq!(relationship_touch_rate(&w, rel.id, BatchId(0)), 0.0);
    }

    #[test]
    fn relationship_touch_rate_scales_with_touches() {
        use graph_core::{BatchId, LocusId};
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        w.insert_locus(graph_core::Locus::new(LocusId(0), LocusKindId(1), StateVector::from_slice(&[0.5])));
        w.insert_locus(graph_core::Locus::new(LocusId(1), LocusKindId(1), StateVector::from_slice(&[0.5])));

        use graph_core::{Endpoints, Relationship, RelationshipLineage};
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::directed(LocusId(0), LocusId(1)),
            state: StateVector::from_slice(&[0.5, 0.0]),
            lineage: RelationshipLineage {
                created_by: None, last_touched_by: None,
                change_count: 6,  // touched 6 times
                kinds_observed: vec![rk],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        // age = 12 - 0 = 12, change_count = 6 → rate = 0.5
        let rate = relationship_touch_rate(&w, id, BatchId(12));
        assert!((rate - 0.5).abs() < 1e-5, "expected 0.5, got {rate}");
    }

    // ── Degree metric tests ──────────────────────────────────────────────────

    fn degree_world() -> World {
        use graph_core::{Endpoints, LocusId};
        // L0 → L1, L0 → L2, L3 → L1 (L4 isolated)
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        for i in 0u64..5 {
            w.insert_locus(graph_core::Locus::new(LocusId(i), LocusKindId(1), StateVector::from_slice(&[0.5])));
        }
        w.add_relationship(Endpoints::directed(LocusId(0), LocusId(1)), rk, StateVector::from_slice(&[1.0, 0.0]));
        w.add_relationship(Endpoints::directed(LocusId(0), LocusId(2)), rk, StateVector::from_slice(&[1.0, 0.0]));
        w.add_relationship(Endpoints::directed(LocusId(3), LocusId(1)), rk, StateVector::from_slice(&[1.0, 0.0]));
        w
    }

    #[test]
    fn locus_degree_counts_all_edges() {
        use graph_core::LocusId;
        let w = degree_world();
        // L0: 2 outgoing
        assert_eq!(locus_degree(&w, LocusId(0)), 2);
        // L1: 2 incoming
        assert_eq!(locus_degree(&w, LocusId(1)), 2);
        // L4: isolated
        assert_eq!(locus_degree(&w, LocusId(4)), 0);
    }

    #[test]
    fn locus_in_out_degree_are_directional() {
        use graph_core::LocusId;
        let w = degree_world();
        assert_eq!(locus_in_degree(&w, LocusId(0)), 0);
        assert_eq!(locus_out_degree(&w, LocusId(0)), 2);
        assert_eq!(locus_in_degree(&w, LocusId(1)), 2);
        assert_eq!(locus_out_degree(&w, LocusId(1)), 0);
    }

    #[test]
    fn most_connected_loci_returns_top_n_by_degree() {
        use graph_core::LocusId;
        let w = degree_world();
        // degrees: L0=2, L1=2, L2=1, L3=1; L4=0 excluded
        let top1 = most_connected_loci(&w, 1);
        assert_eq!(top1.len(), 1);
        // degree 2 loci: L0 and L1; either can be first
        assert!(top1[0] == LocusId(0) || top1[0] == LocusId(1));

        let top4 = most_connected_loci(&w, 4);
        assert_eq!(top4.len(), 4); // L0, L1, L2, L3 (L4 excluded)

        let zero = most_connected_loci(&w, 0);
        assert!(zero.is_empty());
    }

    // ─── Relationship metadata property filters ──────────────────────────────

    fn world_with_metadata_rels() -> World {
        use graph_core::{Endpoints, InfluenceKindId, LocusId, Properties, StateVector};
        let mut w = World::new();
        let a = LocusId(0);
        let b = LocusId(1);
        let c = LocusId(2);
        let kind = InfluenceKindId(1);
        // Relationship A→B with metadata type="trust", confidence=0.9
        let rel_ab = w.add_relationship(
            Endpoints::directed(a, b),
            kind,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        {
            let mut props = Properties::new();
            props.set("type", "trust");
            props.set("confidence", 0.9f64);
            w.relationships_mut().get_mut(rel_ab).unwrap().metadata = Some(props);
        }
        // Relationship B→C with metadata type="inhibit", confidence=0.4
        let rel_bc = w.add_relationship(
            Endpoints::directed(b, c),
            kind,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        {
            let mut props = Properties::new();
            props.set("type", "inhibit");
            props.set("confidence", 0.4f64);
            w.relationships_mut().get_mut(rel_bc).unwrap().metadata = Some(props);
        }
        // Relationship A→C with no metadata
        let _rel_ac = w.add_relationship(
            Endpoints::directed(a, c),
            kind,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        w
    }

    #[test]
    fn relationships_with_str_property_filters_by_type() {
        let w = world_with_metadata_rels();
        let trust = relationships_with_str_property(&w, "type", |v| v == "trust");
        assert_eq!(trust.len(), 1);
        assert_eq!(trust[0].get_str_property("type"), Some("trust"));

        let inhibit = relationships_with_str_property(&w, "type", |v| v == "inhibit");
        assert_eq!(inhibit.len(), 1);

        // Relationship without metadata is excluded
        let all = relationships_with_str_property(&w, "type", |_| true);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn relationships_with_f64_property_filters_by_confidence() {
        let w = world_with_metadata_rels();
        let high = relationships_with_f64_property(&w, "confidence", |v| v >= 0.8);
        assert_eq!(high.len(), 1);
        assert!((high[0].get_f64_property("confidence").unwrap() - 0.9).abs() < 1e-9);

        let low = relationships_with_f64_property(&w, "confidence", |v| v < 0.5);
        assert_eq!(low.len(), 1);
    }

    #[test]
    fn relationships_metadata_absent_excluded() {
        let w = world_with_metadata_rels();
        // Only relationships with the "type" key present are returned
        let typed = relationships_with_str_property(&w, "type", |_| true);
        assert_eq!(typed.len(), 2); // A→C has no metadata
    }
}
