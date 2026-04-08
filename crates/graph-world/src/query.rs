use crate::{EntitySelector, ResolvedSelection, WorldSnapshot};
use graph_core::{Channel, ChannelId, Entity, EntityId, EntityKindId};

#[derive(Clone, Copy)]
pub struct EntityProjection<'a> {
    entity: &'a Entity,
}

#[derive(Clone, Copy)]
pub struct SnapshotQuery<'a> {
    snapshot: WorldSnapshot<'a>,
}

#[derive(Clone)]
pub struct EntityQuery<'a> {
    snapshot: WorldSnapshot<'a>,
    entity_ids: Vec<EntityId>,
}

#[derive(Clone)]
pub struct ChannelQuery<'a> {
    snapshot: WorldSnapshot<'a>,
    channel_ids: Vec<ChannelId>,
}

impl<'a> SnapshotQuery<'a> {
    pub fn new(snapshot: WorldSnapshot<'a>) -> Self {
        Self { snapshot }
    }

    pub fn snapshot(&self) -> WorldSnapshot<'a> {
        self.snapshot
    }

    pub fn entities(&self) -> EntityQuery<'a> {
        EntityQuery::new(
            self.snapshot,
            self.snapshot.entities().map(|entity| entity.id).collect(),
        )
    }

    pub fn channels(&self) -> ChannelQuery<'a> {
        ChannelQuery::new(
            self.snapshot,
            self.snapshot.channels().map(|channel| channel.id).collect(),
        )
    }

    pub fn entity(&self, id: EntityId) -> Option<&'a Entity> {
        self.snapshot.entity(id)
    }

    pub fn entity_projection(&self, id: EntityId) -> Option<EntityProjection<'a>> {
        self.entity(id).map(EntityProjection::new)
    }

    pub fn channels_from(&self, source: EntityId) -> impl Iterator<Item = &'a Channel> {
        self.snapshot.outbound_channels(source)
    }

    pub fn channels_to(&self, target: EntityId) -> Vec<&'a Channel> {
        self.channels().to(target).collect()
    }

    pub fn entities_by_kind(&self, kind: EntityKindId) -> Vec<&'a Entity> {
        self.entities().kind(kind).collect()
    }

    pub fn select(&self, selector: &EntitySelector) -> ResolvedSelection {
        self.snapshot.resolve_selector(selector)
    }

    pub fn selected_entities(&self, selector: &EntitySelector) -> Vec<&'a Entity> {
        self.entities().select(selector).collect()
    }

    pub fn selected_projections(&self, selector: &EntitySelector) -> Vec<EntityProjection<'a>> {
        self.entities().select(selector).projections()
    }

    pub fn count_entities(&self) -> usize {
        self.snapshot.entities().count()
    }

    pub fn count_channels(&self) -> usize {
        self.snapshot.channels().count()
    }
}

impl<'a> EntityQuery<'a> {
    pub fn new(snapshot: WorldSnapshot<'a>, entity_ids: Vec<EntityId>) -> Self {
        Self {
            snapshot,
            entity_ids,
        }
    }

    pub fn ids(&self) -> &[EntityId] {
        &self.entity_ids
    }

    pub fn count(&self) -> usize {
        self.entity_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entity_ids.is_empty()
    }

    pub fn kind(mut self, kind: EntityKindId) -> Self {
        self.entity_ids.retain(|entity_id| {
            self.snapshot
                .entity(*entity_id)
                .is_some_and(|entity| entity.kind == kind)
        });
        self
    }

    pub fn select(mut self, selector: &EntitySelector) -> Self {
        let resolved = self.snapshot.resolve_selector(selector);
        self.entity_ids = intersect_sorted_entity_ids(self.entity_ids, resolved.targets);
        self
    }

    pub fn collect(&self) -> Vec<&'a Entity> {
        self.entity_ids
            .iter()
            .filter_map(|entity_id| self.snapshot.entity(*entity_id))
            .collect()
    }

    pub fn first(&self) -> Option<&'a Entity> {
        self.entity_ids
            .first()
            .and_then(|entity_id| self.snapshot.entity(*entity_id))
    }

    pub fn projections(&self) -> Vec<EntityProjection<'a>> {
        self.collect()
            .into_iter()
            .map(EntityProjection::new)
            .collect()
    }
}

