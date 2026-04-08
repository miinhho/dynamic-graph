//! Dynamics-regime classification.
//!
//! See `docs/identity.md` for the framing this module operates under. The
//! short version: this module sorts the engine's recent behaviour into one
//! of several **observation regimes**. It is not a "did the system converge"
//! classifier — convergence is not a goal of this engine.
//!
//! What is provided:
//!
//! - [`TickMetrics`] — per-tick scalar metrics derived from a committed
//!   [`TickTransaction`]'s deltas.
//! - [`MetricsHistory`] — fixed-capacity ring buffer of metrics across ticks.
//! - [`RegimeClassifier`] — stateless trait that maps a history into a
//!   [`DynamicsRegime`].
//! - [`DefaultRegimeClassifier`] — small heuristic implementation.
//!
//! The classifier is intentionally decoupled from the tick path. Callers
//! thread a [`MetricsHistory`] between ticks and call
//! [`RegimeClassifier::classify`] when they want a regime read. This keeps
//! the engine's tick path unchanged.
//!
//! ## Regime semantics
//!
//! Of the six regimes, only [`DynamicsRegime::Diverging`] indicates that the
//! guard rail (the stabilizer) should *do something*. The others are valid
//! observation modes — including [`DynamicsRegime::Oscillating`] and
//! [`DynamicsRegime::LimitCycleSuspect`], which previous iterations of this
//! module incorrectly treated as failure conditions.
//!
//! See `docs/identity.md` §4 for the full table.

use std::collections::VecDeque;

use graph_tx::TickTransaction;

/// Per-tick aggregate metrics derived from the committed transaction deltas.
///
/// All values are aggregated over the entities touched in this tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TickMetrics {
    /// Sum of L2 norms of `(after.internal - before.internal)` across deltas.
    pub total_delta_norm: f32,
    /// Largest single per-entity internal-state delta norm.
    pub max_delta_norm: f32,
    /// Sum of L2 norms of `after.internal` — used as an energy proxy.
    pub total_energy: f32,
    /// Number of scalar components whose sign flipped between before and after.
    pub sign_flips: u32,
    /// Number of entities that produced a recorded delta.
    pub touched_entities: u32,
}

impl TickMetrics {
    /// Compute metrics from a committed transaction. Uncommitted transactions
    /// (no `committed_version`) still produce a value — callers can decide
    /// whether to feed those into history.
    pub fn from_transaction(tx: &TickTransaction) -> Self {
        let mut metrics = TickMetrics::default();
        for delta in &tx.deltas {
            let diff = delta.after.internal.sub(&delta.before.internal);
            let delta_norm = diff.l2_norm();
            metrics.total_delta_norm += delta_norm;
            if delta_norm > metrics.max_delta_norm {
                metrics.max_delta_norm = delta_norm;
            }
            metrics.total_energy += delta.after.internal.l2_norm();
            metrics.sign_flips += sign_flip_count(
                delta.before.internal.values(),
                delta.after.internal.values(),
            );
            metrics.touched_entities += 1;
        }
        metrics
    }
}

fn sign_flip_count(before: &[f32], after: &[f32]) -> u32 {
    let len = before.len().max(after.len());
    let mut flips = 0;
    for i in 0..len {
        let b = before.get(i).copied().unwrap_or(0.0);
        let a = after.get(i).copied().unwrap_or(0.0);
        // Treat exact zero as "no sign", so 0 -> +x is not a flip.
        if b == 0.0 || a == 0.0 {
            continue;
        }
        if b.signum() != a.signum() {
            flips += 1;
        }
    }
    flips
}

/// Fixed-capacity ring buffer of [`TickMetrics`] used by classifiers.
#[derive(Debug, Clone)]
pub struct MetricsHistory {
    capacity: usize,
    samples: VecDeque<TickMetrics>,
}

impl MetricsHistory {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn push(&mut self, metrics: TickMetrics) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(metrics);
    }

    pub fn samples(&self) -> &VecDeque<TickMetrics> {
        &self.samples
    }

    pub fn last(&self) -> Option<&TickMetrics> {
        self.samples.back()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Coarse classification of the engine's current observation regime.
///
/// This is **not** a success/failure verdict. Only [`Self::Diverging`] is a
/// regime the guard rail should counteract; everything else is a valid
/// dynamical mode that the engine is here to let the caller observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicsRegime {
    /// Not enough history to classify.
    #[default]
    Initializing,
    /// Per-tick deltas are decreasing — system is in a transient.
    Settling,
    /// Per-tick deltas are at or below the noise floor. The system is
    /// currently producing no observable change. This is a valid regime, not
    /// a success state. The guard rail may relax in this regime.
    Quiescent,
    /// Bounded sign-flipping behaviour. Valid regime; do not suppress.
    Oscillating,
    /// Recent samples show a repeated pattern. Valid regime; do not suppress.
    LimitCycleSuspect,
    /// Energy or per-tick delta is growing past the configured ratio. The
    /// only regime in which the guard rail should push back.
    Diverging,
}

