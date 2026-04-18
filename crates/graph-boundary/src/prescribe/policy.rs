use super::PrescriptionConfig;
use super::candidates::{GhostCandidate, ShadowCandidate};

pub(super) fn should_retract_ghost(
    candidate: &GhostCandidate,
    config: &PrescriptionConfig,
) -> bool {
    config
        .ghost_version_threshold
        .is_some_and(|threshold| candidate.age_versions >= threshold)
}

pub(super) fn should_assert_shadow(
    candidate: &ShadowCandidate,
    config: &PrescriptionConfig,
) -> bool {
    config
        .shadow_signal_threshold
        .is_some_and(|threshold| candidate.signal >= threshold)
}
