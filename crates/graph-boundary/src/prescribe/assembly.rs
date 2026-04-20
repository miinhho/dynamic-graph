use super::candidates::{GhostCandidate, ShadowCandidate};
use super::{BoundaryAction, PrescriptionConfig, RetractReason};

/// Normalise ghost age to `[0, 1)`. A ghost at exactly the retract
/// threshold scores 0.5; older ghosts asymptote towards 1.0. If the
/// threshold is `None` (disabled) the retraction wouldn't reach
/// assembly, but we still handle it defensively: fall back to a fixed
/// denominator so severity stays well-defined.
fn retraction_severity(age_versions: u64, threshold: Option<u64>) -> f32 {
    let age = age_versions as f32;
    let denom = threshold.map(|t| t.max(1) as f32).unwrap_or(5.0);
    let mag = age / (age + denom);
    mag.clamp(0.0, 1.0)
}

/// Saturating normalisation of a shadow relationship's signal into
/// `[0, 1)`. `signal / (signal + 1.0)` maps `[0, ∞)` monotonically.
fn assertion_severity(signal: f32) -> f32 {
    if signal <= 0.0 {
        0.0
    } else {
        (signal / (signal + 1.0)).clamp(0.0, 1.0)
    }
}

pub(super) fn assemble_retraction(
    candidate: GhostCandidate,
    config: &PrescriptionConfig,
) -> BoundaryAction {
    BoundaryAction::RetractFact {
        fact_id: candidate.fact_id,
        reason: RetractReason::LongRunningGhost {
            age_versions: candidate.age_versions,
        },
        severity: retraction_severity(candidate.age_versions, config.ghost_version_threshold),
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
        severity: assertion_severity(candidate.signal),
    }
}
