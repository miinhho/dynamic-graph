//! [`BoundaryReport`]: the result of a static ↔ dynamic boundary analysis.

use graph_core::{LocusId, RelationshipId};
use graph_schema::DeclaredRelKind;

/// A directed pair with its declared predicate.
///
/// Used in both `confirmed` and `ghost` lists. Subject and object are as
/// declared in the static schema; directionality is the schema's, not the
/// dynamic engine's.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryEdge {
    pub subject: LocusId,
    pub predicate: DeclaredRelKind,
    pub object: LocusId,
    /// The matching dynamic [`RelationshipId`], if one exists.
    pub dynamic_rel: Option<RelationshipId>,
}

/// Result of [`crate::analyze_boundary`].
///
/// See the crate-level documentation for the four-quadrant model and the
/// tension score formula.
#[derive(Debug, Clone)]
pub struct BoundaryReport {
    /// Declared facts that have a matching active dynamic relationship.
    pub confirmed: Vec<BoundaryEdge>,
    /// Declared facts whose dynamic relationship is absent or dormant.
    pub ghost: Vec<BoundaryEdge>,
    /// Active dynamic relationships with no declared counterpart.
    pub shadow: Vec<RelationshipId>,
    /// Divergence score in `[0.0, 1.0]`.
    /// `0.0` = perfectly aligned; `1.0` = no overlap.
    pub tension: f32,
}

impl BoundaryReport {
    /// Total number of edges considered (confirmed + ghost + shadow).
    pub fn total(&self) -> usize {
        self.confirmed.len() + self.ghost.len() + self.shadow.len()
    }

    /// Returns `true` if both worlds are fully aligned (tension == 0.0).
    pub fn is_aligned(&self) -> bool {
        self.tension == 0.0
    }
}