/// Stateless classifier that maps a [`MetricsHistory`] into a
/// [`DynamicsRegime`].
pub trait RegimeClassifier: Send + Sync {
    fn classify(&self, history: &MetricsHistory) -> DynamicsRegime;
}

/// Tunable threshold-based classifier covering all six regimes.
#[derive(Debug, Clone, Copy)]
pub struct DefaultRegimeClassifier {
    /// `total_delta_norm` at or below this is considered quiescent (no
    /// observable change in this tick).
    pub quiescent_delta: f32,
    /// `total_energy` ratio between consecutive ticks above this triggers
    /// `Diverging` (e.g. `1.5` = 50% growth per tick).
    pub divergence_growth_ratio: f32,
    /// Per-tick sign flip count above this contributes to oscillation.
    pub oscillation_sign_flip_threshold: u32,
    /// Minimum samples needed before any non-`Initializing` verdict.
    pub min_samples: usize,
    /// How many recent samples to scan for repeated-pattern detection.
    pub limit_cycle_window: usize,
    /// L2 distance below which two samples are considered "the same" when
    /// checking for limit cycles.
    pub limit_cycle_eps: f32,
}

impl Default for DefaultRegimeClassifier {
    fn default() -> Self {
        Self {
            quiescent_delta: 1e-4,
            divergence_growth_ratio: 1.5,
            oscillation_sign_flip_threshold: 1,
            min_samples: 3,
            limit_cycle_window: 6,
            limit_cycle_eps: 1e-3,
        }
    }
}

impl RegimeClassifier for DefaultRegimeClassifier {
    fn classify(&self, history: &MetricsHistory) -> DynamicsRegime {
        if history.len() < self.min_samples {
            return DynamicsRegime::Initializing;
        }

        let samples = history.samples();
        let last = *samples.back().expect("non-empty by min_samples check");
        let prev = *samples
            .iter()
            .nth_back(1)
            .expect("at least two samples by min_samples check");

        // Divergence: energy growing past the configured ratio.
        if prev.total_energy > 0.0
            && last.total_energy > prev.total_energy * self.divergence_growth_ratio
        {
            return DynamicsRegime::Diverging;
        }
        // Or delta itself blowing up.
        if prev.total_delta_norm > 0.0
            && last.total_delta_norm > prev.total_delta_norm * self.divergence_growth_ratio
            && last.total_delta_norm > self.quiescent_delta
        {
            return DynamicsRegime::Diverging;
        }

        // Quiescent: delta has flattened to noise. This is reported, not
        // celebrated — see DynamicsRegime::Quiescent's doc comment.
        if last.total_delta_norm <= self.quiescent_delta {
            return DynamicsRegime::Quiescent;
        }

        // Oscillation: sign flips above threshold and delta not shrinking
        // monotonically. Check this before limit cycle so that a
        // sign-flipping run is not misclassified as a degenerate (period-1)
        // cycle.
        let recent_flips: u32 = samples
            .iter()
            .rev()
            .take(self.min_samples)
            .map(|m| m.sign_flips)
            .sum();
        if recent_flips >= self.oscillation_sign_flip_threshold * self.min_samples as u32
            && last.total_delta_norm >= prev.total_delta_norm
        {
            return DynamicsRegime::Oscillating;
        }

        // Limit cycle: recent samples repeat (similar delta+energy) within
        // eps without sign-flipping behaviour.
        if self.detect_limit_cycle(samples) {
            return DynamicsRegime::LimitCycleSuspect;
        }

        // Default: deltas shrinking but not yet at the quiescent floor — a
        // transient regime.
        DynamicsRegime::Settling
    }
}

