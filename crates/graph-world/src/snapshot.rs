use graph_core::{Channel, ChannelId, Entity, EntityId, EntityKindId, WorldVersion};

use crate::{EntitySelector, ResolvedSelection, World};

#[derive(Clone, Copy)]
pub struct WorldSnapshot<'a> {
    world: &'a World,
}

impl<'a> WorldSnapshot<'a> {
    pub fn new(world: &'a World) -> Self {
        Self { world }
    }

    pub fn version(&self) -> WorldVersion {
        self.world.version()
    }

    pub fn entity(&self, id: EntityId) -> Option<&'a Entity> {
        self.world.entity(id)
    }

    pub fn entity_version(&self, id: EntityId) -> Option<WorldVersion> {
        self.world.entity_version(id)
    }

    pub fn entities(&self) -> impl Iterator<Item = &'a Entity> {
        self.world.entities()
    }

    pub fn channel(&self, id: ChannelId) -> Option<&'a Channel> {
        self.world.channel(id)
    }

    pub fn channels(&self) -> impl Iterator<Item = &'a Channel> {
        self.world.channels()
    }

    pub fn outbound_channels(&self, source: EntityId) -> impl Iterator<Item = &'a Channel> {
        self.world.outbound_channels(source)
    }

    pub fn explicit_inbound_channels(&self, target: EntityId) -> impl Iterator<Item = &'a Channel> {
        self.world.explicit_inbound_channels(target)
    }

    pub fn inbound_channels_for_kind(
        &self,
        kind: EntityKindId,
    ) -> impl Iterator<Item = &'a Channel> {
        self.world.inbound_channels_for_kind(kind)
    }

    pub fn inbound_field_channels(
        &self,
        target_position: &graph_core::StateVector,
    ) -> impl Iterator<Item = &'a Channel> {
        self.world.inbound_field_channels(target_position)
    }

    pub fn entity_ids_of_kind(&self, kind: EntityKindId) -> &'a [EntityId] {
        self.world.entity_ids_of_kind(kind)
    }

    pub fn resolve_selector(&self, selector: &EntitySelector) -> ResolvedSelection {
        self.world.resolve_selector(selector)
    }

    pub fn distance_between(&self, source: EntityId, target: EntityId) -> Option<f32> {
        self.world.distance_between(source, target)
    }
}
