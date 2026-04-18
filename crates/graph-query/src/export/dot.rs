use graph_core::{InfluenceKindId, LocusId};
use graph_world::World;

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
    to_dot_with_filter(world, None)
}

/// Export only the relationships of a specific `kind` as a DOT string.
///
/// Loci that have no relationships of the given kind are still included as
/// isolated nodes so that the node set is complete.
pub fn to_dot_filtered(world: &World, kind: InfluenceKindId) -> String {
    to_dot_with_filter(world, Some(kind))
}

fn to_dot_with_filter(world: &World, filter_kind: Option<InfluenceKindId>) -> String {
    let mut out = String::with_capacity(512);
    push_dot_header(&mut out);
    write_nodes(world, &mut out);
    out.push('\n');
    write_edges(world, filter_kind, &mut out);
    out.push_str("}\n");
    out
}

fn push_dot_header(out: &mut String) {
    out.push_str("digraph {\n");
    out.push_str("  graph [rankdir=LR];\n");
    out.push_str("  node  [shape=circle fontname=\"Helvetica\" fontsize=10];\n");
    out.push_str("  edge  [fontname=\"Helvetica\" fontsize=9];\n\n");
}

fn write_nodes(world: &World, out: &mut String) {
    let mut locus_ids: Vec<LocusId> = world.loci().iter().map(|locus| locus.id).collect();
    locus_ids.sort_by_key(|id| id.0);

    for locus_id in locus_ids {
        let Some(locus) = world.locus(locus_id) else {
            continue;
        };
        let state0 = locus.state.as_slice().first().copied().unwrap_or(0.0);
        out.push_str(&format!(
            "  n{} [label=\"{}\\n{:.3}\"];\n",
            locus_id.0, locus_id.0, state0
        ));
    }
}

fn write_edges(world: &World, filter_kind: Option<InfluenceKindId>, out: &mut String) {
    let mut relationships: Vec<_> = world.relationships().iter().collect();
    relationships.sort_by_key(|relationship| relationship.id.0);

    for relationship in relationships {
        if let Some(kind) = filter_kind
            && relationship.kind != kind
        {
            continue;
        }
        write_edge(relationship, out);
    }
}

fn write_edge(relationship: &graph_core::Relationship, out: &mut String) {
    let activity = relationship
        .state
        .as_slice()
        .first()
        .copied()
        .unwrap_or(0.0);
    let weight = relationship.state.as_slice().get(1).copied().unwrap_or(0.0);

    match relationship.endpoints {
        graph_core::Endpoints::Directed { from, to } => {
            out.push_str(&format!(
                "  n{} -> n{} [label=\"k{}\\na={:.2} w={:.2}\" color=\"{}\"];\n",
                from.0,
                to.0,
                relationship.kind.0,
                activity,
                weight,
                activity_color(activity),
            ));
        }
        graph_core::Endpoints::Symmetric { a, b } => {
            let label = format!(
                "k{}\\na={:.2} w={:.2}",
                relationship.kind.0, activity, weight
            );
            let color = activity_color(activity);
            out.push_str(&format!(
                "  n{} -> n{} [label=\"{label}\" color=\"{color}\" dir=both];\n",
                a.0, b.0,
            ));
        }
    }
}

fn activity_color(activity: f32) -> &'static str {
    match activity {
        a if a < 0.2 => "#cccccc",
        a if a < 0.4 => "#99aacc",
        a if a < 0.6 => "#6688bb",
        a if a < 0.8 => "#3366aa",
        _ => "#003399",
    }
}
