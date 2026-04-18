//! Fluent query builders for the emergent layers — entities (Layer 3) and
//! cohere clusters (Layer 4).
//!
//! These mirror the `LociQuery` / `RelationshipsQuery` pattern in `query.rs`
//! and share the same lifetime `'w` so cross-layer navigation composes
//! without re-borrowing.
//!
//! ## Examples
//!
//! ```ignore
//! // All active entities with coherence ≥ 0.6, resolved to their loci.
//! let loci = graph_query::entities(&world)
//!     .active()
//!     .with_min_coherence(0.6)
//!     .member_loci()
//!     .collect();
//!
//! // The strongest cohere cluster under the "structural" perspective.
//! let top = graph_query::coheres(&world, "structural")
//!     .with_min_strength(0.5)
//!     .strongest();
//! ```

use graph_core::{Cohere, Entity};
use graph_world::World;

mod cohere_filters;
mod cohere_terminals;
mod entities_filters;
mod entities_terminals;
mod entity_projection;

// ─── EntitiesQuery ────────────────────────────────────────────────────────────

/// Fluent query builder over the entity store (Layer 3).
///
/// Created by [`entities`]. Filters are applied lazily when a terminal method
/// is called. All methods consume `self` and return a new `EntitiesQuery`
/// (or a concrete value for terminal methods).
pub struct EntitiesQuery<'w> {
    world: &'w World,
    candidates: Vec<&'w Entity>,
}

impl<'w> EntitiesQuery<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self {
            candidates: world.entities().iter().collect(),
            world,
        }
    }
}

// ─── CohereQuery ─────────────────────────────────────────────────────────────

/// Fluent query builder over the cohere cluster store (Layer 4).
///
/// Created by [`coheres`] (single perspective) or [`all_coheres`] (all
/// perspectives). All filter methods consume `self` and return a new
/// `CohereQuery`.
pub struct CohereQuery<'w> {
    candidates: Vec<&'w Cohere>,
}

impl<'w> CohereQuery<'w> {
    pub(crate) fn from_perspective(world: &'w World, perspective: &str) -> Self {
        let candidates = world
            .coheres()
            .get(perspective)
            .map(|slice| slice.iter().collect())
            .unwrap_or_default();
        Self { candidates }
    }

    pub(crate) fn from_all(world: &'w World) -> Self {
        let candidates = world.coheres().iter_all().map(|(_, c)| c).collect();
        Self { candidates }
    }
}

// ─── Public constructors ──────────────────────────────────────────────────────

/// Start a query over all entities in `world`.
///
/// ```ignore
/// let active_count = graph_query::entities(&world).active().count();
/// ```
pub fn entities(world: &World) -> EntitiesQuery<'_> {
    EntitiesQuery::new(world)
}

/// Start a query over the cohere clusters registered under `perspective`.
///
/// Returns an empty query if the perspective has no coheres yet.
///
/// ```ignore
/// let top = graph_query::coheres(&world, "structural").strongest();
/// ```
pub fn coheres<'w>(world: &'w World, perspective: &str) -> CohereQuery<'w> {
    CohereQuery::from_perspective(world, perspective)
}

