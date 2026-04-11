//! Read-only graph traversal and query operations over a `World`.
//!
//! This crate sits above `graph-world` (which owns storage and mutation)
//! and provides three query surfaces:
//!
//! ## Structural traversal (`traversal`)
//!
//! BFS-based graph operations treating the relationship graph as undirected.
//! Kind-filtered variants (`_of_kind`) restrict traversal to edges of a
//! specific `RelationshipKindId`.
//!
//! ```ignore
//! let path  = graph_query::path_between(&world, from, to);
//! let reach = graph_query::reachable_from(&world, start, 2);
//! let comps = graph_query::connected_components(&world);
//! ```
//!
//! ## State and property filters (`filter`)
//!
//! Filter loci or relationships by kind, numeric state, named slot value,
//! or domain properties. All functions return `Vec<&Locus>` or
//! `Vec<&Relationship>` valid for the borrow of `world`.
//!
//! ```ignore
//! let active_orgs = graph_query::loci_with_str_property(&world, "type", |v| v == "ORG");
//! let hot_edges   = graph_query::relationships_with_activity(&world, |a| a > 0.5);
//! let tagged      = graph_query::relationships_with_slot(&world, 2, |v| v > 0.3);
//! ```
//!
//! ## Causal log queries (`causality`)
//!
//! Walk the predecessor DAG recorded in the `ChangeLog`.
//!
//! ```ignore
//! let ancestors = graph_query::causal_ancestors(&world, change_id);
//! let roots     = graph_query::root_stimuli(&world, change_id);
//! let history   = graph_query::changes_to_locus_in_range(&world, locus, from, to);
//! ```

mod causality;
mod filter;
mod traversal;

pub use causality::{
    causal_ancestors, changes_to_locus_in_range, changes_to_relationship_in_range,
    is_ancestor_of, relationship_volatility, root_stimuli, root_stimuli_for_relationship,
};
pub use filter::{
    active_entities, entities_matching, entities_with_coherence, entities_with_member,
    loci_matching, loci_of_kind, loci_with_f64_property, loci_with_state,
    loci_with_str_property,
    relationships_between, relationships_between_of_kind,
    relationships_from, relationships_from_of_kind,
    relationships_to, relationships_to_of_kind,
    relationships_matching, relationships_of_kind,
    relationships_with_activity, relationships_with_slot, relationships_with_weight,
};
pub use traversal::{
    connected_components, connected_components_of_kind,
    directed_path, directed_path_of_kind,
    downstream_of, downstream_of_kind,
    path_between, path_between_of_kind,
    reachable_from, reachable_from_of_kind, reachable_matching,
    strongest_path,
    upstream_of, upstream_of_kind,
};