impl DefaultRegimeClassifier {
    fn detect_limit_cycle(&self, samples: &VecDeque<TickMetrics>) -> bool {
        let window = self.limit_cycle_window.min(samples.len());
        if window < 4 {
            return false;
        }
        let recent: Vec<&TickMetrics> = samples.iter().rev().take(window).collect();
        // Compare sample[0] with sample[2], sample[1] with sample[3] etc.
        // — a period-2 limit cycle shows up as adjacent-pair similarity.
        let mut matches = 0;
        for i in 0..(window - 2) {
            let a = recent[i];
            let b = recent[i + 2];
            let de = (a.total_energy - b.total_energy).abs();
            let dd = (a.total_delta_norm - b.total_delta_norm).abs();
            if de <= self.limit_cycle_eps && dd <= self.limit_cycle_eps {
                matches += 1;
            }
        }
        // Require at least two paired matches inside the window so a single
        // coincidence does not flip the verdict.
        matches >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{EntityId, EntityState, SignalVector, StateVector, TickId, WorldVersion};
    use graph_tx::{DeltaProvenance, TickTransaction};

    fn tx_with_delta(before: Vec<f32>, after: Vec<f32>) -> TickTransaction {
        let mut tx = TickTransaction::simulate(TickId(0), WorldVersion(0));
        tx.record(
            EntityId(1),
            EntityState {
                internal: StateVector::new(before),
                emitted: SignalVector::default(),
                cooldown: 0,
            },
            EntityState {
                internal: StateVector::new(after),
                emitted: SignalVector::default(),
                cooldown: 0,
            },
            DeltaProvenance::default(),
        );
        tx
    }

    #[test]
    fn metrics_capture_delta_norm_and_sign_flip() {
        let tx = tx_with_delta(vec![1.0], vec![-1.0]);
        let m = TickMetrics::from_transaction(&tx);
        assert!((m.total_delta_norm - 2.0).abs() < 1e-6);
        assert!((m.max_delta_norm - 2.0).abs() < 1e-6);
        assert_eq!(m.sign_flips, 1);
        assert_eq!(m.touched_entities, 1);
    }

    #[test]
    fn empty_history_is_initializing() {
        let policy = DefaultRegimeClassifier::default();
        let history = MetricsHistory::new(8);
        assert_eq!(policy.classify(&history), DynamicsRegime::Initializing);
    }

    #[test]
    fn shrinking_to_zero_is_quiescent() {
        let policy = DefaultRegimeClassifier::default();
        let mut h = MetricsHistory::new(8);
        for delta in [1.0, 0.5, 0.25, 0.0] {
            h.push(TickMetrics {
                total_delta_norm: delta,
                total_energy: delta,
                ..TickMetrics::default()
            });
        }
        assert_eq!(policy.classify(&h), DynamicsRegime::Quiescent);
    }

    #[test]
    fn growing_energy_is_diverging() {
        let policy = DefaultRegimeClassifier::default();
        let mut h = MetricsHistory::new(8);
        for energy in [1.0, 2.0, 4.0, 8.0] {
            h.push(TickMetrics {
                total_delta_norm: energy,
                total_energy: energy,
                ..TickMetrics::default()
            });
        }
        assert_eq!(policy.classify(&h), DynamicsRegime::Diverging);
    }

    #[test]
    fn sustained_sign_flips_signal_oscillation() {
        let policy = DefaultRegimeClassifier::default();
        let mut h = MetricsHistory::new(8);
        for _ in 0..4 {
            h.push(TickMetrics {
                total_delta_norm: 1.0,
                total_energy: 1.0,
                sign_flips: 2,
                ..TickMetrics::default()
            });
        }
        assert_eq!(policy.classify(&h), DynamicsRegime::Oscillating);
    }

    #[test]
    fn period_two_pattern_is_limit_cycle() {
        let policy = DefaultRegimeClassifier::default();
        let mut h = MetricsHistory::new(8);
        for i in 0..6 {
            let delta = if i % 2 == 0 { 1.0 } else { 0.5 };
            h.push(TickMetrics {
                total_delta_norm: delta,
                total_energy: delta,
                ..TickMetrics::default()
            });
        }
        assert_eq!(policy.classify(&h), DynamicsRegime::LimitCycleSuspect);
    }

    #[test]
    fn ring_buffer_drops_oldest() {
        let mut h = MetricsHistory::new(2);
        h.push(TickMetrics {
            total_delta_norm: 1.0,
            ..TickMetrics::default()
        });
        h.push(TickMetrics {
            total_delta_norm: 2.0,
            ..TickMetrics::default()
        });
        h.push(TickMetrics {
            total_delta_norm: 3.0,
            ..TickMetrics::default()
        });
        assert_eq!(h.len(), 2);
        let first = h.samples().front().unwrap();
        assert!((first.total_delta_norm - 2.0).abs() < 1e-6);
    }
}
