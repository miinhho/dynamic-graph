//! Plain numeric state container and slot metadata.
//!
//! `StateVector` is the dumbest-possible bag of `f32` slots used uniformly
//! across the substrate (locus state, change deltas, relationship metrics,
//! entity coherence summaries). It is intentionally untyped — the
//! `LocusKindId` / `InfluenceKindId` carried alongside it tells the engine
//! and the user how to interpret each slot.
//!
//! `StateSlotDef` carries human-readable metadata (name, description, range)
//! for each slot in a locus kind's `StateVector`, enabling named-slot queries
//! and diagnostic output without encoding indices in caller code.
//!
//! Performance is not a concern at this layer of the redesign; correctness
//! and clarity are. A SmallVec/aligned-buffer variant can land later.

/// Metadata describing one slot in a locus's `StateVector`.
///
/// `StateSlotDef`s are attached to a `LocusKindConfig` so the engine and
/// query surface can report slot names and expected ranges without the caller
/// needing to hard-code numeric indices. They are purely advisory — the
/// engine never enforces clamping based on `range`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StateSlotDef {
    /// Short label used in diagnostics and query filters (e.g. `"belief"`).
    pub name: String,
    /// Optional human-readable description of what this slot represents.
    pub description: Option<String>,
    /// Optional `(min, max)` expected range. Advisory only — not enforced.
    pub range: Option<(f32, f32)>,
}

impl StateSlotDef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), description: None, range: None }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn with_range(mut self, min: f32, max: f32) -> Self {
        self.range = Some((min, max));
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

    /// Returns a copy of this vector with `slots[idx]` incremented by `delta`.
    ///
    /// Like `with_slot`, the vector is extended with zeros if `idx` is beyond
    /// the current length.
    ///
    /// ```rust,ignore
    /// // bump reliability by 0.1 without touching other slots
    /// let updated = rel.state.clone().with_slot_delta(RELIABILITY_SLOT, 0.1);
    /// ```
    pub fn with_slot_delta(mut self, idx: usize, delta: f32) -> Self {
        if idx >= self.slots.len() {
            self.slots.resize(idx + 1, 0.0);
        }
        self.slots[idx] += delta;
        self
    }

    /// Returns a copy of this vector with `slots[idx]` set to `val`.
    ///
    /// If `idx` is beyond the current length, the vector is extended
    /// with zeros up to (and including) `idx`. This makes partial slot
    /// updates concise without requiring the caller to reconstruct the
    /// full slice:
    ///
    /// ```rust,ignore
    /// let updated = rel.state.with_slot(HOSTILITY_SLOT, new_hostility);
    /// ```
    pub fn with_slot(mut self, idx: usize, val: f32) -> Self {
        if idx >= self.slots.len() {
            self.slots.resize(idx + 1, 0.0);
        }
        self.slots[idx] = val;
        self
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

    #[test]
    fn with_slot_delta_increments_existing_slot() {
        let v = StateVector::from_slice(&[1.0, 2.0, 3.0]);
        let v2 = v.with_slot_delta(1, 0.5);
        assert_eq!(v2.as_slice(), &[1.0, 2.5, 3.0]);
    }

    #[test]
    fn with_slot_delta_extends_when_out_of_bounds() {
        let v = StateVector::from_slice(&[1.0]);
        let v2 = v.with_slot_delta(2, 0.7);
        assert_eq!(v2.as_slice(), &[1.0, 0.0, 0.7]);
    }

    #[test]
    fn with_slot_delta_negative_decrements() {
        let v = StateVector::from_slice(&[5.0, 3.0]);
        let v2 = v.with_slot_delta(0, -2.0);
        assert_eq!(v2.as_slice(), &[3.0, 3.0]);
    }
}
