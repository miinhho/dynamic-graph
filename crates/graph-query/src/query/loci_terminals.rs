use graph_core::LocusId;

use super::LociQuery;

impl<'w> LociQuery<'w> {
    /// Collect matching loci as `&Locus` references.
    pub fn collect(self) -> Vec<&'w graph_core::Locus> {
        self.loci
    }

    /// Collect just the `LocusId`s.
    pub fn ids(self) -> Vec<LocusId> {
        self.loci.into_iter().map(|l| l.id).collect()
    }

    /// Number of matching loci.
    pub fn count(self) -> usize {
        self.loci.len()
    }

    /// First matching locus (in current order), or `None` if the set is empty.
    pub fn first(self) -> Option<&'w graph_core::Locus> {
        self.loci.into_iter().next()
    }

    /// `true` when no loci match the current constraints.
    pub fn is_empty(&self) -> bool {
        self.loci.is_empty()
    }

    /// Sum of `state[slot]` across all matching loci.
    pub fn sum_state_slot(self, slot: usize) -> f32 {
        self.loci
            .iter()
            .map(|l| l.state.as_slice().get(slot).copied().unwrap_or(0.0))
            .sum()
    }

    /// Mean of `state[slot]` across all matching loci that have the slot.
    pub fn mean_state_slot(self, slot: usize) -> Option<f32> {
        let values: Vec<f32> = self
            .loci
            .iter()
            .filter_map(|l| l.state.as_slice().get(slot).copied())
            .collect();
        let count = values.len();
        (count > 0).then(|| values.iter().sum::<f32>() / count as f32)
    }

    /// Maximum of `state[slot]` across all matching loci that have the slot.
    pub fn max_state_slot(self, slot: usize) -> Option<f32> {
        self.loci
            .iter()
            .filter_map(|l| l.state.as_slice().get(slot).copied())
            .reduce(f32::max)
    }
}
