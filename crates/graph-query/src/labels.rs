//! Domain-readable labels and named exports.
//!
//! Loci, relationships, and entities carry numeric IDs internally. Users
//! typically assign human-readable names via the `PropertyStore` ("name"
//! key). This module provides:
//!
//! 1. **[`NameMap`]** — a cheap snapshot of the `PropertyStore`'s "name"
//!    field, built once from `&World` and used as a lookup for display.
//! 2. **Named DOT export** — [`to_dot_named`] and [`to_dot_named_filtered`]
//!    render the relationship graph using human-readable node labels instead
//!    of raw numeric IDs.
//! 3. **Named entity summary** — [`entity_summary`] and
//!    [`entities_summary`] produce readable text descriptions of entity
//!    state, resolving locus IDs to names where available.
//! 4. **Named relationship list** — [`relationship_list`] formats a
//!    human-readable table of relationships with named endpoints.
//!
//! ## Example
//!
//! ```ignore
//! let names = graph_query::NameMap::from_world(&world);
//! let dot   = graph_query::to_dot_named(&world, &names);
//! std::fs::write("named_graph.dot", &dot).unwrap();
//!
//! for line in graph_query::relationship_list(&world, &names) {
//!     println!("{}", line);
//! }
//! ```

use graph_core::{EntityId, InfluenceKindId, LocusId};
use graph_world::World;
use rustc_hash::FxHashMap;

// ─── NameMap ──────────────────────────────────────────────────────────────────

/// A snapshot of human-readable names for loci.
///
/// Built from the `PropertyStore` "name" field. Missing entries fall back to
/// `"locus_<id>"` automatically.
///
/// `NameMap` is cheap to query (single hash lookup) but does *not* stay in
/// sync with the world after construction. Rebuild it whenever the property
/// store may have changed.
#[derive(Debug, Default, Clone)]
pub struct NameMap {
    names: FxHashMap<LocusId, String>,
}

impl NameMap {
    /// Build a `NameMap` from the current `PropertyStore` "name" fields.
    ///
    /// Loci that have no "name" property are silently omitted; lookups on
    /// those IDs fall back to `"locus_<id>"` at display time.
    pub fn from_world(world: &World) -> Self {
        let mut names = FxHashMap::default();
        for locus in world.loci().iter() {
            if let Some(props) = world.properties().get(locus.id) {
                if let Some(name) = props.get_str("name") {
                    names.insert(locus.id, name.to_owned());
                }
            }
        }
        Self { names }
    }

    /// Build a `NameMap` from an explicit iterator of `(LocusId, name)` pairs.
    ///
    /// Useful in tests or when names come from a source other than the
    /// `PropertyStore`.
    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (LocusId, S)>,
        S: Into<String>,
    {
        let names = pairs.into_iter().map(|(id, s)| (id, s.into())).collect();
        Self { names }
    }

    /// Look up the human-readable name for `locus`.
    ///
    /// Returns the registered name if present, or `"locus_<id>"` as a
    /// stable fallback.
    pub fn name(&self, locus: LocusId) -> String {
        self.names
            .get(&locus)
            .cloned()
            .unwrap_or_else(|| format!("locus_{}", locus.0))
    }

    /// Look up the name, returning `None` if no explicit name is registered.
    pub fn get(&self, locus: LocusId) -> Option<&str> {
        self.names.get(&locus).map(|s| s.as_str())
    }

    /// Number of explicitly registered names.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// True if no names have been registered.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

// ─── Named DOT export ─────────────────────────────────────────────────────────

/// Export the relationship graph as a DOT string, using human-readable node
/// labels from `names`.
///
/// Node labels show `"<name>\n<state0>"` instead of raw IDs. Edge labels
/// retain activity and weight.
///
/// Equivalent to [`to_dot`](crate::to_dot) but with names substituted in.
pub fn to_dot_named(world: &World, names: &NameMap) -> String {
    to_dot_named_impl(world, names, None)
}

/// Export only relationships of `kind` as a named DOT string.
///
/// Same as [`to_dot_named`] but restricted to a single relationship kind.
pub fn to_dot_named_filtered(world: &World, names: &NameMap, kind: InfluenceKindId) -> String {
    to_dot_named_impl(world, names, Some(kind))
}

fn to_dot_named_impl(
    world: &World,
    names: &NameMap,
    filter_kind: Option<InfluenceKindId>,
) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("digraph {\n");
    out.push_str("  graph [rankdir=LR];\n");
    out.push_str("  node  [shape=ellipse fontname=\"Helvetica\" fontsize=10];\n");
    out.push_str("  edge  [fontname=\"Helvetica\" fontsize=9];\n\n");

