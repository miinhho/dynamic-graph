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

use graph_core::{InfluenceKindId, LocusId};
use graph_world::World;

// ─── to_dot ───────────────────────────────────────────────────────────────────

/// Export the entire relationship graph as a DOT string.
///
/// All loci present in the world become nodes, labelled with their ID and
/// slot-0 state value. All relationships become directed edges annotated with
/// their `activity` (slot 0) and `weight` (slot 1).
///
/// The DOT output can be fed to any Graphviz layout engine:
///
/// ```sh
/// dot -Tsvg graph.dot -o graph.svg
/// neato -Tpng graph.dot -o graph.png
/// ```
pub fn to_dot(world: &World) -> String {
    to_dot_impl(world, None)
}

/// Export only the relationships of a specific `kind` as a DOT string.
///
/// Loci that have no relationships of the given kind are still included as
/// isolated nodes so that the node set is complete.
pub fn to_dot_filtered(world: &World, kind: InfluenceKindId) -> String {
    to_dot_impl(world, Some(kind))
}

// ─── Internal implementation ──────────────────────────────────────────────────

fn to_dot_impl(world: &World, filter_kind: Option<InfluenceKindId>) -> String {
    let mut out = String::with_capacity(512);

    out.push_str("digraph {\n");
    out.push_str("  graph [rankdir=LR];\n");
    out.push_str("  node  [shape=circle fontname=\"Helvetica\" fontsize=10];\n");
    out.push_str("  edge  [fontname=\"Helvetica\" fontsize=9];\n\n");

    // ── Nodes ────────────────────────────────────────────────────────────────
    // Sort loci by ID for deterministic output.
    let mut locus_ids: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    locus_ids.sort_by_key(|id| id.0);

    for lid in &locus_ids {
        let locus = match world.locus(*lid) {
            Some(l) => l,
            None => continue,
        };
        let state0 = locus.state.as_slice().first().copied().unwrap_or(0.0);
        out.push_str(&format!(
            "  n{} [label=\"{}\\n{:.3}\"];\n",
            lid.0, lid.0, state0
        ));
    }

    out.push('\n');

    // ── Edges ────────────────────────────────────────────────────────────────
    // Collect and sort relationships by ID for deterministic output.
    let mut rels: Vec<_> = world.relationships().iter().collect();
    rels.sort_by_key(|r| r.id.0);

    for rel in rels {
        if let Some(kind) = filter_kind {
            if rel.kind != kind {
                continue;
            }
        }

        let activity = rel.state.as_slice().first().copied().unwrap_or(0.0);
        let weight = rel.state.as_slice().get(1).copied().unwrap_or(0.0);

        match rel.endpoints {
            graph_core::Endpoints::Directed { from, to } => {
                out.push_str(&format!(
                    "  n{} -> n{} [label=\"k{}\\na={:.2} w={:.2}\" color=\"{}\"];\n",
                    from.0, to.0, rel.kind.0, activity, weight, activity_color(activity),
                ));
            }
            graph_core::Endpoints::Symmetric { a, b } => {
                // Render symmetric relationships as a bidirectional pair.
                let label = format!("k{}\\na={:.2} w={:.2}", rel.kind.0, activity, weight);
                let color = activity_color(activity);
                out.push_str(&format!(
                    "  n{} -> n{} [label=\"{label}\" color=\"{color}\" dir=both];\n",
                    a.0, b.0,
                ));
            }
        }
    }

    out.push_str("}\n");
    out
}

/// Map activity [0, 1] to a DOT color string.
///
/// Low activity → light grey; high activity → dark blue.
fn activity_color(activity: f32) -> &'static str {
    match activity {
        a if a < 0.2 => "#cccccc",
        a if a < 0.4 => "#99aacc",
        a if a < 0.6 => "#6688bb",
        a if a < 0.8 => "#3366aa",
        _ => "#003399",
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship,
        RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn make_world_with_edge(activity: f32) -> World {
        let mut w = World::new();
        w.insert_locus(Locus::new(LocusId(0), LocusKindId(1), StateVector::from_slice(&[0.3])));
        w.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::from_slice(&[0.7])));
        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
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
        assert!(dot.contains("a=0.75") || dot.contains("a=0.7"), "got: {dot}");
        assert!(dot.contains("w=0.50") || dot.contains("w=0.5"), "got: {dot}");
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
