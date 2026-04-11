//! State- and property-based filtering of loci and relationships.
//!
//! All functions take `&World` and return `Vec` of references valid for
//! the lifetime of the world borrow. They are intentionally simple:
//! no builder pattern, no lazy iterators — just composable free functions
//! that can be chained by the caller.

use graph_core::{Entity, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship};
use graph_world::World;

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
            last_decayed_batch: 0,
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
}
