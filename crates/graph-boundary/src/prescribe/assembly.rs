use super::candidates::{GhostCandidate, ShadowCandidate};
use super::{BoundaryAction, PrescriptionConfig, RetractReason};

pub(super) fn assemble_retraction(candidate: GhostCandidate) -> BoundaryAction {
    BoundaryAction::RetractFact {
        fact_id: candidate.fact_id,
        reason: RetractReason::LongRunningGhost {
            age_versions: candidate.age_versions,
        },
    }
}

pub(super) fn assemble_assertion(
    candidate: ShadowCandidate,
    config: &PrescriptionConfig,
) -> BoundaryAction {
    BoundaryAction::AssertFact {
        subject: candidate.subject,
        predicate: config.shadow_predicate.clone(),
        object: candidate.object,
        shadow_rel: candidate.shadow_rel,
    }
}
