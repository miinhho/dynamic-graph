//! Adaptive guard rail driven by regime feedback.
//!
//! See `docs/identity.md` for the framing this module operates under. The
//! short version: stabilization is a guard rail, not a goal. This module
//! lets the guard rail react to *only* the one regime that needs
//! counteracting (`Diverging`), relax in the one regime where the system is
//! producing nothing observable (`Quiescent`), and stay out of the way in
//! every other regime — including `Oscillating` and `LimitCycleSuspect`,
//! which are valid dynamical modes the engine is here to expose.
//!
//! Implementation: a wrapper around [`BasicStabilizer`] that owns a current
//! alpha scale and updates it from a [`DynamicsRegime`] fed in between ticks.
//! The scale lives in an [`AtomicU32`] holding the bit pattern of an `f32`,
//! so the engine can keep using `&self`-only stabilizer methods on parallel
//! iterators.

use std::sync::atomic::{AtomicU32, Ordering};

use graph_core::{Emission, Entity, EntityState};

use crate::regime::DynamicsRegime;
use crate::stabilizer::{BasicStabilizer, Stabilizer};

/// Tunable knobs for [`AdaptiveStabilizer`]'s feedback loop.
#[derive(Debug, Clone, Copy)]
pub struct AdaptiveConfig {
    /// Floor for the alpha multiplier. The effective alpha never falls below
    /// `base.alpha * min_scale`.
    pub min_scale: f32,
    /// Ceiling for the alpha multiplier. Recovery never raises the effective
    /// alpha above `base.alpha * max_scale`.
    pub max_scale: f32,
    /// Multiplier applied to the current scale when [`DynamicsRegime::Diverging`]
    /// is observed. Should be in `(0, 1)`.
    pub shrink_factor: f32,
    /// Multiplier applied to the current scale when [`DynamicsRegime::Quiescent`]
    /// is observed. Should be `>= 1`.
    pub recovery_factor: f32,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            min_scale: 0.1,
            max_scale: 1.0,
            shrink_factor: 0.5,
            recovery_factor: 1.1,
        }
    }
}

/// Wrap a [`BasicStabilizer`] with regime-driven alpha adaptation.
#[derive(Debug)]
pub struct AdaptiveStabilizer {
    base: BasicStabilizer,
    config: AdaptiveConfig,
    /// Bit pattern of the current alpha scale `f32`. Read/written with
    /// `Relaxed` ordering — this is a soft control signal, not a fence.
    scale_bits: AtomicU32,
}

impl AdaptiveStabilizer {
    pub fn new(base: BasicStabilizer, config: AdaptiveConfig) -> Self {
        let initial = config.max_scale.clamp(config.min_scale, config.max_scale);
        Self {
            base,
            config,
            scale_bits: AtomicU32::new(initial.to_bits()),
        }
    }

    /// Convenience constructor with [`AdaptiveConfig::default`].
    pub fn from_base(base: BasicStabilizer) -> Self {
        Self::new(base, AdaptiveConfig::default())
    }

    /// Current alpha multiplier (in `[min_scale, max_scale]`).
    pub fn current_scale(&self) -> f32 {
        f32::from_bits(self.scale_bits.load(Ordering::Relaxed))
    }

    /// Effective alpha actually fed to the inner policy (`base.alpha * scale`).
    pub fn effective_alpha(&self) -> f32 {
        self.base.alpha * self.current_scale()
    }

