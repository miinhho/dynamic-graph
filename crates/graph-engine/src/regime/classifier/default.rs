use crate::regime::{BatchHistory, DynamicsRegime, RegimeClassifier};

/// Default regime classifier. Algorithm (unchanged from phase 1+2,
/// re-indexed to batches instead of ticks):
///
/// 1. If fewer than `window` batches recorded -> `Initializing`.
/// 2. If max energy < `quiescent_threshold` -> `Quiescent`.
/// 3. If all deltas are decreasing -> `Settling`.
/// 4. If energy is monotonically increasing -> `Diverging`.
/// 5. If any two consecutive (energy, energy+2) pairs are identical
///    within tolerance -> `LimitCycleSuspect`.
/// 6. Otherwise -> `Oscillating`.
///
/// Phase 7: thresholds hard-coded. No benchmark used non-default values.
#[derive(Debug, Clone, Default)]
pub struct DefaultRegimeClassifier;

const QUIESCENT_THRESHOLD: f32 = 1e-4;
const DIVERGE_THRESHOLD: f32 = 1e3;
const LIMIT_CYCLE_TOLERANCE: f32 = 1e-3;

impl RegimeClassifier for DefaultRegimeClassifier {
    fn classify(&self, history: &BatchHistory) -> DynamicsRegime {
        if !history.is_full() {
            return DynamicsRegime::Initializing;
        }

        let energies: Vec<f32> = history.iter().map(|metrics| metrics.total_energy).collect();

        if is_quiescent(&energies) {
            return DynamicsRegime::Quiescent;
        }

        if is_diverging(&energies) {
            return DynamicsRegime::Diverging;
        }

        if is_settling(&energies) {
            return DynamicsRegime::Settling;
        }

        if is_limit_cycle_suspect(&energies) {
            return DynamicsRegime::LimitCycleSuspect;
        }

        DynamicsRegime::Oscillating
    }
}

fn is_quiescent(energies: &[f32]) -> bool {
    energies.iter().all(|&energy| energy < QUIESCENT_THRESHOLD)
}

fn is_diverging(energies: &[f32]) -> bool {
    if energies.iter().any(|&energy| energy > DIVERGE_THRESHOLD) {
        return true;
    }
    let monotone_increasing = energies.windows(2).all(|window| window[1] >= window[0]);
    monotone_increasing && energies.last() > energies.first()
}

fn is_settling(energies: &[f32]) -> bool {
    energies.windows(2).all(|window| window[1] <= window[0])
}

fn is_limit_cycle_suspect(energies: &[f32]) -> bool {
    energies.len() >= 4
        && energies.windows(4).any(|window| {
            (window[0] - window[2]).abs() < LIMIT_CYCLE_TOLERANCE
                && (window[1] - window[3]).abs() < LIMIT_CYCLE_TOLERANCE
        })
}
