//! Graph export utilities.
//!
//! Export the relationship graph to common visualization formats.
//!
//! ## DOT / Graphviz
//!
//! [`to_dot`] and [`to_dot_filtered`] produce DOT language strings that can be
//! piped directly into Graphviz tools (`dot`, `neato`, etc.) or pasted into
//! online viewers.
//!
//! Nodes represent loci, labelled with their ID and slot-0 state value.
//! Directed edges represent relationships, annotated with activity and weight.
//!
//! ## Example
//!
//! ```rust,ignore
//! let dot = graph_query::to_dot(&world);
//! std::fs::write("graph.dot", &dot).unwrap();
//! // Then: dot -Tsvg graph.dot -o graph.svg
//! ```
//!
//! ```rust,ignore
//! let dot = graph_query::to_dot_filtered(&world, excitatory_kind);
//! println!("{dot}");
//! ```

mod dot;
pub use dot::{to_dot, to_dot_filtered};

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship, RelationshipLineage,
        StateVector,
    };
    use graph_world::World;

    fn make_world_with_edge(activity: f32) -> World {
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
        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[activity, 0.5]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: smallvec::SmallVec::new(),
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        w
    }

    #[test]
    fn dot_contains_digraph_header() {
        let w = make_world_with_edge(0.5);
        let dot = to_dot(&w);
        assert!(dot.starts_with("digraph {"), "got: {dot}");
    }

    #[test]
    fn dot_contains_node_for_each_locus() {
        let w = make_world_with_edge(0.5);
        let dot = to_dot(&w);
        assert!(dot.contains("n0"), "got: {dot}");
        assert!(dot.contains("n1"), "got: {dot}");
    }

    #[test]
    fn dot_contains_edge_between_loci() {
        let w = make_world_with_edge(0.5);
        let dot = to_dot(&w);
        assert!(dot.contains("n0 -> n1"), "got: {dot}");
    }

    #[test]
    fn dot_edge_contains_activity_and_weight() {
        let w = make_world_with_edge(0.75);
        let dot = to_dot(&w);
        assert!(
            dot.contains("a=0.75") || dot.contains("a=0.7"),
            "got: {dot}"
        );
        assert!(
            dot.contains("w=0.50") || dot.contains("w=0.5"),
            "got: {dot}"
        );
    }

    #[test]
    fn dot_filtered_excludes_other_kinds() {
        let w = make_world_with_edge(0.5);
        // The relationship has kind 1; filter for kind 2 — no edges expected.
        let dot = to_dot_filtered(&w, InfluenceKindId(2));
        assert!(!dot.contains("n0 -> n1"), "got: {dot}");
        // But nodes should still be present.
        assert!(dot.contains("n0"), "got: {dot}");
        assert!(dot.contains("n1"), "got: {dot}");
    }

    #[test]
    fn dot_filtered_includes_matching_kind() {
        let w = make_world_with_edge(0.5);
        let dot = to_dot_filtered(&w, InfluenceKindId(1));
        assert!(dot.contains("n0 -> n1"), "got: {dot}");
    }

    #[test]
    fn dot_empty_world_produces_valid_stub() {
        let w = World::new();
        let dot = to_dot(&w);
        assert!(dot.contains("digraph {"));
        assert!(dot.trim_end().ends_with('}'));
    }

    #[test]
    fn dot_node_label_contains_state() {
        let w = make_world_with_edge(0.5);
        let dot = to_dot(&w);
        // Locus 0 has state 0.3, locus 1 has state 0.7.
        assert!(dot.contains("0.300") || dot.contains("0.3"), "got: {dot}");
        assert!(dot.contains("0.700") || dot.contains("0.7"), "got: {dot}");
    }
}
