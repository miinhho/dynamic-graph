use graph_core::LocusId;

use super::{PartitionFn, PartitionIndex, World};

impl World {
    pub fn set_partition_fn(&mut self, f: Option<PartitionFn>) {
        self.partition_index = f.map(|partition_fn| {
            let mut index = PartitionIndex::new(partition_fn);
            for locus in self.loci.iter() {
                index.assign(locus);
            }
            index
        });
    }

    pub fn repartition(&mut self) {
        if let Some(index) = self.partition_index.take() {
            let partition_fn = index.fn_.clone();
            self.partition_index = Some(self.rebuild_partition_index(partition_fn));
        }
    }

    pub fn partition_index(&self) -> Option<&PartitionIndex> {
        self.partition_index.as_ref()
    }

    pub fn partition_of(&self, locus_id: LocusId) -> Option<u64> {
        self.partition_index.as_ref()?.bucket_of(locus_id)
    }

    pub(super) fn assign_partition_for_locus(&mut self, locus: &graph_core::Locus) {
        if let Some(index) = &mut self.partition_index {
            index.assign(locus);
        }
    }

    fn rebuild_partition_index(&self, partition_fn: PartitionFn) -> PartitionIndex {
        let mut index = PartitionIndex::new(partition_fn);
        for locus in self.loci.iter() {
            index.assign(locus);
        }
        index
    }
}