    let mut locus_ids: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    locus_ids.sort_by_key(|id| id.0);

    for lid in &locus_ids {
        let locus = match world.locus(*lid) {
            Some(l) => l,
            None => continue,
        };
        let state0 = locus.state.as_slice().first().copied().unwrap_or(0.0);
        let label = names.name(*lid);
        // Escape backslashes and quotes in label for DOT safety.
        let safe_label = label.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "  n{} [label=\"{}\\n{:.3}\"];\n",
            lid.0, safe_label, state0
        ));
    }

    out.push('\n');

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
        let color = activity_color(activity);

        match rel.endpoints {
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

    out.push_str("}\n");
    out
}

// ─── Named relationship list ──────────────────────────────────────────────────

/// Format each relationship as a human-readable line.
///
/// Returns one string per relationship, of the form:
/// ```text
/// "AWAL" → "AIZL"   activity=0.35  weight=0.12  kind=2
/// ```
///
/// Loci without registered names fall back to `"locus_<id>"`.
pub fn relationship_list(world: &World, names: &NameMap) -> Vec<String> {
    let mut rels: Vec<_> = world.relationships().iter().collect();
    rels.sort_by_key(|r| r.id.0);

    rels.iter()
        .map(|rel| {
            let activity = rel.state.as_slice().first().copied().unwrap_or(0.0);
            let weight = rel.state.as_slice().get(1).copied().unwrap_or(0.0);
            match rel.endpoints {
                graph_core::Endpoints::Directed { from, to } => {
                    format!(
                        "\"{}\" → \"{}\"   activity={:.3}  weight={:.3}  kind={}",
                        names.name(from),
                        names.name(to),
                        activity,
                        weight,
                        rel.kind.0,
                    )
                }
                graph_core::Endpoints::Symmetric { a, b } => {
                    format!(
                        "\"{}\" ↔ \"{}\"   activity={:.3}  weight={:.3}  kind={}",
                        names.name(a),
                        names.name(b),
                        activity,
                        weight,
                        rel.kind.0,
                    )
                }
            }
        })
        .collect()
}

// ─── Named entity summary ─────────────────────────────────────────────────────

/// A human-readable summary of a single entity.
#[derive(Debug, Clone)]
pub struct EntitySummary {
    pub entity_id: EntityId,
    /// Display name derived from member names (first 3 members, then "…").
    pub display_name: String,
    /// Coherence score.
    pub coherence: f32,
    /// Human-readable names of current members.
    pub member_names: Vec<String>,
    /// Entity status as a string.
    pub status: String,
    /// Number of sediment layers.
    pub layer_count: usize,
}

impl std::fmt::Display for EntitySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Entity#{} [{}]  coherence={:.3}  status={}  layers={}  members=[{}]",
            self.entity_id.0,
            self.display_name,
            self.coherence,
            self.status,
            self.layer_count,
            self.member_names.join(", "),
        )
    }
}

/// Produce a named summary for a single entity.
///
/// Returns `None` if the entity is not found.
pub fn entity_summary(
    world: &World,
    entity_id: EntityId,
    names: &NameMap,
) -> Option<EntitySummary> {
    let entity = world.entities().get(entity_id)?;
    Some(make_summary(entity, names))
}

/// Produce named summaries for all entities, sorted by coherence descending.
pub fn entities_summary(world: &World, names: &NameMap) -> Vec<EntitySummary> {
    let mut summaries: Vec<EntitySummary> = world
        .entities()
        .iter()
        .map(|e| make_summary(e, names))
        .collect();
    summaries.sort_by(|a, b| {
        b.coherence
            .partial_cmp(&a.coherence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    summaries
}

fn make_summary(entity: &graph_core::Entity, names: &NameMap) -> EntitySummary {
    let member_names: Vec<String> = entity
        .current
        .members
        .iter()
        .map(|&id| names.name(id))
        .collect();

    let display_name = {
        let first_three: Vec<&str> = member_names.iter().take(3).map(|s| s.as_str()).collect();
        if entity.current.members.len() > 3 {
            format!("{}…", first_three.join(", "))
        } else {
            first_three.join(", ")
        }
    };

    let status = match entity.status {
        graph_core::EntityStatus::Active => "active".to_owned(),
        graph_core::EntityStatus::Dormant => "dormant".to_owned(),
    };

    EntitySummary {
        entity_id: entity.id,
        display_name,
        coherence: entity.current.coherence,
        member_names,
        status,
        layer_count: entity.layers.len(),
    }
}

// ─── Helper ───────────────────────────────────────────────────────────────────

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
