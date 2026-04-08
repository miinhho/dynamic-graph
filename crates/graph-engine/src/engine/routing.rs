use graph_core::{Channel, ChannelId, ChannelMode, EntityId, EntityKindId};
use graph_world::{EntitySelector, ResolvedSelection, WorldSnapshot};
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Copy)]
pub struct RoutingPolicy {
    pub field_threshold: usize,
    pub cohort_threshold: usize,
}

pub trait RoutingStrategy {
    fn route(
        &self,
        channel: &Channel,
        resolved: &ResolvedSelection,
        snapshot: WorldSnapshot<'_>,
    ) -> ChannelMode;
}

#[derive(Default)]
pub(crate) struct SelectorCache {
    entries: FxHashMap<ChannelId, ResolvedSelection>,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            field_threshold: usize::MAX,
            cohort_threshold: usize::MAX,
        }
    }
}

impl RoutingStrategy for RoutingPolicy {
    fn route(
        &self,
        channel: &Channel,
        resolved: &ResolvedSelection,
        _: WorldSnapshot<'_>,
    ) -> ChannelMode {
        route_channel_mode(
            channel,
            *self,
            resolved.targets.len(),
            resolved.cohort_kinds.len(),
        )
    }
}

pub enum DispatchPlan<'a> {
    Direct {
        mode: ChannelMode,
        targets: &'a [EntityId],
        available: usize,
    },
    Cohort {
        kinds: &'a [EntityKindId],
        available: usize,
    },
}

impl DispatchPlan<'_> {
    pub fn mode(&self) -> ChannelMode {
        match self {
            Self::Direct { mode, .. } => *mode,
            Self::Cohort { .. } => ChannelMode::Cohort,
        }
    }
}

pub fn plan_channel_dispatch<'a>(
    world: WorldSnapshot<'_>,
    channel: &Channel,
    strategy: &impl RoutingStrategy,
    remaining_targets: usize,
    selector_cache: &'a mut SelectorCache,
) -> DispatchPlan<'a> {
    let resolved = selector_cache.resolve(world, channel);
    let mode = strategy.route(channel, resolved, world);

    match mode {
        ChannelMode::Cohort => {
            let available = resolved.cohort_kinds.len();
            let count = available.min(remaining_targets);
            let kinds = &resolved.cohort_kinds[..count];
            DispatchPlan::Cohort { kinds, available }
        }
        ChannelMode::Pairwise | ChannelMode::Broadcast | ChannelMode::Field => {
            let available = resolved.targets.len();
            let count = available.min(remaining_targets);
            let targets = &resolved.targets[..count];
            DispatchPlan::Direct {
                mode,
                targets,
                available,
            }
        }
    }
}

fn route_channel_mode(
    channel: &Channel,
    policy: RoutingPolicy,
    field_count: usize,
    cohort_count: usize,
) -> ChannelMode {
    match channel.kind {
        ChannelMode::Pairwise | ChannelMode::Field | ChannelMode::Cohort => channel.kind,
        ChannelMode::Broadcast => {
            if cohort_count >= policy.cohort_threshold && cohort_count > 0 {
                return ChannelMode::Cohort;
            }
            if channel.field_radius.is_some() && field_count >= policy.field_threshold {
                return ChannelMode::Field;
            }
            ChannelMode::Broadcast
        }
    }
}

impl SelectorCache {
    fn resolve(&mut self, world: WorldSnapshot<'_>, channel: &Channel) -> &ResolvedSelection {
        self.entries
            .entry(channel.id)
            .or_insert_with(|| world.resolve_selector(&EntitySelector::from_channel(channel)))
    }
}
