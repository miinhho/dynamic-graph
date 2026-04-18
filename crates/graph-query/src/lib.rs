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

mod api_dogfood;
pub mod causal_strength;
mod causality;
mod centrality;
mod counterfactual;
mod debug;
mod deviation;
mod emergence;
mod entity_causality;
mod entity_query;
mod export;
mod filter;
mod labels;
mod planner;
mod profile;
mod query;
mod query_api;
mod temporal;
mod time_travel;
mod traversal;

pub use causal_strength::{
    causal_direction, causal_in_strength, causal_out_strength, dominant_causes, dominant_effects,
    feedback_pairs,
};
pub use causality::{
    CoarseTrail, Trend, causal_ancestors, causal_coarse_trail, causal_depth, causal_descendants,
    changes_to_locus_in_range, changes_to_relationship_in_range, committed_batches,
    common_ancestors, is_ancestor_of, last_change_to_locus, last_change_to_relationship,
    loci_changed_in_batch, relationship_activity_trend, relationship_activity_trend_with_threshold,
    relationship_volatility, relationship_volatility_all, relationship_weight_delta,
    relationship_weight_trend, relationship_weight_trend_delta,
    relationship_weight_trend_with_threshold, relationships_changed_in_batch, root_stimuli,
    root_stimuli_for_relationship,
};
pub use centrality::{
    TriangleBalance, all_betweenness, all_closeness, all_constraints, all_triangles, balance_index,
    betweenness_centrality, closeness_centrality, effective_network_size, louvain,
    louvain_with_resolution, modularity, pagerank, pagerank_centrality, structural_constraint,
    triangle_balance, unstable_triangles,
};
pub use filter::{
    active_entities, dominant_flow_kind, entities_matching, entities_with_coherence,
    entities_with_member, entity_member_loci, incoming_activity_sum, kind_flow_diversity,
    kind_transition_rate, loci_matching, loci_of_kind, loci_top_n_by_state, loci_with_f64_property,
    loci_with_state, loci_with_str_property, locus_degree, locus_entities, locus_in_degree,
    locus_out_degree, lookup_loci, lookup_relationships, most_changed_relationships,
    most_connected_loci, most_connected_loci_with_degree, most_similar_relationships,
    net_influence_balance, net_influence_between, outgoing_activity_sum, relationship_touch_rate,
    relationships_above_strength, relationships_between, relationships_between_of_kind,
    relationships_by_change_count, relationships_created_in, relationships_from,
    relationships_from_of_kind, relationships_idle_for, relationships_matching,
    relationships_matching_slots, relationships_of_kind, relationships_of_kinds,
    relationships_older_than, relationships_to, relationships_to_of_kind,
    relationships_top_n_by_strength, relationships_with_activity, relationships_with_f64_property,
    relationships_with_slot, relationships_with_str_property, relationships_with_weight,
    top_entity_members,
};
pub use profile::{RelationshipBundle, relationship_profile};
// `RelationshipBundle::profile_trend_similarity` is a method — no separate re-export needed.
// `net_influence_between` is also re-exported via filter; callers that prefer
// the bundle-first style should use `relationship_profile(...).net_activity_with_interactions(...)`.
pub use counterfactual::{
    CounterfactualDiff, CounterfactualQuery, counterfactual, counterfactual_replay,
    relationships_absent_without, relationships_caused_by,
};
pub use debug::{CausalStep, CausalTrace, causal_trace};
pub use deviation::{EntityDiff, entity_deviations_since, entity_diff};
pub use emergence::{
    DecayRates, DropResult, EmergenceEntry, EmergenceReport, EmergenceSynergyEntry,
    EmergenceSynergyReport, LeaveOneOutResult, PsiResult, PsiSynergyResult, SynergyPair,
    UnmeasuredEntry, UnmeasuredReason, coherence_autocorrelation, coherence_dense_series,
    coherence_dense_series_with_decay, coherence_stable_series, emergence_report,
    emergence_report_synergy, emergence_report_synergy_with_decay, emergence_report_with_decay,
    psi_scalar, psi_scalar_with_decay, psi_synergy, psi_synergy_leave_one_out,
    psi_synergy_leave_one_out_with_decay, psi_synergy_with_decay,
};
pub use entity_causality::{
    cause_seed_changes, entity_layers_in_range, entity_transition_cause,
    entity_upstream_transitions,
};
pub use entity_query::{CohereQuery, EntitiesQuery, all_coheres, coheres, entities};
pub use export::{to_dot, to_dot_filtered};
pub use labels::{
    EntitySummary, NameMap, entities_summary, entity_summary, relationship_list, to_dot_named,
    to_dot_named_filtered,
};
pub use query::{
    ActivityStats, LociQuery, RelationshipsQuery, loci, loci_from_ids, relationships,
    relationships_from_ids,
};
pub use temporal::{
    BatchStats, batch_stats, changed_since, last_n_changes_to_locus,
    last_n_changes_to_relationship, loci_by_change_frequency, relationships_by_change_frequency,
};
pub use time_travel::{TimeTravelResult, time_travel};
/// Serializable query API — [`Query`], [`QueryResult`], [`api::execute`], and [`api::explain`].
///
/// Notable additions over the base traversal layer:
/// - Active-traversal variants (`ReachableFromActive`, `DownstreamOfActive`,
///   `UpstreamOfActive`, `PathBetweenActive`) that prune dormant edges during BFS.
/// - `LocusPredicate::ReachableFromActive` / `DownstreamOfActive` / `UpstreamOfActive`
///   for filtering loci in `FindLoci` using active-subgraph reachability.
pub mod api {
    pub use super::planner::{CostClass, PlanStep, QueryPlan, explain};
    pub use super::query_api::{
        CohereResult, EntityDiffSummary, EntityPredicate, EntitySort, FindEntitiesBuilder,
        FindLociBuilder, FindRelationshipsBuilder, LocusPredicate, LocusSort, LocusSummary, Query,
        QueryResult, RelSort, RelationshipPredicate, RelationshipProfileResult,
        RelationshipSummary, TrendResult, WorldMetricsResult, execute,
    };
}

pub use traversal::{
    TransitiveRule, connected_components, connected_components_of_kind, directed_path,
    directed_path_of_kind, downstream_of, downstream_of_active, downstream_of_kind, has_cycle,
    hub_loci, infer_transitive, isolated_loci, neighbors_of, neighbors_of_kind, path_between,
    path_between_active, path_between_of_kind, reachable_from, reachable_from_active,
    reachable_from_of_kind, reachable_matching, reciprocal_of, reciprocal_pairs, sink_loci,
    source_loci, strongest_path, upstream_of, upstream_of_active, upstream_of_kind,
};
