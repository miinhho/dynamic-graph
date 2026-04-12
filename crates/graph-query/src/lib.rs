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
mod query;
mod traversal;

pub use causality::{
    causal_ancestors, causal_depth, changes_to_locus_in_range,
    changes_to_relationship_in_range, common_ancestors, committed_batches,
    is_ancestor_of, last_change_to_locus, last_change_to_relationship,
    loci_changed_in_batch, relationship_volatility, relationship_volatility_all,
    relationships_changed_in_batch, root_stimuli, root_stimuli_for_relationship,
};
pub use filter::{
    active_entities, entities_matching, entities_with_coherence, entities_with_member,
    entity_member_loci, locus_entities, top_entity_members,
    incoming_activity_sum, outgoing_activity_sum, net_influence_balance,
    net_influence_between,
    loci_matching, loci_of_kind, loci_top_n_by_state, loci_with_f64_property,
    loci_with_state, loci_with_str_property,
    locus_degree, locus_in_degree, locus_out_degree,
    lookup_loci, lookup_relationships,
    most_connected_loci, most_connected_loci_with_degree,
    most_changed_relationships, relationships_by_change_count,
    relationships_above_strength, relationships_top_n_by_strength,
    relationships_created_in, relationships_idle_for, relationships_older_than,
    relationship_touch_rate,
    relationships_between, relationships_between_of_kind,
    relationships_from, relationships_from_of_kind,
    relationships_to, relationships_to_of_kind,
    relationships_matching, relationships_of_kind, relationships_of_kinds,
    relationships_with_activity, relationships_with_slot, relationships_with_weight,
    relationships_with_str_property, relationships_with_f64_property,
};
pub use query::{
    loci, loci_from_ids, LociQuery,
    relationships, relationships_from_ids, RelationshipsQuery,
};
pub use traversal::{
    connected_components, connected_components_of_kind,
    directed_path, directed_path_of_kind,
    downstream_of, downstream_of_kind,
    hub_loci, isolated_loci,
    neighbors_of, neighbors_of_kind,
    path_between, path_between_of_kind,
    reachable_from, reachable_from_of_kind, reachable_matching,
    reciprocal_of, reciprocal_pairs,
    strongest_path,
    upstream_of, upstream_of_kind,
};
