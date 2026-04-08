use std::collections::BTreeMap;

use graph_core::{Entity, EntityId, EntityKindId, StateVector};
pub trait SelectorIndex {
    fn insert_entity(&mut self, entity: &Entity);
    fn remove_entity(&mut self, entity: &Entity);
    fn entities_of_kind(&self, kind: EntityKindId) -> &[EntityId];
    fn resolve_candidates(&self, query: IndexQuery<'_>) -> Vec<EntityId>;
}

#[derive(Clone, Copy)]
pub struct IndexQuery<'a> {
    pub source_position: Option<&'a StateVector>,
    pub target_kinds: &'a [EntityKindId],
    pub radius: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct WorldIndex {
    kind_entities: BTreeMap<EntityKindId, Vec<EntityId>>,
    spatial_buckets: BTreeMap<AxisBucket, Vec<EntityId>>,
    cell_size: f32,
}

impl Default for WorldIndex {
    fn default() -> Self {
        Self::new(1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AxisBucket(i64);

impl WorldIndex {
    pub fn new(cell_size: f32) -> Self {
        Self {
            kind_entities: BTreeMap::new(),
            spatial_buckets: BTreeMap::new(),
            cell_size: cell_size.max(f32::EPSILON),
        }
    }

    fn collect_kind_candidates(&self, target_kinds: &[EntityKindId]) -> Vec<EntityId> {
        let mut ids = Vec::new();
        for kind in target_kinds {
            if let Some(candidates) = self.kind_entities.get(kind) {
                ids.extend(candidates.iter().copied());
            }
        }
        ids.sort_unstable_by_key(|id| id.0);
        ids.dedup_by_key(|id| id.0);
        ids
    }

    fn collect_spatial_candidates(
        &self,
        source_position: &StateVector,
        radius: f32,
    ) -> Vec<EntityId> {
        let Some(source_axis) = source_position.first() else {
            return Vec::new();
        };
        let low = bucket_for(source_axis - radius, self.cell_size);
        let high = bucket_for(source_axis + radius, self.cell_size);
        let mut ids = Vec::new();

        for (_, bucket_ids) in self.spatial_buckets.range(low..=high) {
            ids.extend(bucket_ids.iter().copied());
        }

        ids.sort_unstable_by_key(|id| id.0);
        ids.dedup_by_key(|id| id.0);
        ids
    }
}

impl SelectorIndex for WorldIndex {
    fn insert_entity(&mut self, entity: &Entity) {
        self.kind_entities
            .entry(entity.kind)
            .or_default()
            .push(entity.id);
        if let Some(bucket) = spatial_bucket(&entity.position, self.cell_size) {
            self.spatial_buckets
                .entry(bucket)
                .or_default()
                .push(entity.id);
        }
    }

    fn remove_entity(&mut self, entity: &Entity) {
        if let Some(ids) = self.kind_entities.get_mut(&entity.kind) {
            ids.retain(|id| *id != entity.id);
            if ids.is_empty() {
                self.kind_entities.remove(&entity.kind);
            }
        }

        if let Some(bucket) = spatial_bucket(&entity.position, self.cell_size)
            && let Some(ids) = self.spatial_buckets.get_mut(&bucket)
        {
            ids.retain(|id| *id != entity.id);
            if ids.is_empty() {
                self.spatial_buckets.remove(&bucket);
            }
        }
    }

    fn entities_of_kind(&self, kind: EntityKindId) -> &[EntityId] {
        self.kind_entities
            .get(&kind)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn resolve_candidates(&self, query: IndexQuery<'_>) -> Vec<EntityId> {
        match (
            query.source_position,
            query.radius,
            query.target_kinds.is_empty(),
        ) {
            (Some(source_position), Some(radius), false) => {
                let kind_candidates = self.collect_kind_candidates(query.target_kinds);
                let spatial_candidates = self.collect_spatial_candidates(source_position, radius);
                intersect_candidates(kind_candidates, spatial_candidates)
            }
            (Some(source_position), Some(radius), true) => {
                self.collect_spatial_candidates(source_position, radius)
            }
            (_, None, false) => self.collect_kind_candidates(query.target_kinds),
            _ => Vec::new(),
        }
    }
}

fn intersect_candidates(mut left: Vec<EntityId>, mut right: Vec<EntityId>) -> Vec<EntityId> {
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }
    left.sort_unstable_by_key(|id| id.0);
    left.dedup_by_key(|id| id.0);
    right.sort_unstable_by_key(|id| id.0);
    right.dedup_by_key(|id| id.0);

    let mut left_index = 0;
    let mut right_index = 0;
    let mut intersection = Vec::with_capacity(left.len().min(right.len()));

    while left_index < left.len() && right_index < right.len() {
        match left[left_index].0.cmp(&right[right_index].0) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                intersection.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }

    intersection
}

fn spatial_bucket(position: &StateVector, cell_size: f32) -> Option<AxisBucket> {
    position.first().map(|value| bucket_for(value, cell_size))
}

fn bucket_for(value: f32, cell_size: f32) -> AxisBucket {
    AxisBucket((value / cell_size).floor() as i64)
}
