use std::collections::BTreeMap;

use crate::index::{IndexQuery, SelectorIndex, WorldIndex};
use crate::selector::{EntitySelector, ResolvedSelection, SelectorMode};
use crate::snapshot::WorldSnapshot;
use graph_core::{
    Channel, ChannelId, Entity, EntityId, EntityKindId, EntityState, StateVector, WorldVersion,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitConflict {
    pub expected: WorldVersion,
    pub actual: WorldVersion,
}

const FIELD_BUCKET_CELL_SIZE: f32 = 1.0;

#[derive(Debug, Clone, Default)]
pub struct World {
    version: WorldVersion,
    entities: BTreeMap<EntityId, Entity>,
    entity_versions: BTreeMap<EntityId, WorldVersion>,
    channels: BTreeMap<ChannelId, Channel>,
    outbound: BTreeMap<EntityId, Vec<ChannelId>>,
    inbound_explicit: BTreeMap<EntityId, Vec<ChannelId>>,
    inbound_kind: BTreeMap<EntityKindId, Vec<ChannelId>>,
    inbound_field_spatial: BTreeMap<i64, Vec<ChannelId>>,
    inbound_field_global: Vec<ChannelId>,
    index: WorldIndex,
}

impl World {
    pub fn insert_entity(&mut self, entity: Entity) {
        let id = entity.id;
        let previous = self.entities.insert(id, entity);
        self.bump_version();
        if let Some(previous) = previous.as_ref() {
            self.index.remove_entity(previous);
        }
        if let Some(current) = self.entities.get(&id) {
            self.index.insert_entity(current);
            self.entity_versions.insert(id, self.version);
        }
    }

    pub fn insert_channel(&mut self, channel: Channel) {
        self.bump_version();
        self.outbound
            .entry(channel.source)
            .or_default()
            .push(channel.id);
        for target in &channel.targets {
            self.inbound_explicit
                .entry(*target)
                .or_default()
                .push(channel.id);
        }
        for kind in &channel.target_kinds {
            self.inbound_kind.entry(*kind).or_default().push(channel.id);
        }
        if channel.targets.is_empty() {
            index_dynamic_inbound_channel(
                &mut self.inbound_field_spatial,
                &mut self.inbound_field_global,
                self.entities
                    .get(&channel.source)
                    .map(|entity| &entity.position),
                &channel,
            );
        }
        self.channels.insert(channel.id, channel);
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn version(&self) -> WorldVersion {
        self.version
    }

    pub fn snapshot(&self) -> WorldSnapshot<'_> {
        WorldSnapshot::new(self)
    }

    pub fn entity_version(&self, id: EntityId) -> Option<WorldVersion> {
        self.entity_versions.get(&id).copied()
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.get(&id)
    }

    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    pub fn outbound_channels(&self, source: EntityId) -> impl Iterator<Item = &Channel> + '_ {
        self.outbound
            .get(&source)
            .into_iter()
            .flatten()
            .filter_map(|channel_id| self.channels.get(channel_id))
    }

    pub fn explicit_inbound_channels(
        &self,
        target: EntityId,
    ) -> impl Iterator<Item = &Channel> + '_ {
        self.inbound_explicit
            .get(&target)
            .into_iter()
            .flatten()
            .filter_map(|channel_id| self.channels.get(channel_id))
    }

    pub fn inbound_channels_for_kind(
        &self,
        kind: EntityKindId,
    ) -> impl Iterator<Item = &Channel> + '_ {
        self.inbound_kind
            .get(&kind)
            .into_iter()
            .flatten()
            .filter_map(|channel_id| self.channels.get(channel_id))
    }

    pub fn inbound_field_channels(
        &self,
        target_position: &StateVector,
    ) -> impl Iterator<Item = &Channel> + '_ {
        let mut ids = Vec::new();
        if let Some(bucket) = spatial_bucket(target_position, FIELD_BUCKET_CELL_SIZE)
            && let Some(channel_ids) = self.inbound_field_spatial.get(&bucket)
        {
            ids.extend(channel_ids.iter().copied());
        }
        ids.extend(self.inbound_field_global.iter().copied());
        ids.sort_unstable_by_key(|id| id.0);
        ids.dedup_by_key(|id| id.0);

        ids.into_iter()
            .filter_map(|channel_id| self.channels.get(&channel_id))
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn entity_ids_of_kind(&self, kind: EntityKindId) -> &[EntityId] {
        self.index.entities_of_kind(kind)
    }

    pub fn resolve_selector(&self, selector: &EntitySelector) -> ResolvedSelection {
        let source_position = self
            .entities
            .get(&selector.source)
            .map(|entity| entity.position.clone())
            .unwrap_or_default();
        if !selector.targets.is_empty() {
            let mut matched = selector
                .targets
                .iter()
                .filter_map(|target_id| self.entities.get(target_id))
                .filter(|entity| entity.id != selector.source)
                .filter(|entity| {
                    selector.target_kinds.is_empty() || selector.target_kinds.contains(&entity.kind)
                })
                .filter(|entity| match selector.radius {
                    Some(max_radius) => source_position.distance(&entity.position) <= max_radius,
                    None => true,
                })
                .map(|entity| entity.id)
                .collect::<Vec<_>>();
            matched.sort_unstable_by_key(|id| id.0);
            matched.dedup_by_key(|id| id.0);
            return ResolvedSelection {
                cohort_kinds: collect_cohort_kinds(self, &matched),
                targets: matched,
            };
        }

        let candidates = match selector.mode {
            SelectorMode::ExplicitOnly => Vec::new(),
            SelectorMode::IndexedOnly => {
                match (selector.radius, selector.target_kinds.is_empty()) {
                    (Some(_), _) if source_position.is_empty() => Vec::new(),
                    (Some(_), _) | (None, false) => self.index.resolve_candidates(IndexQuery {
                        source_position: (!source_position.is_empty()).then_some(&source_position),
                        target_kinds: &selector.target_kinds,
                        radius: selector.radius,
                    }),
                    (None, true) => Vec::new(),
                }
            }
            SelectorMode::AllowFullScan => {
                match (selector.radius, selector.target_kinds.is_empty()) {
                    (Some(_), _) if source_position.is_empty() => {
                        self.entities.keys().copied().collect()
                    }
                    (Some(_), _) | (None, false) => self.index.resolve_candidates(IndexQuery {
                        source_position: (!source_position.is_empty()).then_some(&source_position),
                        target_kinds: &selector.target_kinds,
                        radius: selector.radius,
                    }),
                    (None, true) => self.entities.keys().copied().collect(),
                }
            }
        };

        let mut matched = candidates
            .into_iter()
            .filter(|entity_id| *entity_id != selector.source)
            .filter_map(|entity_id| self.entities.get(&entity_id))
            .filter(|entity| {
                selector.target_kinds.is_empty() || selector.target_kinds.contains(&entity.kind)
            })
            .filter(|entity| match selector.radius {
                Some(max_radius) => source_position.distance(&entity.position) <= max_radius,
                None => true,
            })
            .map(|entity| entity.id)
            .collect::<Vec<_>>();
        matched.sort_unstable_by_key(|id| id.0);
        matched.dedup_by_key(|id| id.0);
        ResolvedSelection {
            cohort_kinds: collect_cohort_kinds(self, &matched),
            targets: matched,
        }
    }

    pub fn distance_between(&self, source: EntityId, target: EntityId) -> Option<f32> {
        let source_position = self.entities.get(&source)?.position.clone();
        let target_position = self.entities.get(&target)?.position.clone();
        Some(source_position.distance(&target_position))
    }

    pub fn commit_entity_states(
        &mut self,
        expected_version: WorldVersion,
        updates: &[(EntityId, EntityState)],
    ) -> Result<WorldVersion, CommitConflict> {
        if self.version != expected_version {
            return Err(CommitConflict {
                expected: expected_version,
                actual: self.version,
            });
        }

        if updates.is_empty() {
            return Ok(self.version);
        }

        self.bump_version();
        for (entity_id, state) in updates {
            if let Some(entity) = self.entities.get_mut(entity_id) {
                entity.state = state.clone();
                self.entity_versions.insert(*entity_id, self.version);
            }
        }
        Ok(self.version)
    }

    fn bump_version(&mut self) {
        self.version = WorldVersion(self.version.0.saturating_add(1));
    }
}

