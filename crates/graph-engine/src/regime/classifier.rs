//! Regime classification: observing the engine's dynamical state.
//!
//! Ported from phase 1+2's `convergence.rs` / `regime.rs` with two
//! key changes per `docs/redesign.md` §7:
//!
//! 1. Time is now measured in *batches*, not ticks. `BatchMetrics`
//!    holds per-batch summary statistics; `BatchHistory` is the ring
//!    buffer of recent batches the classifier inspects.
//!
//! 2. Classification is *per-kind*: each `InfluenceKindId` can be in
//!    a different regime. The `DefaultRegimeClassifier` produces one
//!    regime for the aggregate (whole-batch statistics), but the
//!    `RegimeClassifier` trait is open for per-kind implementations.
//!
//! The regimes themselves are unchanged from the redesign framing in
//! `docs/identity.md`: Quiescent and Settling are observation modes,
//! Oscillating and LimitCycleSuspect are valid dynamics, only
//! Diverging triggers the guard rail.

use std::collections::VecDeque;

/// Observed dynamical regime of the system at a given point.
///
/// See `docs/identity.md` §4 for the full framing. The short version:
/// - `Quiescent` and `Settling` are transient / quiet states.
/// - `Oscillating` and `LimitCycleSuspect` are **valid** dynamical
///   modes — the engine does not suppress them.
/// - Only `Diverging` triggers the guard rail (shrinks alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicsRegime {
    /// Not enough history to classify yet (fewer than `window` batches
    /// have committed). All components start here.
    Initializing,
    /// Energy is decreasing monotonically toward zero.
    Settling,
    /// Energy is near zero; nothing significant is happening.
    Quiescent,
    /// Energy oscillates — increases and decreases alternate. This is a
    /// valid observation mode; the guard rail must not suppress it.
    Oscillating,
    /// Same period-2 (or longer) pattern repeats — candidate for a
    /// stable limit cycle. Valid dynamical mode; do not suppress.
    LimitCycleSuspect,
    /// Energy is increasing without bound. The guard rail acts here.
    Diverging,
}

impl DynamicsRegime {
    /// Convert to the lightweight `RegimeTag` defined in graph-core.
    pub fn to_tag(self) -> graph_core::RegimeTag {
        match self {
            DynamicsRegime::Initializing => graph_core::RegimeTag::Initializing,
            DynamicsRegime::Settling => graph_core::RegimeTag::Settling,
            DynamicsRegime::Quiescent => graph_core::RegimeTag::Quiescent,
            DynamicsRegime::Oscillating => graph_core::RegimeTag::Oscillating,
            DynamicsRegime::LimitCycleSuspect => graph_core::RegimeTag::LimitCycleSuspect,
            DynamicsRegime::Diverging => graph_core::RegimeTag::Diverging,
        }
    }
}

// DynamicsRegime does not have an Equilibrium variant (Settling/Quiescent
// cover that space), but RegimeTag does for forward-compatibility.

impl std::fmt::Display for DynamicsRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DynamicsRegime::Initializing => write!(f, "Initializing"),
            DynamicsRegime::Settling => write!(f, "Settling"),
            DynamicsRegime::Quiescent => write!(f, "Quiescent"),
            DynamicsRegime::Oscillating => write!(f, "Oscillating"),
            DynamicsRegime::LimitCycleSuspect => write!(f, "LimitCycleSuspect"),
            DynamicsRegime::Diverging => write!(f, "Diverging"),
        }
    }
}

/// Per-batch summary statistics recorded by the engine.
#[derive(Debug, Clone, Default)]
pub struct BatchMetrics {
    /// Sum of `|after - before|.l2_norm()` over all committed changes
    /// in the batch. Zero if no changes fired.
    pub total_delta_norm: f32,
    /// Number of changes committed in this batch.
    pub change_count: u32,
    /// Sum of `after.l2_norm()` over all committed changes. Proxy for
    /// "how much energy is in the system" after this batch.
    pub total_energy: f32,
}

impl BatchMetrics {
    /// Build from the changes committed in one batch.
    pub fn from_changes<'a>(changes: impl Iterator<Item = &'a graph_core::Change>) -> Self {
        let mut m = Self::default();
        for change in changes {
            m.change_count += 1;
            let delta_norm = {
                let before = &change.before;
                let after = &change.after;
                // ||after - before||₂
                let dim = before.dim().max(after.dim());
                let delta_sq: f32 = (0..dim)
                    .map(|i| {
                        let b = before.as_slice().get(i).copied().unwrap_or(0.0);
                        let a = after.as_slice().get(i).copied().unwrap_or(0.0);
                        (a - b).powi(2)
                    })
                    .sum();
                delta_sq.sqrt()
            };
            m.total_delta_norm += delta_norm;
            m.total_energy += change.after.l2_norm();
        }
        m
    }
}

/// Ring buffer of recent per-batch metrics.
#[derive(Debug, Clone)]
pub struct BatchHistory {
    window: usize,
    history: VecDeque<BatchMetrics>,
}

impl BatchHistory {
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            history: VecDeque::with_capacity(window),
        }
    }

    pub fn push(&mut self, metrics: BatchMetrics) {
        if self.history.len() >= self.window {
            self.history.pop_front();
        }
        self.history.push_back(metrics);
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.history.len() >= self.window
    }

    pub fn window(&self) -> usize {
        self.window
    }

    pub fn iter(&self) -> impl Iterator<Item = &BatchMetrics> {
        self.history.iter()
    }
}

/// Classifies the dynamical regime from a `BatchHistory`.
pub trait RegimeClassifier: Send + Sync {
    fn classify(&self, history: &BatchHistory) -> DynamicsRegime;
}

