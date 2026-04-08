//! SCC-aware iterative tick driver.
//!
//! [`ScheduledDriver`] wraps a base [`Engine`] and turns one logical tick into
//! a short sequence of inner sub-ticks. The first sub-tick runs the full
//! engine pipeline as usual; subsequent sub-ticks re-run the engine on the
//! same world to let cyclic regions iterate toward equilibrium. Acyclic
//! regions only really need one pass — they will simply produce empty deltas
//! on the follow-up sub-ticks because their inputs no longer change.
//!
//! This is the opt-in companion to architecture §7.6: the engine's commit
//! boundary stays singular per *sub*-tick, but a logical tick can now contain
//! multiple committed sub-ticks. Users who want the original
//! single-commit-per-tick semantics keep using `Engine` directly.
//!
//! The driver short-circuits whenever
//!
//! - the latest sub-tick's `total_delta_norm` falls below
//!   `convergence_epsilon`, or
//! - the static SCC plan contains zero cyclic blocks (no cycles → nothing to
//!   iterate).

use graph_core::{Stimulus, TickId};
use graph_world::World;

use crate::regime::TickMetrics;
use crate::engine::{Engine, TickResult};
use crate::scheduler::{compute_scc_plan, SccPlan};
use crate::{LawCatalog, ProgramCatalog, Stabilizer};

/// Tunable parameters for [`ScheduledDriver`].
#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// Hard cap on the number of inner sub-ticks executed per logical tick.
    /// Includes the initial pass.
    pub max_inner_ticks: u32,
    /// Stop iterating once the latest sub-tick's `total_delta_norm` is at or
    /// below this value.
    pub convergence_epsilon: f32,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            max_inner_ticks: 8,
            convergence_epsilon: 1e-4,
        }
    }
}

/// Result of running a single logical tick through the [`ScheduledDriver`].
#[derive(Debug, Clone)]
pub struct ScheduledTickResult {
    /// SCC plan computed from the snapshot at the start of the logical tick.
    pub plan: SccPlan,
    /// Inner sub-ticks executed, in chronological order. The first entry is
    /// the initial pass; later entries are iterative settle steps.
    pub sub_ticks: Vec<TickResult>,
    /// `true` if the loop terminated because the convergence threshold was
    /// reached, `false` if it hit `max_inner_ticks` first.
    pub converged: bool,
}

impl ScheduledTickResult {
    pub fn iterations(&self) -> usize {
        self.sub_ticks.len()
    }

    /// The transaction of the *last* committed sub-tick — the one that
    /// represents the world state at the end of the logical tick.
    pub fn final_result(&self) -> Option<&TickResult> {
        self.sub_ticks.last()
    }
}

/// Wrap an [`Engine`] with iterative SCC-aware scheduling.
pub struct ScheduledDriver<S> {
    inner: Engine<S>,
    config: ScheduleConfig,
}

impl<S> ScheduledDriver<S>
where
    S: Stabilizer,
{
    pub fn new(inner: Engine<S>, config: ScheduleConfig) -> Self {
        Self { inner, config }
    }

    pub fn config(&self) -> ScheduleConfig {
        self.config
    }

    pub fn inner(&self) -> &Engine<S> {
        &self.inner
    }

    /// Run one logical tick, possibly executing several committed sub-ticks
    /// behind the scenes.
    pub fn tick<P, L>(
        &self,
        tick: TickId,
        world: &mut World,
        programs: &P,
        laws: &L,
        stimuli: &[Stimulus],
    ) -> ScheduledTickResult
    where
        P: ProgramCatalog,
        L: LawCatalog,
    {
        // Snapshot the topology *before* the first commit so that the static
        // plan reflects the world we are about to update.
        let plan = compute_scc_plan(world.snapshot());
        let mut sub_ticks = Vec::with_capacity(self.config.max_inner_ticks as usize);

        // First pass: full pipeline with the user-supplied stimuli.
        let first = self.inner.tick(tick, world, programs, laws, stimuli);
        let mut latest = TickMetrics::from_transaction(&first.transaction);
        sub_ticks.push(first);

        let mut converged = latest.total_delta_norm <= self.config.convergence_epsilon;
        let needs_iteration = !plan.cyclic_components.is_empty();

        if needs_iteration && !converged {
            let max = self.config.max_inner_ticks.max(1);
            for _ in 1..max {
                let next = self.inner.tick(tick, world, programs, laws, &[]);
                latest = TickMetrics::from_transaction(&next.transaction);
                sub_ticks.push(next);
                if latest.total_delta_norm <= self.config.convergence_epsilon {
                    converged = true;
                    break;
                }
            }
        }

        ScheduledTickResult {
            plan,
            sub_ticks,
            converged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_config_defaults_are_reasonable() {
        let cfg = ScheduleConfig::default();
        assert!(cfg.max_inner_ticks >= 2);
        assert!(cfg.convergence_epsilon > 0.0);
    }

    #[test]
    fn scheduled_result_iterations_matches_sub_ticks() {
        // Construct a result by hand to verify the accessor surface without
        // pulling a real engine into a unit test.
        let result = ScheduledTickResult {
            plan: SccPlan::default(),
            sub_ticks: vec![],
            converged: false,
        };
        assert_eq!(result.iterations(), 0);
        assert!(result.final_result().is_none());
    }
}
