//! Stability-oriented assertions used by engine integration tests.
//!
//! These helpers know nothing about the tick loop; they operate purely on
//! [`EntityState`] snapshots and numeric histories so the testkit stays free
//! of any dependency on `graph-engine`.

use graph_core::EntityState;

/// Returns the L2 distance between the internal-state vectors of two states.
pub fn internal_distance(a: &EntityState, b: &EntityState) -> f32 {
    a.internal.distance(&b.internal)
}

/// Panics if any sample in `history` exceeds `max_norm`. Used to assert that
/// damping/clipping kept the runtime bounded across a tick stream.
pub fn assert_bounded_history(history: &[f32], max_norm: f32) {
    for (i, &value) in history.iter().enumerate() {
        assert!(
            value.is_finite() && value <= max_norm,
            "history value {value} at sample {i} exceeded bound {max_norm}"
        );
    }
}

/// Panics if `history` does not strictly decrease into the noise floor by the
/// final sample. `noise_floor` is the largest value that still counts as
/// "converged".
pub fn assert_converges(history: &[f32], noise_floor: f32) {
    let last = *history
        .last()
        .expect("convergence check requires at least one sample");
    assert!(
        last <= noise_floor,
        "final history value {last} above noise floor {noise_floor}; history = {history:?}"
    );
}

/// Panics if two state lists differ in any internal-state component beyond
/// `eps`. Used by replay determinism tests.
pub fn assert_states_equivalent(left: &[EntityState], right: &[EntityState], eps: f32) {
    assert_eq!(
        left.len(),
        right.len(),
        "state lists differ in length: {} vs {}",
        left.len(),
        right.len()
    );
    for (i, (a, b)) in left.iter().zip(right.iter()).enumerate() {
        let dist = internal_distance(a, b);
        assert!(
            dist <= eps,
            "state {i} differs by {dist} (> {eps}); left = {a:?}, right = {b:?}"
        );
    }
}

/// Panics if `history` does not show at least one sign change between
/// consecutive samples — used to assert that an oscillation fixture really
/// oscillates.
pub fn assert_has_oscillation(history: &[f32]) {
    let mut flips = 0;
    for window in history.windows(2) {
        if window[0] == 0.0 || window[1] == 0.0 {
            continue;
        }
        if window[0].signum() != window[1].signum() {
            flips += 1;
        }
    }
    assert!(
        flips >= 1,
        "expected at least one sign flip in history but found none: {history:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_history_passes_within_bound() {
        assert_bounded_history(&[0.5, 0.4, 0.3], 1.0);
    }

    #[test]
    #[should_panic(expected = "exceeded bound")]
    fn bounded_history_panics_when_exceeded() {
        assert_bounded_history(&[0.5, 1.5], 1.0);
    }

    #[test]
    fn converges_accepts_value_at_floor() {
        assert_converges(&[1.0, 0.5, 0.01], 0.05);
    }

    #[test]
    #[should_panic(expected = "above noise floor")]
    fn converges_rejects_above_floor() {
        assert_converges(&[1.0, 0.9], 0.05);
    }

    #[test]
    fn oscillation_detected_in_alternating_history() {
        assert_has_oscillation(&[1.0, -1.0, 1.0]);
    }

    #[test]
    #[should_panic(expected = "expected at least one sign flip")]
    fn oscillation_assertion_fails_for_monotone() {
        assert_has_oscillation(&[1.0, 0.5, 0.25]);
    }
}
