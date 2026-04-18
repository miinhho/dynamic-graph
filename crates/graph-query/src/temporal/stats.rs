use graph_core::{BatchId, ChangeSubject, LocusId, RelationshipId};
use graph_world::World;
use rustc_hash::FxHashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct BatchStats {
    pub batch: BatchId,
    pub total_changes: usize,
    pub loci_changed: usize,
    pub relationships_changed: usize,
    pub mean_delta: f32,
}

pub fn batch_stats(world: &World, batch: BatchId) -> Option<BatchStats> {
    let changes: Vec<_> = world.log().batch(batch).collect();
    if changes.is_empty() {
        return None;
    }

    let mut loci: FxHashSet<LocusId> = FxHashSet::default();
    let mut rels: FxHashSet<RelationshipId> = FxHashSet::default();
    let mut total_delta = 0.0f32;
    let mut delta_count = 0usize;

    for c in &changes {
        match c.subject {
            ChangeSubject::Locus(id) => {
                loci.insert(id);
            }
            ChangeSubject::Relationship(id) => {
                rels.insert(id);
            }
        }
        let before = c.before.as_slice();
        let after = c.after.as_slice();
        let len = before.len().max(after.len());
        for i in 0..len {
            let b = before.get(i).copied().unwrap_or(0.0);
            let a = after.get(i).copied().unwrap_or(0.0);
            total_delta += (a - b).abs();
            delta_count += 1;
        }
    }

    Some(BatchStats {
        batch,
        total_changes: changes.len(),
        loci_changed: loci.len(),
        relationships_changed: rels.len(),
        mean_delta: if delta_count > 0 {
            total_delta / delta_count as f32
        } else {
            0.0
        },
    })
}