fn collect_cohort_kinds(world: &World, targets: &[EntityId]) -> Vec<EntityKindId> {
    let mut cohort_kinds = targets
        .iter()
        .filter_map(|entity_id| world.entity(*entity_id).map(|entity| entity.kind))
        .collect::<Vec<_>>();
    cohort_kinds.sort_unstable_by_key(|kind| kind.0);
    cohort_kinds.dedup_by_key(|kind| kind.0);
    cohort_kinds
}

fn index_dynamic_inbound_channel(
    inbound_field_spatial: &mut BTreeMap<i64, Vec<ChannelId>>,
    inbound_field_global: &mut Vec<ChannelId>,
    source_position: Option<&StateVector>,
    channel: &Channel,
) {
    match (
        channel.field_radius,
        source_position.and_then(|position| spatial_bucket(position, FIELD_BUCKET_CELL_SIZE)),
    ) {
        (Some(radius), Some(bucket)) => {
            let bucket_radius = (radius / FIELD_BUCKET_CELL_SIZE).ceil() as i64;
            for target_bucket in (bucket - bucket_radius)..=(bucket + bucket_radius) {
                inbound_field_spatial
                    .entry(target_bucket)
                    .or_default()
                    .push(channel.id);
            }
        }
        _ => inbound_field_global.push(channel.id),
    }
}

fn spatial_bucket(position: &StateVector, cell_size: f32) -> Option<i64> {
    position.first().map(|value| bucket_for(value, cell_size))
}

fn bucket_for(value: f32, cell_size: f32) -> i64 {
    (value / cell_size).floor() as i64
}
