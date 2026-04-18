use graph_core::{InfluenceKindId, LocusId};
use graph_world::World;

use super::NameMap;

pub fn to_dot_named(world: &World, names: &NameMap) -> String {
    render_dot(world, names, None)
}

pub fn to_dot_named_filtered(world: &World, names: &NameMap, kind: InfluenceKindId) -> String {
    render_dot(world, names, Some(kind))
}

pub fn relationship_list(world: &World, names: &NameMap) -> Vec<String> {
    let mut relationships: Vec<_> = world.relationships().iter().collect();
    relationships.sort_by_key(|relationship| relationship.id.0);

    relationships
        .iter()
        .map(|relationship| render_relationship_line(relationship, names))
        .collect()
}

fn render_dot(world: &World, names: &NameMap, filter_kind: Option<InfluenceKindId>) -> String {
    let mut out = String::with_capacity(512);
    push_dot_header(&mut out);
    write_nodes(world, names, &mut out);
    out.push('\n');
    write_edges(world, filter_kind, &mut out);
    out.push_str("}\n");
    out
}

fn push_dot_header(out: &mut String) {
    out.push_str("digraph {\n");
    out.push_str("  graph [rankdir=LR];\n");
    out.push_str("  node  [shape=ellipse fontname=\"Helvetica\" fontsize=10];\n");
    out.push_str("  edge  [fontname=\"Helvetica\" fontsize=9];\n\n");
}

fn write_nodes(world: &World, names: &NameMap, out: &mut String) {
    let mut locus_ids: Vec<LocusId> = world.loci().iter().map(|locus| locus.id).collect();
    locus_ids.sort_by_key(|id| id.0);

    for locus_id in locus_ids {
        let Some(locus) = world.locus(locus_id) else {
            continue;
        };
        let state0 = locus.state.as_slice().first().copied().unwrap_or(0.0);
        let label = escape_dot_label(&names.name(locus_id));
        out.push_str(&format!(
            "  n{} [label=\"{}\\n{:.3}\"];\n",
            locus_id.0, label, state0
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
    let color = activity_color(activity);

    match relationship.endpoints {
        graph_core::Endpoints::Directed { from, to } => {
            out.push_str(&format!(
                "  n{} -> n{} [label=\"a={:.2} w={:.2}\" color=\"{color}\"];\n",
                from.0, to.0, activity, weight,
            ));
        }
        graph_core::Endpoints::Symmetric { a, b } => {
            out.push_str(&format!(
                "  n{} -> n{} [label=\"a={:.2} w={:.2}\" color=\"{color}\" dir=both];\n",
                a.0, b.0, activity, weight,
            ));
        }
    }
}

fn render_relationship_line(relationship: &graph_core::Relationship, names: &NameMap) -> String {
    let activity = relationship
        .state
        .as_slice()
        .first()
        .copied()
        .unwrap_or(0.0);
    let weight = relationship.state.as_slice().get(1).copied().unwrap_or(0.0);

    match relationship.endpoints {
        graph_core::Endpoints::Directed { from, to } => {
            format!(
                "\"{}\" → \"{}\"   activity={:.3}  weight={:.3}  kind={}",
                names.name(from),
                names.name(to),
                activity,
                weight,
                relationship.kind.0,
            )
        }
        graph_core::Endpoints::Symmetric { a, b } => {
            format!(
                "\"{}\" ↔ \"{}\"   activity={:.3}  weight={:.3}  kind={}",
                names.name(a),
                names.name(b),
                activity,
                weight,
                relationship.kind.0,
            )
        }
    }
}

fn escape_dot_label(label: &str) -> String {
    label.replace('\\', "\\\\").replace('"', "\\\"")
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
