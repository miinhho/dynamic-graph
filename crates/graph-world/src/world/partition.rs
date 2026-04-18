use std::sync::Arc;
use rustc_hash::FxHashMap;
use graph_core::{Locus, LocusId};

/// Maps each locus to a partition bucket (u64). The value is arbitrary —
/// the engine groups by equality, not range. Two loci with the same return
/// value are co-located.
///
/// Called once per locus on `repartition()` and incrementally on new locus
/// creation. Must be `Send + Sync` because partition shards are processed
/// on rayon worker threads.
pub type PartitionFn = Arc<dyn Fn(&Locus) -> u64 + Send + Sync>;

#[derive(Clone)]
pub struct PartitionIndex {
    pub(crate) fn_: PartitionFn,
    /// locus_id → partition bucket
    pub(crate) assignment: FxHashMap<LocusId, u64>,
    /// partition bucket → Vec<LocusId>
    pub(crate) members: FxHashMap<u64, Vec<LocusId>>,
}

impl PartitionIndex {
    pub fn new(fn_: PartitionFn) -> Self {
        Self {
            fn_,
            assignment: FxHashMap::default(),
            members: FxHashMap::default(),
        }
    }

    /// Assign `locus` to its partition bucket.
    pub fn assign(&mut self, locus: &Locus) {
        let bucket = (self.fn_)(locus);
        let id = locus.id;
        if let Some(&old) = self.assignment.get(&id) {
            if old == bucket {
                return;
            }
            if let Some(v) = self.members.get_mut(&old) {
                v.retain(|&x| x != id);
            }
        }
        self.assignment.insert(id, bucket);
        self.members.entry(bucket).or_default().push(id);
    }

    /// Remove `locus_id` from the index (called on locus deletion).
    pub fn remove(&mut self, id: LocusId) {
        if let Some(bucket) = self.assignment.remove(&id) {
            if let Some(v) = self.members.get_mut(&bucket) {
                v.retain(|&x| x != id);
            }
        }
    }

    /// Return the partition bucket for `locus_id`, or `None` if not assigned.
    pub fn bucket_of(&self, id: LocusId) -> Option<u64> {
        self.assignment.get(&id).copied()
    }

    /// Sorted list of distinct partition bucket IDs.
    pub fn buckets(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.members.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Loci in bucket `b`.
    pub fn members_of(&self, b: u64) -> &[LocusId] {
        self.members.get(&b).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Number of distinct partition buckets.
    pub fn bucket_count(&self) -> usize {
        self.members.len()
    }

    /// Read-only view of the locus → bucket assignment map.
    /// Used by the engine's parallel Apply phase to extract per-partition
    /// relationship shards without cloning via the `PartitionIndex` borrow.
    pub fn assignment(&self) -> &rustc_hash::FxHashMap<LocusId, u64> {
        &self.assignment
    }
}

impl std::fmt::Debug for PartitionIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartitionIndex")
            .field("buckets", &self.members.len())
            .field("loci_assigned", &self.assignment.len())
            .finish()
    }
}
