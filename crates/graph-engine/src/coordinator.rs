use graph_core::{Stimulus, TickId};
use graph_world::World;

use crate::{Engine, LawCatalog, ProgramCatalog, Stabilizer, TickResult};

pub trait TickDriver {
    fn tick<P, L>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &P,
        laws: &L,
        stimuli: &[Stimulus],
    ) -> TickResult
    where
        P: ProgramCatalog,
        L: LawCatalog;
}

impl<S> TickDriver for Engine<S>
where
    S: Stabilizer,
{
    fn tick<P, L>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &P,
        laws: &L,
        stimuli: &[Stimulus],
    ) -> TickResult
    where
        P: ProgramCatalog,
        L: LawCatalog,
    {
        Engine::tick(self, tick, world, programs, laws, stimuli)
    }
}

pub trait StimulusAdapter<I> {
    fn adapt(&self, input: I) -> Vec<Stimulus>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityStimulusAdapter;

impl StimulusAdapter<&[Stimulus]> for IdentityStimulusAdapter {
    fn adapt(&self, input: &[Stimulus]) -> Vec<Stimulus> {
        input.to_vec()
    }
}

pub trait CommitPolicy {
    fn should_retry(&self, attempt: usize, result: &TickResult) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: usize,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 3 }
    }
}

impl CommitPolicy for RetryPolicy {
    fn should_retry(&self, attempt: usize, result: &TickResult) -> bool {
        attempt < self.max_attempts.max(1) && result.transaction.conflict.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeTick {
    pub attempts: usize,
    pub result: TickResult,
}

pub struct RuntimeCoordinator<D, P = RetryPolicy> {
    driver: D,
    commit_policy: P,
}

impl<D> RuntimeCoordinator<D, RetryPolicy> {
    pub fn new(driver: D, retry_policy: RetryPolicy) -> Self {
        Self {
            driver,
            commit_policy: retry_policy,
        }
    }
}

impl<D, P> RuntimeCoordinator<D, P> {
    pub fn with_policy(driver: D, commit_policy: P) -> Self {
        Self {
            driver,
            commit_policy,
        }
    }

    pub fn driver(&self) -> &D {
        &self.driver
    }

    pub fn into_driver(self) -> D {
        self.driver
    }
}

impl<D, P> RuntimeCoordinator<D, P>
where
    D: TickDriver,
    P: CommitPolicy,
{
    pub fn tick(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &impl ProgramCatalog,
        laws: &impl LawCatalog,
        stimuli: &[Stimulus],
    ) -> RuntimeTick {
        let mut last_result = self.driver.tick(tick, world, programs, laws, stimuli);
        let mut attempts = 1;

        while self.commit_policy.should_retry(attempts, &last_result) {
            attempts += 1;
            last_result = self.driver.tick(tick, world, programs, laws, stimuli);
        }

        RuntimeTick {
            attempts,
            result: last_result,
        }
    }

    pub fn tick_with_adapter<I, A>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &impl ProgramCatalog,
        laws: &impl LawCatalog,
        input: I,
        adapter: &A,
    ) -> RuntimeTick
    where
        A: StimulusAdapter<I>,
    {
        let stimuli = adapter.adapt(input);
        self.tick(tick, world, programs, laws, &stimuli)
    }
}
