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
pub mod causal_strength;
mod entity_causality;
mod centrality;
mod counterfactual;
mod debug;
mod deviation;
mod entity_query;
mod export;
mod filter;
mod labels;
mod planner;
mod profile;
mod query;
mod api_dogfood;
mod query_api;
mod temporal;
mod traversal;

pub use causal_strength::{
    causal_direction, causal_in_strength, causal_out_strength,
    dominant_causes, dominant_effects, feedback_pairs,
};
pub use centrality::{
    all_betweenness, all_closeness, all_constraints,
    betweenness_centrality, closeness_centrality,
    effective_network_size, structural_constraint,
    louvain, louvain_with_resolution,
    pagerank, pagerank_centrality,
    TriangleBalance, all_triangles, balance_index, triangle_balance, unstable_triangles,
    modularity,
};
pub use causality::{
    causal_ancestors, causal_depth, causal_descendants,
    changes_to_locus_in_range,
    changes_to_relationship_in_range, common_ancestors, committed_batches,
    is_ancestor_of, last_change_to_locus, last_change_to_relationship,
    loci_changed_in_batch, relationship_volatility, relationship_volatility_all,
    relationship_activity_trend, relationship_activity_trend_with_threshold,
    relationship_weight_delta, relationship_weight_trend,
    relationship_weight_trend_delta, relationship_weight_trend_with_threshold, Trend,
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
    most_changed_relationships, most_similar_relationships, relationships_by_change_count,
    relationships_above_strength, relationships_top_n_by_strength,
    relationships_created_in, relationships_idle_for, relationships_older_than,
    relationship_touch_rate,
    relationships_between, relationships_between_of_kind,
    relationships_from, relationships_from_of_kind,
    relationships_to, relationships_to_of_kind,
    relationships_matching, relationships_matching_slots,
    relationships_of_kind, relationships_of_kinds,
    relationships_with_activity, relationships_with_slot, relationships_with_weight,
    relationships_with_str_property, relationships_with_f64_property,
    dominant_flow_kind, kind_flow_diversity, kind_transition_rate,
};
pub use profile::{relationship_profile, RelationshipBundle};
// `RelationshipBundle::profile_trend_similarity` is a method — no separate re-export needed.
// `net_influence_between` is also re-exported via filter; callers that prefer
// the bundle-first style should use `relationship_profile(...).net_activity_with_interactions(...)`.
pub use query::{
    loci, loci_from_ids, LociQuery,
    relationships, relationships_from_ids, RelationshipsQuery,
    ActivityStats,
};
pub use entity_query::{
    all_coheres, coheres, entities,
    CohereQuery, EntitiesQuery,
};
pub use temporal::{
    batch_stats, changed_since,
    last_n_changes_to_locus, last_n_changes_to_relationship,
    loci_by_change_frequency, relationships_by_change_frequency,
    BatchStats,
};
pub use counterfactual::{
    counterfactual, counterfactual_replay,
    relationships_absent_without, relationships_caused_by,
    CounterfactualDiff, CounterfactualQuery,
};
pub use debug::{causal_trace, CausalStep, CausalTrace};
pub use deviation::{entity_diff, entity_deviations_since, EntityDiff};
pub use entity_causality::{
    cause_seed_changes, entity_layers_in_range, entity_transition_cause,
    entity_upstream_transitions,
};
pub use export::{to_dot, to_dot_filtered};
pub use labels::{
    entities_summary, entity_summary, relationship_list,
    to_dot_named, to_dot_named_filtered,
    EntitySummary, NameMap,
};
/// Serializable query API — [`Query`], [`QueryResult`], [`api::execute`], and [`api::explain`].
///
/// Notable additions over the base traversal layer:
/// - Active-traversal variants (`ReachableFromActive`, `DownstreamOfActive`,
///   `UpstreamOfActive`, `PathBetweenActive`) that prune dormant edges during BFS.
/// - `LocusPredicate::ReachableFromActive` / `DownstreamOfActive` / `UpstreamOfActive`
///   for filtering loci in `FindLoci` using active-subgraph reachability.
pub mod api {
    pub use super::query_api::{
        execute,
        EntityPredicate, EntitySort, LocusPredicate, LocusSort, Query, QueryResult,
        RelationshipPredicate, RelationshipProfileResult, RelSort,
        RelationshipSummary, LocusSummary,
        EntityDiffSummary, CohereResult, TrendResult,
        WorldMetricsResult,
        FindRelationshipsBuilder, FindLociBuilder, FindEntitiesBuilder,
    };
    pub use super::planner::{explain, CostClass, PlanStep, QueryPlan};
}

pub use traversal::{
    connected_components, connected_components_of_kind,
    directed_path, directed_path_of_kind,
    downstream_of, downstream_of_active, downstream_of_kind,
    has_cycle,
    hub_loci, infer_transitive, isolated_loci,
    neighbors_of, neighbors_of_kind,
    path_between, path_between_active, path_between_of_kind,
    reachable_from, reachable_from_active, reachable_from_of_kind, reachable_matching,
    reciprocal_of, reciprocal_pairs,
    sink_loci, source_loci,
    strongest_path,
    upstream_of, upstream_of_active, upstream_of_kind,
    TransitiveRule,
};
