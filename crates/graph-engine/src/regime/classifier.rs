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

mod default;
mod history;

pub use default::DefaultRegimeClassifier;
pub use history::{BatchHistory, BatchMetrics};

#[cfg(test)]
mod tests {
    use super::{BatchHistory, BatchMetrics, DefaultRegimeClassifier};
    use crate::regime::{DynamicsRegime, RegimeClassifier};
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
        assert_eq!(
            DefaultRegimeClassifier::default().classify(&h),
            DynamicsRegime::Quiescent
        );
    }

    #[test]
    fn settling_on_decreasing_energy() {
        let mut h = BatchHistory::new(4);
        for e in [4.0, 3.0, 2.0, 1.0] {
            push_energy(&mut h, e);
        }
        assert_eq!(
            DefaultRegimeClassifier::default().classify(&h),
            DynamicsRegime::Settling
        );
    }

    #[test]
    fn diverging_on_monotone_increase() {
        let mut h = BatchHistory::new(4);
        for e in [1.0, 2.0, 3.0, 4.0] {
            push_energy(&mut h, e);
        }
        assert_eq!(
            DefaultRegimeClassifier::default().classify(&h),
            DynamicsRegime::Diverging
        );
    }

    #[test]
    fn oscillating_on_alternating_energy() {
        let mut h = BatchHistory::new(6);
        for e in [1.0, 2.0, 1.1, 1.9, 1.0, 2.0] {
            push_energy(&mut h, e);
        }
        let regime = DefaultRegimeClassifier::default().classify(&h);
        assert!(
            matches!(
                regime,
                DynamicsRegime::Oscillating | DynamicsRegime::LimitCycleSuspect
            ),
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
            wall_time: None,
            metadata: None,
        };
        let m = BatchMetrics::from_changes(std::iter::once(&change));
        // delta norm: ||(3,4) - (0,0)|| = 5
        assert!((m.total_delta_norm - 5.0).abs() < 1e-5);
        // energy: ||(3,4)|| = 5
        assert!((m.total_energy - 5.0).abs() < 1e-5);
        assert_eq!(m.change_count, 1);
    }
}