/// Start a query over **all** cohere clusters across every perspective.
///
/// Useful for cross-perspective analysis. Deduplication is not performed —
/// the same logical concept can appear in multiple perspectives.
///
/// ```ignore
/// let strongest_overall = graph_query::all_coheres(&world).strongest();
/// ```
pub fn all_coheres(world: &World) -> CohereQuery<'_> {
    CohereQuery::from_all(world)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Cohere, CohereId, CohereMembers, Entity, EntityId, EntitySnapshot, EntityStatus,
        Locus, LocusId, LocusKindId, StateVector,
    };
    use graph_world::World;

    fn add_locus(world: &mut World, id: u64) {
        world.insert_locus(Locus::new(
            LocusId(id),
            LocusKindId(0),
            StateVector::from_slice(&[0.5]),
        ));
    }

    fn make_entity(id: u64, members: Vec<u64>, coherence: f32, active: bool) -> Entity {
        let snapshot = EntitySnapshot {
            members: members.iter().map(|&m| LocusId(m)).collect(),
            member_relationships: vec![],
            coherence,
        };
        let mut entity = Entity::born(EntityId(id), BatchId(1), snapshot);
        if !active {
            entity.status = EntityStatus::Dormant;
        }
        entity
    }

    fn add_cohere(
        world: &mut World,
        perspective: &str,
        id: u64,
        entities: Vec<u64>,
        strength: f32,
    ) {
        let c = Cohere {
            id: CohereId(id),
            members: CohereMembers::Entities(entities.iter().map(|&e| EntityId(e)).collect()),
            strength,
        };
        let existing = world
            .coheres()
            .get(perspective)
            .map(|s| s.to_vec())
            .unwrap_or_default();
        let mut updated = existing;
        updated.push(c);
        world.coheres_mut().update(perspective, updated);
    }

    // ── entities() ───────────────────────────────────────────────────────────

    #[test]
    fn entities_active_filter() {
        let mut w = World::new();
        let e0 = make_entity(0, vec![], 0.8, true);
        let e1 = make_entity(1, vec![], 0.5, false);
        w.entities_mut().insert(e0);
        w.entities_mut().insert(e1);
        assert_eq!(entities(&w).active().count(), 1);
        assert_eq!(entities(&w).dormant().count(), 1);
    }

    #[test]
    fn entities_with_member_filter() {
        let mut w = World::new();
        w.entities_mut()
            .insert(make_entity(0, vec![10, 20], 0.8, true));
        w.entities_mut().insert(make_entity(1, vec![30], 0.5, true));
        assert_eq!(entities(&w).with_member(LocusId(10)).count(), 1);
        assert_eq!(entities(&w).with_member(LocusId(30)).count(), 1);
        assert_eq!(entities(&w).with_member(LocusId(99)).count(), 0);
    }

    #[test]
    fn entities_with_min_coherence() {
        let mut w = World::new();
        w.entities_mut().insert(make_entity(0, vec![], 0.9, true));
        w.entities_mut().insert(make_entity(1, vec![], 0.3, true));
        assert_eq!(entities(&w).with_min_coherence(0.5).count(), 1);
        assert_eq!(entities(&w).with_min_coherence(0.1).count(), 2);
    }

    #[test]
    fn entities_mean_coherence() {
        let mut w = World::new();
        w.entities_mut().insert(make_entity(0, vec![], 0.8, true));
        w.entities_mut().insert(make_entity(1, vec![], 0.4, true));
        let mean = entities(&w).mean_coherence().unwrap();
        assert!((mean - 0.6).abs() < 1e-5);
    }

    #[test]
    fn entities_mean_coherence_empty_returns_none() {
        let w = World::new();
        assert!(entities(&w).mean_coherence().is_none());
    }

    #[test]
    fn entities_strongest() {
        let mut w = World::new();
        w.entities_mut().insert(make_entity(0, vec![], 0.4, true));
        w.entities_mut().insert(make_entity(1, vec![], 0.9, true));
        let top = entities(&w).strongest().unwrap();
        assert_eq!(top.id, EntityId(1));
    }

    #[test]
    fn entities_member_loci_navigates_cross_layer() {
        let mut w = World::new();
        add_locus(&mut w, 10);
        add_locus(&mut w, 20);
        add_locus(&mut w, 30);
        w.entities_mut()
            .insert(make_entity(0, vec![10, 20], 0.8, true));
        w.entities_mut()
            .insert(make_entity(1, vec![20, 30], 0.6, true));

        // Should include loci 10, 20, 30 — 20 deduplicated
        let loci = entities(&w).active().member_loci().collect();
        let ids: Vec<_> = loci.iter().map(|l| l.id).collect();
        assert_eq!(loci.len(), 3);
        assert!(ids.contains(&LocusId(10)));
        assert!(ids.contains(&LocusId(20)));
        assert!(ids.contains(&LocusId(30)));
    }

    #[test]
    fn entities_born_after_filter() {
        let mut w = World::new();
        let mut e0 = make_entity(0, vec![], 0.5, true);
        // Override the birth batch to test the filter
        e0.layers[0].batch = BatchId(5);
        let mut e1 = make_entity(1, vec![], 0.5, true);
        e1.layers[0].batch = BatchId(10);
        w.entities_mut().insert(e0);
        w.entities_mut().insert(e1);

        assert_eq!(entities(&w).born_after(BatchId(7)).count(), 1);
    }

    // ── coheres() ────────────────────────────────────────────────────────────

    #[test]
    fn coheres_from_perspective_filters_correctly() {
        let mut w = World::new();
        add_cohere(&mut w, "structural", 0, vec![0], 0.9);
        add_cohere(&mut w, "structural", 1, vec![1], 0.4);
        add_cohere(&mut w, "temporal", 2, vec![2], 0.7);

        assert_eq!(coheres(&w, "structural").count(), 2);
        assert_eq!(coheres(&w, "temporal").count(), 1);
        assert_eq!(coheres(&w, "unknown").count(), 0);
    }

    #[test]
    fn coheres_with_min_strength() {
        let mut w = World::new();
        add_cohere(&mut w, "default", 0, vec![0], 0.9);
        add_cohere(&mut w, "default", 1, vec![1], 0.3);
        assert_eq!(coheres(&w, "default").with_min_strength(0.5).count(), 1);
    }

    #[test]
    fn coheres_strongest() {
        let mut w = World::new();
        add_cohere(&mut w, "default", 0, vec![0], 0.4);
        add_cohere(&mut w, "default", 1, vec![1], 0.9);
        add_cohere(&mut w, "default", 2, vec![2], 0.6);
        let top = coheres(&w, "default").strongest().unwrap();
        assert_eq!(top.id, CohereId(1));
    }

    #[test]
    fn coheres_mean_strength() {
        let mut w = World::new();
        add_cohere(&mut w, "default", 0, vec![0], 0.8);
        add_cohere(&mut w, "default", 1, vec![1], 0.4);
        let mean = coheres(&w, "default").mean_strength().unwrap();
        assert!((mean - 0.6).abs() < 1e-5);
    }

    #[test]
    fn coheres_with_entity_member() {
        let mut w = World::new();
        add_cohere(&mut w, "default", 0, vec![5, 6], 0.9);
        add_cohere(&mut w, "default", 1, vec![7], 0.5);
        assert_eq!(
            coheres(&w, "default")
                .with_entity_member(EntityId(5))
                .count(),
            1
        );
        assert_eq!(
            coheres(&w, "default")
                .with_entity_member(EntityId(99))
                .count(),
            0
        );
    }

    #[test]
    fn coheres_with_min_entity_count() {
        let mut w = World::new();
        add_cohere(&mut w, "default", 0, vec![1, 2, 3], 0.9);
        add_cohere(&mut w, "default", 1, vec![4], 0.5);
        assert_eq!(coheres(&w, "default").with_min_entity_count(2).count(), 1);
        assert_eq!(coheres(&w, "default").with_min_entity_count(1).count(), 2);
    }

    #[test]
    fn all_coheres_spans_perspectives() {
        let mut w = World::new();
        add_cohere(&mut w, "structural", 0, vec![0], 0.9);
        add_cohere(&mut w, "temporal", 1, vec![1], 0.5);
        assert_eq!(all_coheres(&w).count(), 2);
    }
}
