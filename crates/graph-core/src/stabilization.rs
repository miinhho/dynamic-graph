//! Stabilization config — minimal guard rail post Phase 4.
//!
//! Phase 4 audit: saturation (None/Tanh/Clip) and trust_region were
//! knob-level alternatives with no benchmark requiring non-None saturation
//! or a bounded trust region. alpha blending (`(1-α)·before + α·after`) is
//! kept because it is the fundamental guard-rail axis; nothing replaces it
//! semantically. The per-kind `StabilizationConfig` struct keeps the
//! type for future reintroduction but exposes only `alpha`.

/// Per-kind guard-rail parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilizationConfig {
    /// Blending weight for state updates: `result = (1 − alpha) · before
    /// + alpha · after`. `1.0` = no blending (pass-through); `0.0` =
    ///   state never moves.
    ///
    /// **Default**: `1.0` (transparent — the guard rail is a no-op).
    /// Kept as the sole blend axis after Phase 4 (see
    /// `docs/complexity-audit.md`): `saturation` (None/Tanh/Clip) and
    /// `trust_region` were removed because no benchmark required
    /// non-default values; `alpha` is the fundamental "don't let state
    /// jump" knob with no semantic replacement.
    ///
    /// **Override when**: a program produces noisy or oscillating
    /// `after` states and you want exponential smoothing. Typical
    /// values: `0.1`–`0.5` for heavy smoothing; never set `0.0` unless
    /// you want the locus permanently frozen.
    pub alpha: f32,
}

impl Default for StabilizationConfig {
    fn default() -> Self {
        Self { alpha: 1.0 }
    }
}

impl StabilizationConfig {
    /// Apply the alpha blend: `result = (1 − alpha) · before + alpha · after`.
    pub fn stabilize(
        &self,
        before: &crate::state::StateVector,
        after: crate::state::StateVector,
    ) -> crate::state::StateVector {
        use crate::state::StateVector;

        let alpha = self.alpha;
        if (alpha - 1.0).abs() < 1e-9 {
            return after;
        }
        let dim = before.dim().max(after.dim());
        let blended: Vec<f32> = (0..dim)
            .map(|i| {
                let b = before.as_slice().get(i).copied().unwrap_or(0.0);
                let a = after.as_slice().get(i).copied().unwrap_or(0.0);
                (1.0 - alpha) * b + alpha * a
            })
            .collect();
        StateVector::from_slice(&blended)
    }
}

/// Stub kept for API stability across downstream callers. All variants now
/// collapse to `None` — the enum exists only so old config literals still
/// parse. Remove once all call sites stop mentioning it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SaturationMode {
    #[default]
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateVector;

    #[test]
    fn alpha_one_passes_through() {
        let cfg = StabilizationConfig::default();
        let before = StateVector::zeros(2);
        let after = StateVector::from_slice(&[1.0, 2.0]);
        assert_eq!(cfg.stabilize(&before, after.clone()), after);
    }

    #[test]
    fn alpha_half_blends() {
        let cfg = StabilizationConfig { alpha: 0.5 };
        let before = StateVector::from_slice(&[0.0, 0.0]);
        let after = StateVector::from_slice(&[2.0, 4.0]);
        let result = cfg.stabilize(&before, after);
        assert!((result.as_slice()[0] - 1.0).abs() < 1e-6);
        assert!((result.as_slice()[1] - 2.0).abs() < 1e-6);
    }
}
