//! Plain numeric state container.
//!
//! `StateVector` is the dumbest-possible bag of `f32` slots used uniformly
//! across the substrate (locus state, change deltas, relationship metrics,
//! entity coherence summaries). It is intentionally untyped — the
//! `LocusKindId` / `InfluenceKindId` carried alongside it tells the engine
//! and the user how to interpret each slot.
//!
//! Performance is not a concern at this layer of the redesign; correctness
//! and clarity are. A SmallVec/aligned-buffer variant can land later.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateVector {
    slots: Vec<f32>,
}

impl StateVector {
    /// Empty vector — equivalent to "no state observed yet".
    pub fn empty() -> Self {
        Self { slots: Vec::new() }
    }

    /// All-zero vector of a given dimensionality.
    pub fn zeros(dim: usize) -> Self {
        Self {
            slots: vec![0.0; dim],
        }
    }

    pub fn from_slice(values: &[f32]) -> Self {
        Self {
            slots: values.to_vec(),
        }
    }

    pub fn dim(&self) -> usize {
        self.slots.len()
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.slots
    }

    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.slots
    }

    /// Element-wise sum, padding the shorter vector with zeros. Returns a
    /// fresh vector with `dim = max(self.dim, other.dim)`.
    pub fn add(&self, other: &StateVector) -> StateVector {
        let len = self.slots.len().max(other.slots.len());
        let mut out = vec![0.0; len];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.slots.get(i).copied().unwrap_or(0.0)
                + other.slots.get(i).copied().unwrap_or(0.0);
        }
        StateVector { slots: out }
    }

    /// L2 norm. Used by guard rails and regime classifiers.
    pub fn l2_norm(&self) -> f32 {
        self.slots.iter().map(|v| v * v).sum::<f32>().sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_pads_with_zero() {
        let a = StateVector::from_slice(&[1.0, 2.0]);
        let b = StateVector::from_slice(&[10.0, 20.0, 30.0]);
        let sum = a.add(&b);
        assert_eq!(sum.as_slice(), &[11.0, 22.0, 30.0]);
    }

    #[test]
    fn l2_norm_of_unit_axis_is_one() {
        let v = StateVector::from_slice(&[1.0, 0.0, 0.0]);
        assert!((v.l2_norm() - 1.0).abs() < 1e-6);
    }
}
