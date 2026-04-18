use graph_core::{Endpoints, InfluenceKindId, LocusId};
use graph_world::World;
use rustc_hash::FxHashSet;

pub fn sort_desc_truncate(scored: &mut Vec<(LocusId, f32)>, n: usize) {
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
}

pub fn neighbors_of_kind(world: &World, locus: LocusId, kind: InfluenceKindId) -> Vec<LocusId> {
    let mut seen = FxHashSet::default();
    for rel in world.relationships().iter() {
        if rel.kind != kind {
            continue;
        }
        match rel.endpoints {
            Endpoints::Directed { from, to } => {
                if from == locus {
                    seen.insert(to);
                } else if to == locus {
                    seen.insert(from);
                }
            }
            Endpoints::Symmetric { a, b } => {
                if a == locus {
                    seen.insert(b);
                } else if b == locus {
                    seen.insert(a);
                }
            }
        }
    }
    seen.into_iter().collect()
}
