//! Adaptive guard rail facade.

mod framework;
mod guard_rail;
mod regime_alpha;

pub use framework::{Learnable, PerKindLearnable};
pub use guard_rail::{AdaptiveConfig, AdaptiveGuardRail};

#[cfg(test)]
mod tests {
    use graph_core::InfluenceKindId;

    use super::{AdaptiveConfig, AdaptiveGuardRail};
    use crate::regime::DynamicsRegime;

    fn rail() -> (AdaptiveGuardRail, InfluenceKindId) {
        let mut g = AdaptiveGuardRail::new(AdaptiveConfig::default());
        let k = InfluenceKindId(1);
        g.register(k);
        (g, k)
    }

    #[test]
    fn starts_at_max_scale() {
        let (g, k) = rail();
        assert!((g.current_scale(k) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn diverging_shrinks_to_floor() {
        let (g, k) = rail();
        for _ in 0..30 {
            g.observe(k, DynamicsRegime::Diverging);
        }
        assert!((g.current_scale(k) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn quiescent_recovers_to_ceiling() {
        let (g, k) = rail();
        for _ in 0..10 {
            g.observe(k, DynamicsRegime::Diverging);
        }
        assert!(g.current_scale(k) < 0.5);
        for _ in 0..200 {
            g.observe(k, DynamicsRegime::Quiescent);
        }
        assert!((g.current_scale(k) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn oscillating_does_not_shrink() {
        let (g, k) = rail();
        let before = g.current_scale(k);
        g.observe(k, DynamicsRegime::Oscillating);
        g.observe(k, DynamicsRegime::LimitCycleSuspect);
        g.observe(k, DynamicsRegime::Settling);
        g.observe(k, DynamicsRegime::Initializing);
        assert!((g.current_scale(k) - before).abs() < 1e-6);
    }

    #[test]
    fn reset_restores_max_scale() {
        let (g, k) = rail();
        for _ in 0..10 {
            g.observe(k, DynamicsRegime::Diverging);
        }
        assert!(g.current_scale(k) < 0.5);
        g.reset(k);
        assert!((g.current_scale(k) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn effective_stabilization_config_scales_alpha() {
        let (g, k) = rail();
        for _ in 0..2 {
            g.observe(k, DynamicsRegime::Diverging);
        }
        let base = graph_core::StabilizationConfig { alpha: 0.8 };
        let effective = g.effective_stabilization_config(k, &base);
        assert!(
            (effective.alpha - 0.2).abs() < 1e-5,
            "alpha={}",
            effective.alpha
        );
    }

    #[test]
    fn unregistered_kind_returns_max_scale() {
        let g = AdaptiveGuardRail::new(AdaptiveConfig::default());
        assert!((g.current_scale(InfluenceKindId(99)) - 1.0).abs() < 1e-6);
    }
}
