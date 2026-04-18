use graph_core::{Endpoints, InfluenceKindId, LocusId};
use graph_world::World;

use super::types::{PairPredictionRanking, RankedPair};

pub(super) fn rank_pairs(world: &World, kind: InfluenceKindId) -> PairPredictionRanking {
    let mut entries: Vec<RankedPair> = world
        .relationships()
        .iter()
        .filter(|relationship| relationship.kind == kind)
        .filter_map(|relationship| match relationship.endpoints {
            Endpoints::Symmetric { a, b } => {
                let pair = canonical_pair(a, b);
                Some(RankedPair {
                    pair,
                    strength: relationship.strength(),
                })
            }
            Endpoints::Directed { .. } => None,
        })
        .collect();
    entries.sort_by(|left, right| right.strength.total_cmp(&left.strength));
    PairPredictionRanking { entries }
}

fn canonical_pair(a: LocusId, b: LocusId) -> (LocusId, LocusId) {
    if a.0 < b.0 { (a, b) } else { (b, a) }
}