    /// Apply a regime observation. Call between ticks with the classifier's
    /// most recent verdict.
    ///
    /// Only [`DynamicsRegime::Diverging`] tightens the guard rail; only
    /// [`DynamicsRegime::Quiescent`] relaxes it. Every other regime —
    /// including `Oscillating` and `LimitCycleSuspect` — is left alone,
    /// because those regimes are valid observation modes, not failures to
    /// suppress. See `docs/identity.md` §3 and §4.
    pub fn observe(&self, regime: DynamicsRegime) {
        let current = self.current_scale();
        let next = match regime {
            DynamicsRegime::Diverging => current * self.config.shrink_factor,
            DynamicsRegime::Quiescent => current * self.config.recovery_factor,
            DynamicsRegime::Initializing
            | DynamicsRegime::Settling
            | DynamicsRegime::Oscillating
            | DynamicsRegime::LimitCycleSuspect => current,
        };
        let clamped = next.clamp(self.config.min_scale, self.config.max_scale);
        self.scale_bits
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Forcefully reset the alpha scale back to `max_scale`. Useful after a
    /// world reset or external command that invalidates prior history.
    pub fn reset(&self) {
        self.scale_bits
            .store(self.config.max_scale.to_bits(), Ordering::Relaxed);
    }

    fn effective_policy(&self) -> BasicStabilizer {
        BasicStabilizer {
            alpha: self.effective_alpha(),
            ..self.base
        }
    }
}

impl Stabilizer for AdaptiveStabilizer {
    fn stabilize_emission(&self, source: &Entity, emission: Emission) -> Emission {
        Stabilizer::stabilize_emission(&self.effective_policy(), source, emission)
    }

    fn stabilize_state(&self, source: &Entity, raw: EntityState) -> EntityState {
        Stabilizer::stabilize_state(&self.effective_policy(), source, raw)
    }

    fn relaxation_alpha(&self) -> f32 {
        self.effective_alpha()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilizer::SaturationMode;

    fn base() -> BasicStabilizer {
        BasicStabilizer {
            alpha: 0.8,
            decay: 1.0,
            saturation: SaturationMode::None,
            trust_region: None,
        }
    }

    #[test]
    fn starts_at_max_scale() {
        let s = AdaptiveStabilizer::from_base(base());
        assert!((s.current_scale() - 1.0).abs() < 1e-6);
        assert!((s.effective_alpha() - 0.8).abs() < 1e-6);
    }

    #[test]
    fn diverging_shrinks_aggressively_until_floor() {
        let s = AdaptiveStabilizer::from_base(base());
        for _ in 0..20 {
            s.observe(DynamicsRegime::Diverging);
        }
        assert!((s.current_scale() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn quiescent_recovers_until_ceiling() {
        let s = AdaptiveStabilizer::from_base(base());
        for _ in 0..5 {
            s.observe(DynamicsRegime::Diverging);
        }
        for _ in 0..200 {
            s.observe(DynamicsRegime::Quiescent);
        }
        assert!((s.current_scale() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn oscillation_is_a_valid_regime_and_does_not_shrink() {
        let s = AdaptiveStabilizer::from_base(base());
        let before = s.current_scale();
        s.observe(DynamicsRegime::Oscillating);
        s.observe(DynamicsRegime::Oscillating);
        assert!(
            (s.current_scale() - before).abs() < 1e-6,
            "oscillation must not move the guard rail: before {before}, after {}",
            s.current_scale()
        );
    }

    #[test]
    fn limit_cycle_is_a_valid_regime_and_does_not_shrink() {
        let s = AdaptiveStabilizer::from_base(base());
        let before = s.current_scale();
        s.observe(DynamicsRegime::LimitCycleSuspect);
        assert!((s.current_scale() - before).abs() < 1e-6);
    }

    #[test]
    fn settling_and_initializing_are_no_ops() {
        let s = AdaptiveStabilizer::from_base(base());
        let before = s.current_scale();
        s.observe(DynamicsRegime::Settling);
        s.observe(DynamicsRegime::Initializing);
        assert!((s.current_scale() - before).abs() < 1e-6);
    }

    #[test]
    fn reset_restores_max_scale() {
        let s = AdaptiveStabilizer::from_base(base());
        for _ in 0..10 {
            s.observe(DynamicsRegime::Diverging);
        }
        assert!(s.current_scale() < 0.5);
        s.reset();
        assert!((s.current_scale() - 1.0).abs() < 1e-6);
    }
}
