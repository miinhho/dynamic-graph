use graph_core::{Endpoints, InfluenceKindId, LocusId};
use graph_world::World;

use super::types::{PairPredictionRanking, RankedPair};

pub(super) fn rank_pairs(world: &World, kind: InfluenceKindId) -> PairPredictionRanking {
    let mut entries: Vec<RankedPair> = world
        .relationships()
        .iter()
        .filter(|relationship| relationship.kind == kind)
        .filter_map(symmetric_ranked_pair)
        .collect();
    sort_ranked_pairs(&mut entries);
    PairPredictionRanking { entries }
}

fn symmetric_ranked_pair(relationship: &graph_core::Relationship) -> Option<RankedPair> {
    match relationship.endpoints {
        Endpoints::Symmetric { a, b } => Some(RankedPair {
            pair: canonical_pair(a, b),
            strength: relationship.strength(),
        }),
        Endpoints::Directed { .. } => None,
    }
}

fn sort_ranked_pairs(entries: &mut [RankedPair]) {
    entries.sort_by(|left, right| right.strength.total_cmp(&left.strength));
}

fn canonical_pair(a: LocusId, b: LocusId) -> (LocusId, LocusId) {
    if a.0 < b.0 { (a, b) } else { (b, a) }
}
