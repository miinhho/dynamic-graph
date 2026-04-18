//! Per-kind learnable state framework — `Learnable` trait + `PerKindLearnable<L>`.
//!
//! This file generalises what `AdaptiveGuardRail` used to do privately.
//! The pattern "for each `InfluenceKindId`, observe a stream of
//! observations and maintain a per-kind `f32` state" recurs whenever
//! an engine parameter is driven by what the engine sees. By factoring
//! it out, future auto-tuning work (e.g. a reopened Phase 9 where
//! `plasticity.learning_rate` is observation-driven) plugs in by writing
//! a new `impl Learnable` rather than re-implementing the atomic-state /
//! register-kind scaffolding.
//!
//! ## Framework
//!
//! - [`Learnable`] — describes the *semantics* of one auto-tuned value:
//!   what observation type it consumes, what initial value a fresh kind
//!   starts with, the [floor, ceiling] clamping range, and the update
//!   rule.
//! - [`PerKindLearnable<L>`] — owns the `FxHashMap<InfluenceKindId,
//!   AtomicU32>` state. Generic over any [`Learnable`]. `observe`,
//!   `current`, `register`, `reset`, and `reset_all` delegate to `L`'s
//!   semantics.
//!
//! ## First concrete instance: `RegimeAlphaScale`
//!
//! The historical `AdaptiveGuardRail` behaviour survives intact, now as
//! a thin newtype around `PerKindLearnable<RegimeAlphaScale>`. The
//! regime-classifier → alpha-scale mapping (Diverging shrinks,
//! Quiescent recovers) is captured by `RegimeAlphaScale`'s
//! [`Learnable`] impl.
//!
//! ## Adding a second instance
//!
//! ```ignore
//! struct MyKnob;
//! impl Learnable for MyKnob {
//!     type Observation = MyObservation;
//!     fn initial() -> f32 { … }
//!     fn clamp_range() -> (f32, f32) { … }
//!     fn step(current: f32, obs: Self::Observation) -> f32 { … }
//! }
//!
//! let learner: PerKindLearnable<MyKnob> = PerKindLearnable::new();
//! ```
//!
//! No new atomic-state or register-kind boilerplate — the framework
//! carries it.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

use graph_core::InfluenceKindId;
use rustc_hash::FxHashMap;

use super::DynamicsRegime;

// ── Framework ───────────────────────────────────────────────────────────────

/// A value learned per `InfluenceKindId` from a stream of observations.
///
/// Implementations describe *what* they consume and *how* each
/// observation updates the state. The generic [`PerKindLearnable<L>`]
/// container handles atomic storage, per-kind registration, and
/// clamping.
pub trait Learnable {
    /// What [`PerKindLearnable::observe`] accepts. For
    /// [`RegimeAlphaScale`] this is [`DynamicsRegime`]; for a future
    /// learning-rate learner it might be a distribution summary.
    type Observation: Copy;

    /// Initial value when a kind is first registered.
    fn initial() -> f32;

    /// `(floor, ceiling)` applied after each update.
    fn clamp_range() -> (f32, f32);

    /// Update rule: given the current value and an observation,
    /// return the next value (before clamping).
    fn step(current: f32, obs: Self::Observation) -> f32;
}

/// Per-kind atomic `f32` state driven by a [`Learnable`] semantics.
///
/// Maintains one `AtomicU32` (bit-pattern of a `f32`) per registered
/// kind. `observe` can be called from any thread without a mutex —
/// it uses `Ordering::Relaxed` because the value is a soft control
/// signal, not a synchronisation fence.
pub struct PerKindLearnable<L: Learnable> {
    scales: FxHashMap<InfluenceKindId, AtomicU32>,
    _phantom: PhantomData<L>,
}

impl<L: Learnable> Default for PerKindLearnable<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: Learnable> PerKindLearnable<L> {
    pub fn new() -> Self {
        Self {
            scales: FxHashMap::default(),
            _phantom: PhantomData,
        }
    }

    /// Pre-register a kind so its value is initialised before the first
    /// observation. Optional — `observe` auto-skips unregistered kinds
    /// to avoid crashing on first-seen kinds.
    pub fn register(&mut self, kind: InfluenceKindId) {
        self.scales
            .entry(kind)
            .or_insert_with(|| AtomicU32::new(L::initial().to_bits()));
    }

    /// Apply one observation to a kind's state.
    ///
    /// No-op if the kind is unregistered (soft control signal — a new
    /// kind should not crash the adaptive loop).
    pub fn observe(&self, kind: InfluenceKindId, obs: L::Observation) {
        let Some(atomic) = self.scales.get(&kind) else {
            return;
        };
        let current = f32::from_bits(atomic.load(Ordering::Relaxed));
        let next = L::step(current, obs);
        let (floor, ceil) = L::clamp_range();
        atomic.store(next.clamp(floor, ceil).to_bits(), Ordering::Relaxed);
    }

