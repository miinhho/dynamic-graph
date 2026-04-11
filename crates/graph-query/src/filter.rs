//! State- and property-based filtering of loci and relationships.
//!
//! All functions take `&World` and return `Vec` of references valid for
//! the lifetime of the world borrow. They are intentionally simple:
//! no builder pattern, no lazy iterators — just composable free functions
//! that can be chained by the caller.

use graph_core::{InfluenceKindId, Locus, LocusKindId, Relationship};
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
}
