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
//! ## Inline storage
//!
//! `StateVector` backs its slots with a `SmallVec<[f32; 4]>`, keeping up to
//! 4 floats on the stack without a heap allocation.  Relationship state
//! (activity + weight = 2 slots) and small locus states fit entirely inline;
//! larger state vectors fall back to the heap transparently.

use smallvec::SmallVec;

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
    #[cfg_attr(
        feature = "serde",
        serde(
            serialize_with = "crate::state::serde_smallvec_f32::serialize",
            deserialize_with = "crate::state::serde_smallvec_f32::deserialize"
        )
    )]
    slots: SmallVec<[f32; 4]>,
}

impl StateVector {
    /// Empty vector — equivalent to "no state observed yet".
    pub fn empty() -> Self {
        Self { slots: SmallVec::new() }
    }

    /// All-zero vector of a given dimensionality.
    pub fn zeros(dim: usize) -> Self {
        Self {
            slots: smallvec::smallvec![0.0; dim],
        }
    }

    pub fn from_slice(values: &[f32]) -> Self {
        Self {
            slots: SmallVec::from_slice(values),
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
        let mut out: SmallVec<[f32; 4]> = SmallVec::with_capacity(len);
        for i in 0..len {
            out.push(
                self.slots.get(i).copied().unwrap_or(0.0)
                    + other.slots.get(i).copied().unwrap_or(0.0),
            );
        }
        StateVector { slots: out }
    }

    /// L2 norm. Used by guard rails and regime classifiers.
    pub fn l2_norm(&self) -> f32 {
        self.slots.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    /// Dot product with another `StateVector`.
    ///
    /// Shorter vectors are treated as zero-padded. Returns 0.0 for empty vectors.
    pub fn dot(&self, other: &StateVector) -> f32 {
        let len = self.slots.len().min(other.slots.len());
        self.slots[..len]
            .iter()
            .zip(&other.slots[..len])
            .map(|(a, b)| a * b)
            .sum()
    }

    /// Cosine similarity with another `StateVector` — the angular similarity
    /// between the two vectors, in `[-1.0, 1.0]`.
    ///
    /// Returns `0.0` when either vector is all-zero (undefined by convention).
    ///
    /// # Dimension mismatch
    ///
    /// **Only the shared prefix** (`length = min(self.dim, other.dim)`) is used
    /// when the two vectors have different dimensions.  Extra slots on the longer
    /// vector are silently ignored — they do **not** contribute to the result.
    pub fn cosine_similarity(&self, other: &StateVector) -> f32 {
        let norm_a = self.l2_norm();
        let norm_b = other.l2_norm();
        if norm_a < 1e-12 || norm_b < 1e-12 {
            return 0.0;
        }
        (self.dot(other) / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }

    /// Euclidean distance between two `StateVector`s.
    ///
    /// Shorter vectors are treated as zero-padded.
    pub fn euclidean_distance(&self, other: &StateVector) -> f32 {
        let len = self.slots.len().max(other.slots.len());
        (0..len)
            .map(|i| {
                let a = self.slots.get(i).copied().unwrap_or(0.0);
                let b = other.slots.get(i).copied().unwrap_or(0.0);
                (a - b) * (a - b)
            })
            .sum::<f32>()
            .sqrt()
    }
}

// ── Serde helpers for SmallVec<[f32; 4]> ─────────────────────────────────────
//
// SmallVec implements Serialize/Deserialize when its element type does, but
// the derive macros on StateVector need explicit paths when the serde feature
// is conditional.  We route through Vec<f32> for maximum compatibility.

#[cfg(feature = "serde")]
mod serde_smallvec_f32 {
    use smallvec::SmallVec;

    pub fn serialize<S>(v: &SmallVec<[f32; 4]>, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.collect_seq(v.iter())
    }

    pub fn deserialize<'de, D>(d: D) -> Result<SmallVec<[f32; 4]>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v: Vec<f32> = serde::Deserialize::deserialize(d)?;
        Ok(SmallVec::from_vec(v))
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

    #[test]
    fn dot_product_is_correct() {
        let a = StateVector::from_slice(&[1.0, 2.0, 3.0]);
        let b = StateVector::from_slice(&[4.0, 5.0, 6.0]);
        assert!((a.dot(&b) - 32.0).abs() < 1e-6); // 1*4 + 2*5 + 3*6 = 32
    }

    #[test]
    fn dot_truncates_to_shorter_vector() {
        let a = StateVector::from_slice(&[1.0, 2.0, 3.0]);
        let b = StateVector::from_slice(&[4.0, 5.0]);
        assert!((a.dot(&b) - 14.0).abs() < 1e-6); // 1*4 + 2*5 = 14 (slot 2 ignored)
    }

    #[test]
    fn cosine_similarity_identical_vectors_is_one() {
        let v = StateVector::from_slice(&[1.0, 2.0, 3.0]);
        assert!((v.cosine_similarity(&v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = StateVector::from_slice(&[1.0, 0.0]);
        let b = StateVector::from_slice(&[0.0, 1.0]);
        assert!(a.cosine_similarity(&b).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector_is_zero() {
        let a = StateVector::from_slice(&[1.0, 2.0]);
        let z = StateVector::zeros(2);
        assert_eq!(a.cosine_similarity(&z), 0.0);
    }

    #[test]
    fn euclidean_distance_same_vector_is_zero() {
        let v = StateVector::from_slice(&[1.0, 2.0, 3.0]);
        assert!(v.euclidean_distance(&v).abs() < 1e-6);
    }

    #[test]
    fn euclidean_distance_known_value() {
        let a = StateVector::from_slice(&[0.0, 0.0]);
        let b = StateVector::from_slice(&[3.0, 4.0]);
        assert!((a.euclidean_distance(&b) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn inline_storage_for_two_slots() {
        // Relationship state (activity + weight) must not heap-allocate.
        let v = StateVector::from_slice(&[1.0, 0.0]);
        assert!(!v.slots.spilled(), "2-slot StateVector should fit inline");
    }

    #[test]
    fn four_slots_still_inline() {
        let v = StateVector::from_slice(&[1.0, 2.0, 3.0, 4.0]);
        assert!(!v.slots.spilled(), "4-slot StateVector should fit inline");
    }

    #[test]
    fn five_slots_spills_to_heap() {
        let v = StateVector::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(v.slots.spilled(), "5-slot StateVector should spill to heap");
    }
}
