use graph_core::{Endpoints, LocusId, RelationshipId};
use graph_schema::{DeclaredFactId, SchemaWorld};
use graph_world::World;

use crate::analysis::{SignalMode, signal};
use crate::report::BoundaryReport;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GhostCandidate {
    pub fact_id: DeclaredFactId,
    pub age_versions: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ShadowCandidate {
    pub subject: LocusId,
    pub object: LocusId,
    pub shadow_rel: RelationshipId,
    pub signal: f32,
}

pub(super) fn collect_ghost_candidates(
    report: &BoundaryReport,
    schema: &SchemaWorld,
) -> Vec<GhostCandidate> {
    let current_version = schema.facts.version();

    report
        .ghost
        .iter()
        .filter_map(|ghost_edge| {
            schema
                .facts
                .facts_between(ghost_edge.subject, &ghost_edge.predicate, ghost_edge.object)
                .next()
                .map(|fact| GhostCandidate {
                    fact_id: fact.id,
                    age_versions: current_version.saturating_sub(fact.asserted_at),
                })
        })
        .collect()
}

pub(super) fn collect_shadow_candidates(
    report: &BoundaryReport,
    dynamic: &World,
    signal_mode: SignalMode,
) -> Vec<ShadowCandidate> {
    report
        .shadow
        .iter()
        .filter_map(|&rel_id| {
            let rel = dynamic.relationships().get(rel_id)?;
            let (subject, object) = endpoints_pair(rel.endpoints.clone());
            Some(ShadowCandidate {
                subject,
                object,
                shadow_rel: rel_id,
                signal: signal(rel, signal_mode),
            })
        })
        .collect()
}

fn endpoints_pair(endpoints: Endpoints) -> (LocusId, LocusId) {
    match endpoints {
        Endpoints::Symmetric { a, b } => (a, b),
        Endpoints::Directed { from, to } => (from, to),
    }
}
