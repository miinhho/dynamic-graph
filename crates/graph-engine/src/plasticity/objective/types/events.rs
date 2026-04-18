use std::collections::HashSet;

use graph_core::LocusId;

pub(super) fn event_pairs(event_log: &[Vec<Vec<u64>>]) -> HashSet<(LocusId, LocusId)> {
    let mut pairs = HashSet::new();
    for block in event_log {
        for event in block {
            pairs.extend(event_pair_iter(event));
        }
    }
    pairs
}

fn event_pair_iter(event: &[u64]) -> impl Iterator<Item = (LocusId, LocusId)> + '_ {
    (0..event.len()).flat_map(move |left| {
        ((left + 1)..event.len())
            .map(move |right| canonical_pair(LocusId(event[left]), LocusId(event[right])))
    })
}

pub(super) fn canonical_pair(a: LocusId, b: LocusId) -> (LocusId, LocusId) {
    if a.0 < b.0 { (a, b) } else { (b, a) }
}
