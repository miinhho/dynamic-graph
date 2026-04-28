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
pub mod coalgebra;
pub mod metric;
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

pub use self::{
    causal_strength::*, causality::*, centrality::*, counterfactual::*, debug::*, deviation::*,
    emergence::*, entity_causality::*, entity_query::*, export::*, filter::*, labels::*,
    profile::*, query::*, temporal::*, time_travel::*, traversal::*,
};
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