impl<'a> ChannelQuery<'a> {
    pub fn new(snapshot: WorldSnapshot<'a>, channel_ids: Vec<ChannelId>) -> Self {
        Self {
            snapshot,
            channel_ids,
        }
    }

    pub fn ids(&self) -> &[ChannelId] {
        &self.channel_ids
    }

    pub fn count(&self) -> usize {
        self.channel_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.channel_ids.is_empty()
    }

    pub fn from(mut self, source: EntityId) -> Self {
        self.channel_ids.retain(|channel_id| {
            self.snapshot
                .channel(*channel_id)
                .is_some_and(|channel| channel.source == source)
        });
        self
    }

    pub fn to(self, target: EntityId) -> Self {
        let Some(target_entity) = self.snapshot.entity(target) else {
            return Self::new(self.snapshot, Vec::new());
        };

        let mut matched = self
            .snapshot
            .explicit_inbound_channels(target)
            .map(|channel| channel.id)
            .collect::<Vec<_>>();
        matched.extend(
            self.snapshot
                .inbound_channels_for_kind(target_entity.kind)
                .filter(|channel| selector_matches_target(self.snapshot, channel, target))
                .map(|channel| channel.id),
        );
        matched.extend(
            self.snapshot
                .inbound_field_channels(&target_entity.position)
                .filter(|channel| selector_matches_target(self.snapshot, channel, target))
                .map(|channel| channel.id),
        );
        matched.sort_unstable_by_key(|channel_id| channel_id.0);
        matched.dedup_by_key(|channel_id| channel_id.0);

        Self::new(
            self.snapshot,
            intersect_sorted_channel_ids(self.channel_ids, matched),
        )
    }

    pub fn collect(&self) -> Vec<&'a Channel> {
        self.channel_ids
            .iter()
            .filter_map(|channel_id| self.snapshot.channel(*channel_id))
            .collect()
    }

    pub fn first(&self) -> Option<&'a Channel> {
        self.channel_ids
            .first()
            .and_then(|channel_id| self.snapshot.channel(*channel_id))
    }
}

fn selector_matches_target(
    snapshot: WorldSnapshot<'_>,
    channel: &Channel,
    target: EntityId,
) -> bool {
    snapshot
        .resolve_selector(&EntitySelector::from_channel(channel))
        .targets
        .contains(&target)
}

pub fn explicit_channel_ids<'a>(
    channels: impl Iterator<Item = &'a Channel> + 'a,
) -> impl Iterator<Item = ChannelId> + 'a {
    channels.map(|channel| channel.id)
}

fn intersect_sorted_entity_ids(mut left: Vec<EntityId>, mut right: Vec<EntityId>) -> Vec<EntityId> {
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

fn intersect_sorted_channel_ids(
    mut left: Vec<ChannelId>,
    mut right: Vec<ChannelId>,
) -> Vec<ChannelId> {
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

impl<'a> EntityProjection<'a> {
    pub fn new(entity: &'a Entity) -> Self {
        Self { entity }
    }

    pub fn entity(&self) -> &'a Entity {
        self.entity
    }

    pub fn id(&self) -> EntityId {
        self.entity.id
    }

    pub fn kind(&self) -> EntityKindId {
        self.entity.kind
    }

    pub fn position(&self) -> &graph_core::StateVector {
        &self.entity.position
    }

    pub fn internal(&self) -> &graph_core::StateVector {
        &self.entity.state.internal
    }

    pub fn emitted(&self) -> &graph_core::SignalVector {
        &self.entity.state.emitted
    }

    pub fn internal_norm(&self) -> f32 {
        self.entity.state.internal.l2_norm()
    }

    pub fn emitted_norm(&self) -> f32 {
        self.entity.state.emitted.l2_norm()
    }

    pub fn cooldown(&self) -> u32 {
        self.entity.state.cooldown
    }

    pub fn refractory_period(&self) -> u32 {
        self.entity.refractory_period
    }
}
