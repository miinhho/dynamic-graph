use crate::report::BoundaryEdge;
use graph_core::RelationshipId;

pub(super) fn compute_tension(
    confirmed: &[BoundaryEdge],
    ghost: &[BoundaryEdge],
    shadow: &[RelationshipId],
) -> f32 {
    let total = (confirmed.len() + ghost.len() + shadow.len()).max(1) as f32;
    let divergence = (ghost.len() + shadow.len()) as f32;
    divergence / total
}
