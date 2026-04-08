use graph_core::{Stimulus, TickId};
use graph_tx::RecordedDelta;
use graph_world::{
    ChannelQuery, EntityProjection, EntityQuery, SnapshotQuery, World, WorldSnapshot,
};

use crate::{
    CommitPolicy, Engine, LawCatalog, ProgramCatalog, RuntimeCoordinator, RuntimeTick, Stabilizer,
    TickDriver, TickInspection, TickResult,
};

pub trait ClientDriver<P, L> {
    type Output;

    fn execute(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &P,
        laws: &L,
        stimuli: &[Stimulus],
    ) -> Self::Output;
}

pub trait InspectableResult {
    fn inspect<'a>(&'a self, snapshot: WorldSnapshot<'a>) -> TickInspection<'a>;
}

pub struct RuntimeClient<'a, D, P, L> {
    driver: &'a D,
    programs: &'a P,
    laws: &'a L,
}

#[derive(Clone, Copy)]
pub struct WorldRead<'a> {
    query: SnapshotQuery<'a>,
}

#[derive(Clone, Copy)]
pub struct TickRead<'a> {
    inspection: TickInspection<'a>,
}

impl<S, P, L> ClientDriver<P, L> for Engine<S>
where
    S: Stabilizer,
    P: ProgramCatalog,
    L: LawCatalog,
{
    type Output = TickResult;

    fn execute(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &P,
        laws: &L,
        stimuli: &[Stimulus],
    ) -> Self::Output {
        self.tick(tick, world, programs, laws, stimuli)
    }
}

impl<D, C, P, L> ClientDriver<P, L> for RuntimeCoordinator<D, C>
where
    D: TickDriver,
    C: CommitPolicy,
    P: ProgramCatalog,
    L: LawCatalog,
{
    type Output = RuntimeTick;

    fn execute(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &P,
        laws: &L,
        stimuli: &[Stimulus],
    ) -> Self::Output {
        self.tick(tick, world, programs, laws, stimuli)
    }
}

impl InspectableResult for TickResult {
    fn inspect<'a>(&'a self, snapshot: WorldSnapshot<'a>) -> TickInspection<'a> {
        TickResult::inspect(self, snapshot)
    }
}

impl InspectableResult for RuntimeTick {
    fn inspect<'a>(&'a self, snapshot: WorldSnapshot<'a>) -> TickInspection<'a> {
        RuntimeTick::inspect(self, snapshot)
    }
}

impl<'a, D, P, L> RuntimeClient<'a, D, P, L> {
    pub fn new(driver: &'a D, programs: &'a P, laws: &'a L) -> Self {
        Self {
            driver,
            programs,
            laws,
        }
    }

    pub fn driver(&self) -> &'a D {
        self.driver
    }

    pub fn programs(&self) -> &'a P {
        self.programs
    }

    pub fn laws(&self) -> &'a L {
        self.laws
    }

    pub fn read<'w>(&self, world: &'w World) -> WorldRead<'w> {
        WorldRead::new(world.snapshot())
    }

    pub fn inspect<'w, R>(&self, world: &'w World, result: &'w R) -> TickRead<'w>
    where
        R: InspectableResult,
    {
        TickRead::new(result.inspect(world.snapshot()))
    }
}

impl<'a, D, P, L> RuntimeClient<'a, D, P, L>
where
    D: ClientDriver<P, L>,
{
    pub fn tick(&self, tick: TickId, world: &mut World, stimuli: &[Stimulus]) -> D::Output {
        self.driver
            .execute(tick, world, self.programs, self.laws, stimuli)
    }
}

impl<'a> WorldRead<'a> {
    pub fn new(snapshot: WorldSnapshot<'a>) -> Self {
        Self {
            query: SnapshotQuery::new(snapshot),
        }
    }

    pub fn snapshot(&self) -> WorldSnapshot<'a> {
        self.query.snapshot()
    }

    pub fn query(&self) -> SnapshotQuery<'a> {
        self.query
    }

    pub fn entities(&self) -> EntityQuery<'a> {
        self.query.entities()
    }

    pub fn channels(&self) -> ChannelQuery<'a> {
        self.query.channels()
    }

    pub fn entity_projection(
        &self,
        entity_id: graph_core::EntityId,
    ) -> Option<EntityProjection<'a>> {
        self.query.entity_projection(entity_id)
    }
}

impl<'a> TickRead<'a> {
    pub fn new(inspection: TickInspection<'a>) -> Self {
        Self { inspection }
    }

    pub fn inspection(&self) -> TickInspection<'a> {
        self.inspection
    }

    pub fn entities(&self) -> EntityQuery<'a> {
        self.inspection.snapshot_query().entities()
    }

    pub fn channels(&self) -> ChannelQuery<'a> {
        self.inspection.snapshot_query().channels()
    }

    pub fn deltas(&self) -> &'a [RecordedDelta] {
        self.inspection.deltas()
    }

    pub fn changed_entity_projections(&self) -> Vec<EntityProjection<'a>> {
        self.inspection.changed_entity_projections()
    }
}
