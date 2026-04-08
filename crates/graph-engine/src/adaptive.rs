//! Per-kind adaptive guard rail.
//!
//! Ported from `AdaptiveStabilizer` in phase 1+2, now operating per
//! `InfluenceKindId` instead of globally. The framing is unchanged
//! from `docs/identity.md` and `docs/adaptive.rs`:
//!
//! - Only `Diverging` shrinks the scale → tightens the guard rail.
//! - Only `Quiescent` recovers the scale → loosens the guard rail.
//! - All other regimes (Oscillating, LimitCycleSuspect, Settling,
//!   Initializing) leave the scale alone — those are valid observation
//!   modes, not failures to suppress.
//!
//! Each kind has its own `AtomicU32`-held scale (bit-pattern of an
//! f32), so `observe()` can be called from any thread without a mutex.
//!
//! ## Integration pattern
//!
//! ```text
//! let guard = AdaptiveGuardRail::new(AdaptiveConfig::default());
//! // after each tick:
//! let metrics = engine.last_tick_metrics();
//! history.push(metrics);
//! let regime = classifier.classify(&history);
//! guard.observe(kind_id, regime);
//! // effective alpha for the next tick:
//! let alpha = guard.effective_alpha(kind_id, base_alpha);
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use graph_core::InfluenceKindId;

use crate::regime::DynamicsRegime;

/// Tuning knobs for the adaptive feedback loop.
#[derive(Debug, Clone, Copy)]
pub struct AdaptiveConfig {
    /// Floor for the alpha multiplier. Effective alpha never falls
    /// below `base_alpha * min_scale`.
    pub min_scale: f32,
    /// Ceiling for the alpha multiplier. Recovery never raises
    /// effective alpha above `base_alpha * max_scale`.
    pub max_scale: f32,
    /// Multiplier applied when `Diverging` is observed. Must be in
    /// `(0, 1)`.
    pub shrink_factor: f32,
    /// Multiplier applied when `Quiescent` is observed. Must be
    /// `>= 1`.
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

/// Per-kind adaptive alpha scales.
///
/// Maintains one `AtomicU32` (bit-pattern of a `f32`) per registered
/// kind. Kinds are initialized to `config.max_scale` on first access
/// via `observe` or `effective_alpha`.
pub struct AdaptiveGuardRail {
    config: AdaptiveConfig,
    scales: HashMap<InfluenceKindId, AtomicU32>,
}

impl AdaptiveGuardRail {
    pub fn new(config: AdaptiveConfig) -> Self {
        Self {
            config,
            scales: HashMap::new(),
        }
    }

    /// Pre-register a kind so its scale is initialized before the
    /// first tick. Optional — the guard rail auto-initializes on first
    /// `observe` or `effective_alpha` call too, but explicit
    /// registration makes the "known kinds" set visible.
    pub fn register(&mut self, kind: InfluenceKindId) {
        self.scales
            .entry(kind)
            .or_insert_with(|| AtomicU32::new(self.config.max_scale.to_bits()));
    }

    /// Apply a regime observation to one kind's scale.
    ///
    /// Safe to call concurrently (uses `Relaxed` atomics — this is a
    /// soft control signal, not a synchronization fence).
    pub fn observe(&self, kind: InfluenceKindId, regime: DynamicsRegime) {
        // If the kind isn't registered yet we silently skip — a new
        // kind that hasn't been seen in the registry shouldn't crash
        // the adaptive loop.
        let Some(atomic) = self.scales.get(&kind) else {
            return;
        };
        let current = f32::from_bits(atomic.load(Ordering::Relaxed));
        let next = match regime {
            DynamicsRegime::Diverging => current * self.config.shrink_factor,
            DynamicsRegime::Quiescent => current * self.config.recovery_factor,
            DynamicsRegime::Initializing
            | DynamicsRegime::Settling
            | DynamicsRegime::Oscillating
            | DynamicsRegime::LimitCycleSuspect => current,
        };
        let clamped = next.clamp(self.config.min_scale, self.config.max_scale);
        atomic.store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Current scale for `kind`. Returns `max_scale` if the kind is
    /// not yet registered (safe default — unobserved = no tightening).
    pub fn current_scale(&self, kind: InfluenceKindId) -> f32 {
        self.scales
            .get(&kind)
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)))
            .unwrap_or(self.config.max_scale)
    }

    /// Effective alpha: `base_alpha * current_scale(kind)`.
    ///
    /// The caller is responsible for applying this to the kind's
    /// `StabilizationConfig.alpha` before passing the config to the
    /// engine, or by calling `effective_stabilization_config()`.
    pub fn effective_alpha(&self, kind: InfluenceKindId, base_alpha: f32) -> f32 {
        base_alpha * self.current_scale(kind)
    }

    /// Return a copy of `base_config` with alpha replaced by
    /// `effective_alpha(kind, base_config.alpha)`. Convenience wrapper
    /// for building the per-tick config to hand to the engine.
    pub fn effective_stabilization_config(
        &self,
        kind: InfluenceKindId,
        base_config: &graph_core::StabilizationConfig,
    ) -> graph_core::StabilizationConfig {
        graph_core::StabilizationConfig {
            alpha: self.effective_alpha(kind, base_config.alpha),
            ..base_config.clone()
        }
    }

    /// Reset one kind's scale back to `max_scale`. Useful after a
    /// world reset or an external command that invalidates prior
    /// history (mirrors `AdaptiveStabilizer::reset()` from phase 1+2).
    pub fn reset(&self, kind: InfluenceKindId) {
        if let Some(atomic) = self.scales.get(&kind) {
            atomic.store(self.config.max_scale.to_bits(), Ordering::Relaxed);
        }
    }

    /// Reset all registered kinds to `max_scale`.
    pub fn reset_all(&self) {
        for atomic in self.scales.values() {
            atomic.store(self.config.max_scale.to_bits(), Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // scale = 1.0 * 0.5 * 0.5 = 0.25
        let base = graph_core::StabilizationConfig {
            alpha: 0.8,
            ..Default::default()
        };
        let effective = g.effective_stabilization_config(k, &base);
        assert!((effective.alpha - 0.2).abs() < 1e-5, "alpha={}", effective.alpha);
    }

    #[test]
    fn unregistered_kind_returns_max_scale() {
        let g = AdaptiveGuardRail::new(AdaptiveConfig::default());
        assert!((g.current_scale(InfluenceKindId(99)) - 1.0).abs() < 1e-6);
    }
}
