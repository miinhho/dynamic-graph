use graph_core::{InfluenceKindId, StabilizationConfig};

use crate::regime::DynamicsRegime;

use super::framework::PerKindLearnable;
use super::regime_alpha::RegimeAlphaScale;

#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveConfig;

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

    pub fn effective_alpha(&self, kind: InfluenceKindId, base_alpha: f32) -> f32 {
        base_alpha * self.current_scale(kind)
    }

    pub fn effective_stabilization_config(
        &self,
        kind: InfluenceKindId,
        base_config: &StabilizationConfig,
    ) -> StabilizationConfig {
        StabilizationConfig {
            alpha: self.effective_alpha(kind, base_config.alpha),
        }
    }

    pub fn reset(&self, kind: InfluenceKindId) {
        self.inner.reset(kind);
    }

    pub fn reset_all(&self) {
        self.inner.reset_all();
    }
}
