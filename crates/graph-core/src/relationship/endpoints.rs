use crate::ids::LocusId;

use super::{EndpointKey, Endpoints};

pub(super) fn directed(from: LocusId, to: LocusId) -> Endpoints {
    Endpoints::Directed { from, to }
}

pub(super) fn symmetric(a: LocusId, b: LocusId) -> Endpoints {
    Endpoints::Symmetric { a, b }
}

pub(super) fn all_endpoints_in(
    endpoints: &Endpoints,
    set: &rustc_hash::FxHashSet<LocusId>,
) -> bool {
    match endpoints {
        Endpoints::Directed { from, to } => set.contains(from) && set.contains(to),
        Endpoints::Symmetric { a, b } => set.contains(a) && set.contains(b),
    }
}

pub(super) fn involves(endpoints: &Endpoints, locus: LocusId) -> bool {
    match endpoints {
        Endpoints::Directed { from, to } => *from == locus || *to == locus,
        Endpoints::Symmetric { a, b } => *a == locus || *b == locus,
    }
}

pub(super) fn other_than(endpoints: &Endpoints, locus: LocusId) -> LocusId {
    match endpoints {
        Endpoints::Directed { from, to } => {
            if *from == locus {
                *to
            } else {
                *from
            }
        }
        Endpoints::Symmetric { a, b } => {
            if *a == locus {
                *b
            } else {
                *a
            }
        }
    }
}

pub(super) fn source(endpoints: &Endpoints) -> Option<LocusId> {
    match endpoints {
        Endpoints::Directed { from, .. } => Some(*from),
        Endpoints::Symmetric { .. } => None,
    }
}

pub(super) fn target(endpoints: &Endpoints) -> Option<LocusId> {
    match endpoints {
        Endpoints::Directed { to, .. } => Some(*to),
        Endpoints::Symmetric { .. } => None,
    }
}

pub(super) fn key(endpoints: &Endpoints) -> EndpointKey {
    match endpoints {
        Endpoints::Directed { from, to } => EndpointKey::Directed(*from, *to),
        Endpoints::Symmetric { a, b } => {
            let (lo, hi) = if a.0 <= b.0 { (*a, *b) } else { (*b, *a) };
            EndpointKey::Symmetric(lo, hi)
        }
    }
}