    /// Current value for `kind`. Returns `L::initial()` if the kind is
    /// not registered (safe default — unobserved = starting value).
    pub fn current(&self, kind: InfluenceKindId) -> f32 {
        self.scales
            .get(&kind)
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)))
            .unwrap_or_else(L::initial)
    }

    /// Reset one kind's value back to [`Learnable::initial`].
    pub fn reset(&self, kind: InfluenceKindId) {
        if let Some(atomic) = self.scales.get(&kind) {
            atomic.store(L::initial().to_bits(), Ordering::Relaxed);
        }
    }

    /// Reset all registered kinds to [`Learnable::initial`].
    pub fn reset_all(&self) {
        for atomic in self.scales.values() {
            atomic.store(L::initial().to_bits(), Ordering::Relaxed);
        }
    }
}

// ── First concrete `Learnable`: alpha scale driven by regime ────────────────

/// The historical [`AdaptiveGuardRail`] semantics, expressed as a
/// [`Learnable`]:
///
/// - Only [`DynamicsRegime::Diverging`] shrinks the scale (tighten
///   the guard rail).
/// - Only [`DynamicsRegime::Quiescent`] recovers the scale (loosen).
/// - All other regimes leave the scale alone.
///
/// Constants are hard-coded (Phase 7). No benchmark used non-default
/// values.
pub struct RegimeAlphaScale;

const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 1.0;
const SHRINK_FACTOR: f32 = 0.5;
const RECOVERY_FACTOR: f32 = 1.1;

impl Learnable for RegimeAlphaScale {
    type Observation = DynamicsRegime;

    fn initial() -> f32 {
        MAX_SCALE
    }

    fn clamp_range() -> (f32, f32) {
        (MIN_SCALE, MAX_SCALE)
    }

    fn step(current: f32, obs: DynamicsRegime) -> f32 {
        match obs {
            DynamicsRegime::Diverging => current * SHRINK_FACTOR,
            DynamicsRegime::Quiescent => current * RECOVERY_FACTOR,
            DynamicsRegime::Initializing
            | DynamicsRegime::Settling
            | DynamicsRegime::Oscillating
            | DynamicsRegime::LimitCycleSuspect => current,
        }
    }
}

// ── Public surface: AdaptiveConfig (ZST) + AdaptiveGuardRail newtype ────────

/// Adaptive-feedback knobs collapsed into hard-coded constants (Phase 7).
/// Kept as a ZST so call sites that pass `AdaptiveConfig::default()`
/// continue to compile; the constants live inside [`RegimeAlphaScale`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveConfig;

/// Per-kind adaptive alpha scales — historical API preserved.
///
/// Thin newtype over `PerKindLearnable<RegimeAlphaScale>`. Methods
/// delegate straight through; the stabilization-config helpers
/// (`effective_alpha`, `effective_stabilization_config`) live here
/// because they are not part of the generic framework.
pub struct AdaptiveGuardRail {
    inner: PerKindLearnable<RegimeAlphaScale>,
}

impl AdaptiveGuardRail {
    pub fn new(_config: AdaptiveConfig) -> Self {
        Self {
            inner: PerKindLearnable::new(),
        }
    }

    pub fn register(&mut self, kind: InfluenceKindId) {
        self.inner.register(kind);
    }

    pub fn observe(&self, kind: InfluenceKindId, regime: DynamicsRegime) {
        self.inner.observe(kind, regime);
    }

    pub fn current_scale(&self, kind: InfluenceKindId) -> f32 {
        self.inner.current(kind)
    }

    /// Effective alpha: `base_alpha * current_scale(kind)`.
    pub fn effective_alpha(&self, kind: InfluenceKindId, base_alpha: f32) -> f32 {
        base_alpha * self.current_scale(kind)
    }

    /// Return a copy of `base_config` with `alpha` replaced by
    /// `effective_alpha(kind, base_config.alpha)`.
    pub fn effective_stabilization_config(
        &self,
        kind: InfluenceKindId,
        base_config: &graph_core::StabilizationConfig,
    ) -> graph_core::StabilizationConfig {
        graph_core::StabilizationConfig {
            alpha: self.effective_alpha(kind, base_config.alpha),
            ..*base_config
        }
    }

    pub fn reset(&self, kind: InfluenceKindId) {
        self.inner.reset(kind);
    }

    pub fn reset_all(&self) {
        self.inner.reset_all();
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
