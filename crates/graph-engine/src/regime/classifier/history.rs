use std::collections::VecDeque;

/// Per-batch summary statistics recorded by the engine.
#[derive(Debug, Clone, Default)]
pub struct BatchMetrics {
    /// Sum of `|after - before|.l2_norm()` over all committed changes
    /// in the batch. Zero if no changes fired.
    pub total_delta_norm: f32,
    /// Number of changes committed in this batch.
    pub change_count: u32,
    /// Sum of `after.l2_norm()` over all committed changes. Proxy for
    /// "how much energy is in the system" after this batch.
    pub total_energy: f32,
}

impl BatchMetrics {
    /// Build from the changes committed in one batch.
    pub fn from_changes<'a>(changes: impl Iterator<Item = &'a graph_core::Change>) -> Self {
        let mut metrics = Self::default();
        for change in changes {
            metrics.change_count += 1;
            metrics.total_delta_norm += delta_norm(change);
            metrics.total_energy += change.after.l2_norm();
        }
        metrics
    }
}

fn delta_norm(change: &graph_core::Change) -> f32 {
    let before = &change.before;
    let after = &change.after;
    let dim = before.dim().max(after.dim());
    let delta_sq: f32 = (0..dim)
        .map(|i| {
            let b = before.as_slice().get(i).copied().unwrap_or(0.0);
            let a = after.as_slice().get(i).copied().unwrap_or(0.0);
            (a - b).powi(2)
        })
        .sum();
    delta_sq.sqrt()
}

/// Ring buffer of recent per-batch metrics.
#[derive(Debug, Clone)]
pub struct BatchHistory {
    window: usize,
    history: VecDeque<BatchMetrics>,
}

impl BatchHistory {
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            history: VecDeque::with_capacity(window),
        }
    }

    pub fn push(&mut self, metrics: BatchMetrics) {
        if self.history.len() >= self.window {
            self.history.pop_front();
        }
        self.history.push_back(metrics);
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.history.len() >= self.window
    }

    pub fn window(&self) -> usize {
        self.window
    }

    pub fn iter(&self) -> impl Iterator<Item = &BatchMetrics> {
        self.history.iter()
    }
}