/// Default regime classifier. Algorithm (unchanged from phase 1+2,
/// re-indexed to batches instead of ticks):
///
/// 1. If fewer than `window` batches recorded → `Initializing`.
/// 2. If max energy < `quiescent_threshold` → `Quiescent`.
/// 3. If all deltas are decreasing → `Settling`.
/// 4. If energy is monotonically increasing → `Diverging`.
/// 5. If any two consecutive (energy, energy+2) pairs are identical
///    within tolerance → `LimitCycleSuspect`.
/// 6. Otherwise → `Oscillating`.
#[derive(Debug, Clone)]
pub struct DefaultRegimeClassifier {
    pub quiescent_threshold: f32,
    pub diverge_threshold: f32,
    pub limit_cycle_tolerance: f32,
}

impl Default for DefaultRegimeClassifier {
    fn default() -> Self {
        Self {
            quiescent_threshold: 1e-4,
            diverge_threshold: 1e3,
            limit_cycle_tolerance: 1e-3,
        }
    }
}

impl RegimeClassifier for DefaultRegimeClassifier {
    fn classify(&self, history: &BatchHistory) -> DynamicsRegime {
        if !history.is_full() {
            return DynamicsRegime::Initializing;
        }

        let energies: Vec<f32> = history.iter().map(|m| m.total_energy).collect();

        // Quiescent: all energy below threshold.
        if energies.iter().all(|&e| e < self.quiescent_threshold) {
            return DynamicsRegime::Quiescent;
        }

        // Diverging: any energy above diverge threshold, or strictly
        // monotonically increasing.
        if energies.iter().any(|&e| e > self.diverge_threshold) {
            return DynamicsRegime::Diverging;
        }
        let monotone_increasing = energies.windows(2).all(|w| w[1] >= w[0]);
        if monotone_increasing && energies.last() > energies.first() {
            return DynamicsRegime::Diverging;
        }

        // Settling: strictly decreasing.
        let settling = energies.windows(2).all(|w| w[1] <= w[0]);
        if settling {
            return DynamicsRegime::Settling;
        }

        // LimitCycleSuspect: period-2 pattern detected.
        if energies.len() >= 4 {
            let period_2 = energies.windows(4).any(|w| {
                (w[0] - w[2]).abs() < self.limit_cycle_tolerance
                    && (w[1] - w[3]).abs() < self.limit_cycle_tolerance
            });
            if period_2 {
                return DynamicsRegime::LimitCycleSuspect;
            }
        }

        DynamicsRegime::Oscillating
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::StateVector;

    fn push_energy(history: &mut BatchHistory, e: f32) {
        history.push(BatchMetrics {
            total_energy: e,
            total_delta_norm: e,
            change_count: 1,
        });
    }

    #[test]
    fn initializing_before_window_fills() {
        let mut h = BatchHistory::new(4);
        push_energy(&mut h, 1.0);
        push_energy(&mut h, 1.0);
        let c = DefaultRegimeClassifier::default();
        assert_eq!(c.classify(&h), DynamicsRegime::Initializing);
    }

    #[test]
    fn quiescent_on_low_energy() {
        let mut h = BatchHistory::new(4);
        for _ in 0..4 {
            push_energy(&mut h, 1e-6);
        }
        assert_eq!(DefaultRegimeClassifier::default().classify(&h), DynamicsRegime::Quiescent);
    }

    #[test]
    fn settling_on_decreasing_energy() {
        let mut h = BatchHistory::new(4);
        for e in [4.0, 3.0, 2.0, 1.0] {
            push_energy(&mut h, e);
        }
        assert_eq!(DefaultRegimeClassifier::default().classify(&h), DynamicsRegime::Settling);
    }

    #[test]
    fn diverging_on_monotone_increase() {
        let mut h = BatchHistory::new(4);
        for e in [1.0, 2.0, 3.0, 4.0] {
            push_energy(&mut h, e);
        }
        assert_eq!(DefaultRegimeClassifier::default().classify(&h), DynamicsRegime::Diverging);
    }

    #[test]
    fn oscillating_on_alternating_energy() {
        let mut h = BatchHistory::new(6);
        for e in [1.0, 2.0, 1.1, 1.9, 1.0, 2.0] {
            push_energy(&mut h, e);
        }
        let regime = DefaultRegimeClassifier::default().classify(&h);
        assert!(
            matches!(regime, DynamicsRegime::Oscillating | DynamicsRegime::LimitCycleSuspect),
            "{regime:?}"
        );
    }

    #[test]
    fn limit_cycle_suspect_on_period_2() {
        let mut h = BatchHistory::new(6);
        for e in [1.0, 2.0, 1.0, 2.0, 1.0, 2.0] {
            push_energy(&mut h, e);
        }
        assert_eq!(
            DefaultRegimeClassifier::default().classify(&h),
            DynamicsRegime::LimitCycleSuspect
        );
    }

    #[test]
    fn batch_metrics_from_changes_computes_delta_and_energy() {
        use graph_core::{BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId};
        let change = Change {
            id: ChangeId(0),
            subject: ChangeSubject::Locus(LocusId(1)),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::zeros(2),
            after: StateVector::from_slice(&[3.0, 4.0]),
            batch: BatchId(0),
        };
        let m = BatchMetrics::from_changes(std::iter::once(&change));
        // delta norm: ||(3,4) - (0,0)|| = 5
        assert!((m.total_delta_norm - 5.0).abs() < 1e-5);
        // energy: ||(3,4)|| = 5
        assert!((m.total_energy - 5.0).abs() < 1e-5);
        assert_eq!(m.change_count, 1);
    }
}
