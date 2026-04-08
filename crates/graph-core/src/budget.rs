#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmissionBudget {
    pub max_targets_per_tick: usize,
    pub max_signal_norm: f32,
}

impl Default for EmissionBudget {
    fn default() -> Self {
        Self {
            max_targets_per_tick: usize::MAX,
            max_signal_norm: f32::MAX,
        }
    }
}
