use graph_core::{Cohere, CohereId};

use super::{CohereQuery, cohere_filters};

impl<'w> CohereQuery<'w> {
    /// Consume the query and return all matching cohere clusters.
    pub fn collect(self) -> Vec<&'w Cohere> {
        self.candidates
    }

    /// Return the `CohereId`s of all matching clusters.
    pub fn ids(self) -> Vec<CohereId> {
        self.candidates.iter().map(|c| c.id).collect()
    }

    /// Number of matching cohere clusters.
    pub fn count(self) -> usize {
        self.candidates.len()
    }

    /// The cohere cluster with the highest strength, or `None` if empty.
    pub fn strongest(self) -> Option<&'w Cohere> {
        cohere_filters::strongest_cohere(self.candidates)
    }

    /// Mean strength across matching cohere clusters, or `None` if empty.
    pub fn mean_strength(self) -> Option<f32> {
        if self.candidates.is_empty() {
            return None;
        }
        let sum: f32 = self.candidates.iter().map(|c| c.strength).sum();
        Some(sum / self.candidates.len() as f32)
    }
}
