use graph_world::World;

use super::{Query, QueryResult, RelationshipProfileResult, TrendResult};

pub(super) fn execute_state_and_profile(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::LocusStateSlot { .. } => Some(execute_locus_state_query(world, query)),
        Query::RelationshipProfile { .. } | Query::ActivityTrend { .. } => {
            Some(execute_profile_query(world, query))
        }
        _ => None,
    }
}

fn execute_locus_state_query(world: &World, query: &Query) -> QueryResult {
    match query {
        Query::LocusStateSlot { locus, slot } => QueryResult::MaybeScore(
            world
                .locus(*locus)
                .and_then(|locus| locus.state.as_slice().get(*slot).copied()),
        ),
        _ => unreachable!("locus state dispatcher received non-locus-state query"),
    }
}

fn execute_profile_query(world: &World, query: &Query) -> QueryResult {
    use crate::{relationship_activity_trend, relationship_profile};

    match query {
        Query::RelationshipProfile { from, to } => {
            let bundle = relationship_profile(world, *from, *to);
            QueryResult::RelationshipProfile(relationship_profile_result(bundle, *from, *to))
        }
        Query::ActivityTrend {
            relationship,
            from_batch,
            to_batch,
        } => QueryResult::Trend(activity_trend_result(relationship_activity_trend(
            world,
            *relationship,
            *from_batch,
            *to_batch,
        ))),
        _ => unreachable!("profile dispatcher received non-profile query"),
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
