//! Stabilization config types shared by graph-core.
//!
//! Stabilization is a *guard rail*, not a goal — per `docs/identity.md`
//! the engine pushes back only against divergence. These types express
//! the per-kind tuning knobs that shape that guard rail.

/// How to saturate a state vector slot before committing it.
///
/// Applied per-slot independently so different dimensions can saturate
/// differently if the user sets up kind-specific configs for each.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SaturationMode {
    /// No saturation. Values can grow unboundedly — only the trust
    /// region (if set) limits magnitude.
    #[default]
    None,
    /// Soft saturation via `tanh`. Slots are mapped through
    /// `tanh(v / scale) * scale` where `scale` is the trust region
    /// radius (or 1.0 if no trust region is set). Asymptotically
    /// bounded, smooth, and differentiable.
    Tanh,
    /// Hard clip to `[-trust_region, trust_region]`. Not smooth, but
    /// gives a strict bound. Requires `trust_region` to be set; if not,
    /// falls back to `None`.
    Clip,
}

/// Per-kind guard-rail parameters. Stored inside `InfluenceKindConfig`
/// so each influence kind has an independently tunable guard rail.
#[derive(Debug, Clone, PartialEq)]
pub struct StabilizationConfig {
    /// Blending weight for state updates. The committed `after` state
    /// is `(1 - alpha) * before + alpha * proposed`. At `alpha = 1.0`
    /// the proposed value is accepted unchanged; at `alpha = 0.0` the
    /// state never moves. Default: `1.0` (no blending).
    pub alpha: f32,
    /// Saturation applied to `after` *before* the alpha blend. Default:
    /// `SaturationMode::None`.
    pub saturation: SaturationMode,
    /// Optional maximum L2 magnitude for the `after` vector. If the
    /// proposed vector exceeds this, it is rescaled to the boundary
    /// before the alpha blend. `None` means no trust region.
    pub trust_region: Option<f32>,
}

impl Default for StabilizationConfig {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            saturation: SaturationMode::None,
            trust_region: None,
        }
    }
}

impl StabilizationConfig {
    /// Apply the guard rail to a proposed `after` state vector given
    /// the locus's current `before` state.
    ///
    /// Steps (in order):
    /// 1. Trust-region rescale: if `||after|| > trust_region`, rescale.
    /// 2. Saturation: apply `saturation` mode slot-by-slot.
    /// 3. Alpha blend: `result = (1 - alpha) * before + alpha * after`.
    pub fn stabilize(
        &self,
        before: &crate::state::StateVector,
        after: crate::state::StateVector,
    ) -> crate::state::StateVector {
        use crate::state::StateVector;

        // 1. Trust-region rescale.
        let after = if let Some(radius) = self.trust_region {
            let norm = after.l2_norm();
            if norm > radius && norm > 0.0 {
                let scale = radius / norm;
                StateVector::from_slice(
                    &after.as_slice().iter().map(|v| v * scale).collect::<Vec<_>>(),
                )
            } else {
                after
            }
        } else {
            after
        };

        // 2. Saturation.
        let scale = self.trust_region.unwrap_or(1.0).max(1e-9);
        let after = match self.saturation {
            SaturationMode::None => after,
            SaturationMode::Tanh => StateVector::from_slice(
                &after
                    .as_slice()
                    .iter()
                    .map(|&v| (v / scale).tanh() * scale)
                    .collect::<Vec<_>>(),
            ),
            SaturationMode::Clip => StateVector::from_slice(
                &after
                    .as_slice()
                    .iter()
                    .map(|&v| v.clamp(-scale, scale))
                    .collect::<Vec<_>>(),
            ),
        };

        // 3. Alpha blend.
        let alpha = self.alpha;
        if (alpha - 1.0).abs() < 1e-9 {
            return after; // fast path: no blending needed
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
        let cfg = StabilizationConfig { alpha: 0.5, ..Default::default() };
        let before = StateVector::from_slice(&[0.0, 0.0]);
        let after = StateVector::from_slice(&[2.0, 4.0]);
        let result = cfg.stabilize(&before, after);
        assert!((result.as_slice()[0] - 1.0).abs() < 1e-6);
        assert!((result.as_slice()[1] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn trust_region_rescales_large_vector() {
        let cfg = StabilizationConfig {
            trust_region: Some(1.0),
            ..Default::default()
        };
        let before = StateVector::zeros(1);
        let after = StateVector::from_slice(&[10.0]);
        let result = cfg.stabilize(&before, after);
        assert!((result.l2_norm() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn tanh_saturation_is_bounded() {
        let cfg = StabilizationConfig {
            saturation: SaturationMode::Tanh,
            trust_region: Some(1.0),
            ..Default::default()
        };
        let before = StateVector::zeros(1);
        let after = StateVector::from_slice(&[100.0]);
        let result = cfg.stabilize(&before, after);
        // tanh(100) ≈ 1; result should be ≤ 1.0.
        assert!(result.as_slice()[0] <= 1.0 + 1e-6);
    }

    #[test]
    fn clip_saturation_hard_bounds() {
        let cfg = StabilizationConfig {
            saturation: SaturationMode::Clip,
            trust_region: Some(2.0),
            ..Default::default()
        };
        let before = StateVector::zeros(1);
        let after = StateVector::from_slice(&[5.0]);
        let result = cfg.stabilize(&before, after);
        assert!((result.as_slice()[0] - 2.0).abs() < 1e-6);
    }
}
