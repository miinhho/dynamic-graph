use graph_world::World;

use super::{Query, QueryResult, RelationshipProfileResult, TrendResult};

pub(super) fn execute_state_and_profile(world: &World, query: &Query) -> Option<QueryResult> {
    use crate::*;

    match query {
        Query::LocusStateSlot { locus, slot } => {
            let v = world
                .locus(*locus)
                .and_then(|l| l.state.as_slice().get(*slot).copied());
            Some(QueryResult::MaybeScore(v))
        }
        Query::RelationshipProfile { from, to } => {
            let bundle = relationship_profile(world, *from, *to);
            let profile = relationship_profile_result(bundle, *from, *to);
            Some(QueryResult::RelationshipProfile(profile))
        }
        Query::ActivityTrend {
            relationship,
            from_batch,
            to_batch,
        } => {
            let trend = relationship_activity_trend(world, *relationship, *from_batch, *to_batch);
            Some(QueryResult::Trend(activity_trend_result(trend)))
        }
        _ => None,
    }
}

fn relationship_profile_result(
    bundle: crate::RelationshipBundle<'_>,
    from: graph_core::LocusId,
    to: graph_core::LocusId,
) -> RelationshipProfileResult {
    RelationshipProfileResult {
        from,
        to,
        relationship_ids: bundle.relationships.iter().map(|r| r.id).collect(),
        total_activity: bundle.net_activity(),
        net_influence: forward_activity(&bundle, from) - backward_activity(&bundle, from),
        dominant_kind: bundle.dominant_kind(),
        activity_by_kind: bundle.activity_by_kind(),
    }
}

fn forward_activity(bundle: &crate::RelationshipBundle<'_>, from: graph_core::LocusId) -> f32 {
    use graph_core::Endpoints;

    bundle
        .relationships
        .iter()
        .filter(|&&relationship| {
            matches!(
                relationship.endpoints,
                Endpoints::Directed { from: source, .. } if source == from
            )
        })
        .map(|relationship| relationship.activity())
        .sum()
}

fn backward_activity(bundle: &crate::RelationshipBundle<'_>, from: graph_core::LocusId) -> f32 {
    use graph_core::Endpoints;

    bundle
        .relationships
        .iter()
        .filter(|&&relationship| {
            matches!(
                relationship.endpoints,
                Endpoints::Directed { to: target, .. } if target == from
            )
        })
        .map(|relationship| relationship.activity())
        .sum()
}

fn activity_trend_result(trend: Option<crate::Trend>) -> TrendResult {
    match trend {
        None => TrendResult::Insufficient,
        Some(crate::Trend::Rising { slope }) => TrendResult::Rising { slope },
        Some(crate::Trend::Falling { slope }) => TrendResult::Falling { slope },
        Some(crate::Trend::Stable) => TrendResult::Stable,
    }
}
