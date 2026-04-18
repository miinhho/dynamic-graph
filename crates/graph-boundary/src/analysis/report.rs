use crate::report::BoundaryReport;

use super::collect::BoundaryMatches;

pub(super) fn build_boundary_report(matches: BoundaryMatches, tension: f32) -> BoundaryReport {
    BoundaryReport {
        confirmed: matches.confirmed,
        ghost: matches.ghost,
        shadow: matches.shadow,
        tension,
    }
}
