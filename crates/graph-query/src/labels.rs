mod dot;
mod entity_summary;
mod name_map;

pub use self::entity_summary::EntitySummary;
pub use self::entity_summary::{entities_summary, entity_summary};
pub use self::name_map::NameMap;
pub use dot::{relationship_list, to_dot_named, to_dot_named_filtered};

#[cfg(test)]
use self::entity_summary::make_summary;

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Entity, EntityId, EntitySnapshot, Locus, LocusId, LocusKindId, StateVector,
    };
    use graph_world::World;

    fn make_world_with_names() -> World {
        let mut w = World::new();
        w.insert_locus(Locus::new(
            LocusId(0),
            LocusKindId(1),
            StateVector::from_slice(&[0.3]),
        ));
        w.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::from_slice(&[0.7]),
        ));
        // Add names via properties.
        let mut p0 = graph_core::Properties::new();
        p0.set("name", "Alice");
        w.properties_mut().insert(LocusId(0), p0);
        let mut p1 = graph_core::Properties::new();
        p1.set("name", "Bob");
        w.properties_mut().insert(LocusId(1), p1);
        w
    }

    #[test]
    fn name_map_from_world_resolves_names() {
        let w = make_world_with_names();
        let map = NameMap::from_world(&w);
        assert_eq!(map.name(LocusId(0)), "Alice");
        assert_eq!(map.name(LocusId(1)), "Bob");
        assert_eq!(map.name(LocusId(99)), "locus_99");
    }

    #[test]
    fn name_map_from_pairs() {
        let map = NameMap::from_pairs([(LocusId(5), "Foo"), (LocusId(6), "Bar")]);
        assert_eq!(map.name(LocusId(5)), "Foo");
        assert_eq!(map.name(LocusId(6)), "Bar");
        assert_eq!(map.name(LocusId(7)), "locus_7");
    }

    #[test]
    fn to_dot_named_uses_human_labels() {
        let w = make_world_with_names();
        let map = NameMap::from_world(&w);
        let dot = to_dot_named(&w, &map);
        assert!(dot.contains("Alice"), "missing Alice: {dot}");
        assert!(dot.contains("Bob"), "missing Bob: {dot}");
        // Should not use raw IDs in labels (they still appear in node IDs though).
        assert!(dot.contains("n0"), "missing node n0: {dot}");
    }

    #[test]
    fn relationship_list_uses_names() {
        let mut w = make_world_with_names();
        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(graph_core::Relationship {
            id: rel_id,
            kind: graph_core::InfluenceKindId(1),
            endpoints: graph_core::Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[0.5, 0.3]),
            lineage: graph_core::RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: smallvec::SmallVec::new(),
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        let map = NameMap::from_world(&w);
        let list = relationship_list(&w, &map);
        assert_eq!(list.len(), 1);
        assert!(list[0].contains("Alice"), "missing Alice: {}", list[0]);
        assert!(list[0].contains("Bob"), "missing Bob: {}", list[0]);
    }

    #[test]
    fn entity_summary_display_name_truncates_at_three() {
        let snapshot = EntitySnapshot {
            members: vec![LocusId(0), LocusId(1), LocusId(2), LocusId(3)],
            member_relationships: vec![],
            coherence: 0.8,
        };
        let entity = Entity::born(EntityId(0), BatchId(1), snapshot);
        let map = NameMap::from_pairs([
            (LocusId(0), "A"),
            (LocusId(1), "B"),
            (LocusId(2), "C"),
            (LocusId(3), "D"),
        ]);
        let summary = make_summary(&entity, &map);
        assert!(
            summary.display_name.contains('…'),
            "expected truncation: {}",
            summary.display_name
        );
    }

    #[test]
    fn entities_summary_sorted_by_coherence_descending() {
        let mut w = World::new();
        let s1 = EntitySnapshot {
            members: vec![],
            member_relationships: vec![],
            coherence: 0.3,
        };
        let s2 = EntitySnapshot {
            members: vec![],
            member_relationships: vec![],
            coherence: 0.9,
        };
        w.entities_mut()
            .insert(Entity::born(EntityId(0), BatchId(1), s1));
        w.entities_mut()
            .insert(Entity::born(EntityId(1), BatchId(1), s2));
        let map = NameMap::default();
        let summaries = entities_summary(&w, &map);
        assert_eq!(summaries[0].entity_id, EntityId(1)); // highest coherence first
        assert_eq!(summaries[1].entity_id, EntityId(0));
    }
}
