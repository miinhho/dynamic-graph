//! The substrate batch loop.
//!
//! Implements the core of `docs/redesign.md` §6: take a set of pending
//! changes, commit them as the current batch, dispatch each affected
//! locus's program, queue any newly proposed changes for the next
//! batch, and repeat until quiescent or until a configurable cap fires.
//!
//! This commit handles the simplest case: a stimulus that flows into
//! one locus, whose program may produce zero or more follow-up changes
//! that target the same locus. Cross-locus change flow lands in a
//! follow-up commit, which will also bring per-kind stabilization and
//! the relationship-emergence path.
//!
//! Design decisions in force here, all from `docs/redesign.md` §8:
//! - **Predecessors are auto-derived** (O1 hybrid, automatic side):
//!   internal changes inherit the ids of the changes that fired into
//!   their subject locus during the same batch. `extra_predecessors`
//!   on a `ProposedChange` are unioned in if present.
//! - **Stimulus = Change with empty predecessors** (O9): user-injected
//!   `ProposedChange`s are committed with no predecessors.
//! - **Single-subject changes only** (O7 tentative).
//! - **Locus state = `change.after`** on commit. The previous state
//!   becomes `change.before`.

use std::collections::HashMap;

use graph_core::{Change, ChangeId, ChangeSubject, LocusId, ProposedChange};
use graph_world::World;

use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Hard cap on the number of batches a single `tick` call may
    /// process before bailing out. Prevents an infinite cascade if a
    /// program is non-quiescent. Default: 64.
    pub max_batches_per_tick: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_batches_per_tick: 64,
        }
    }
}

/// Summary of one `tick` call.
#[derive(Debug, Clone, Default)]
pub struct TickResult {
    pub batches_committed: u32,
    pub changes_committed: u32,
    /// True if the loop stopped because `max_batches_per_tick` fired
    /// rather than because the system went quiescent. A caller can use
    /// this as a signal to escalate (raise the cap, log, etc.).
    pub hit_batch_cap: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Engine {
    config: EngineConfig,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Run the batch loop until quiescent or the per-tick cap fires.
    ///
    /// `stimuli` are the root changes that kick this tick off. Per O9
    /// they are just `ProposedChange`s with no predecessors; the engine
    /// commits them as the first batch's content.
    pub fn tick(
        &self,
        world: &mut World,
        loci_registry: &LocusKindRegistry,
        _influence_registry: &InfluenceKindRegistry,
        stimuli: Vec<ProposedChange>,
    ) -> TickResult {
        let mut result = TickResult::default();
        let mut pending: Vec<PendingChange> = stimuli
            .into_iter()
            .map(|proposed| PendingChange {
                proposed,
                derived_predecessors: Vec::new(),
            })
            .collect();

        while !pending.is_empty() {
            if result.batches_committed >= self.config.max_batches_per_tick {
                result.hit_batch_cap = true;
                break;
            }

            // Commit every pending change as part of the current batch.
            // Build a per-locus index of which change ids fired into
            // each locus, so the next batch's auto-predecessor logic
            // has somewhere to look.
            let batch = world.current_batch();
            let mut committed_ids_by_locus: HashMap<LocusId, Vec<ChangeId>> = HashMap::new();
            let mut affected_loci: Vec<LocusId> = Vec::new();

            for pending_change in pending.drain(..) {
                let PendingChange {
                    proposed,
                    derived_predecessors,
                } = pending_change;

                let ChangeSubject::Locus(locus_id) = proposed.subject;

                // The before-state is the locus's current state at the
                // moment of commit; the after-state was supplied by the
                // proposer (stimulus or program).
                let before = world
                    .locus(locus_id)
                    .map(|l| l.state.clone())
                    .unwrap_or_default();

                let mut predecessors = derived_predecessors;
                predecessors.extend(proposed.extra_predecessors.iter().copied());

                let id = world.mint_change_id();
                let change = Change {
                    id,
                    subject: ChangeSubject::Locus(locus_id),
                    kind: proposed.kind,
                    predecessors,
                    before,
                    after: proposed.after.clone(),
                    batch,
                };

                // Apply the state change to the locus, then record the
                // change in the log. Order matters: state must reflect
                // the change before any program runs against it.
                if let Some(locus) = world.locus_mut(locus_id) {
                    locus.state = proposed.after;
                }
                world.append_change(change);

                committed_ids_by_locus.entry(locus_id).or_default().push(id);
                if !affected_loci.contains(&locus_id) {
                    affected_loci.push(locus_id);
                }
                result.changes_committed += 1;
            }

            // Dispatch programs for every locus that just received at
            // least one change. Each program returns its proposed
            // follow-up changes, which we queue for the next batch.
            for locus_id in &affected_loci {
                let Some(locus) = world.locus(*locus_id) else {
                    continue;
                };
                let program = match loci_registry.require(locus.kind) {
                    Some(p) => p,
                    None => continue,
                };

                // Build the inbox for this locus: every change committed
                // to this locus during the batch we just sealed.
                let inbox: Vec<Change> = committed_ids_by_locus
                    .get(locus_id)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|id| world.log().get(*id).cloned())
                            .collect()
                    })
                    .unwrap_or_default();

                let proposals = program.process(locus, &inbox);
                let derived: Vec<ChangeId> =
                    inbox.iter().map(|c| c.id).collect();

                pending.extend(proposals.into_iter().map(|p| PendingChange {
                    proposed: p,
                    derived_predecessors: derived.clone(),
                }));
            }

            world.advance_batch();
            result.batches_committed += 1;
        }

        result
    }
}

/// A change in flight: the user/program-supplied proposal plus any
/// predecessors the engine derived from the previous batch's commits.
struct PendingChange {
    proposed: ProposedChange,
    derived_predecessors: Vec<ChangeId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, InfluenceKindId, Locus, LocusId, LocusKindId, LocusProgram,
        ProposedChange, StateVector,
    };

    /// A program that, on its first activation, produces one self-targeted
    /// follow-up change with `after = current * 0.5`. On subsequent
    /// activations it does nothing — so the loop converges in two batches.
    struct DampOnceProgram;

    impl LocusProgram for DampOnceProgram {
        fn process(&self, locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
            // Only react to a stimulus (predecessors empty); ignore the
            // damped follow-up so the loop quiesces.
            if incoming.iter().all(|c| c.predecessors.is_empty()) {
                let mut next = locus.state.clone();
                for slot in next.as_mut_slice() {
                    *slot *= 0.5;
                }
                vec![ProposedChange::new(
                    ChangeSubject::Locus(locus.id),
                    InfluenceKindId(1),
                    next,
                )]
            } else {
                Vec::new()
            }
        }
    }

    fn setup() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(DampOnceProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("test"),
        );
        (world, loci, influences)
    }

    #[test]
    fn stimulus_only_commits_one_batch_when_program_is_passive() {
        struct InertProgram;
        impl LocusProgram for InertProgram {
            fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
                Vec::new()
            }
        }
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(InertProgram));
        let influences = InfluenceKindRegistry::new();

        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        );

        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        assert_eq!(result.batches_committed, 1);
        assert_eq!(result.changes_committed, 1);
        assert!(!result.hit_batch_cap);

        // Stimulus state landed.
        let state = &world.locus(LocusId(1)).unwrap().state;
        assert_eq!(state.as_slice(), &[1.0, 1.0]);
    }

    #[test]
    fn stimulus_followed_by_program_response_commits_two_batches() {
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();

        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        );

        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        assert_eq!(result.batches_committed, 2);
        assert_eq!(result.changes_committed, 2);
        assert!(!result.hit_batch_cap);

        // After damping, state should be 0.5,0.5.
        let state = &world.locus(LocusId(1)).unwrap().state;
        assert_eq!(state.as_slice(), &[0.5, 0.5]);
    }

    #[test]
    fn internal_change_inherits_stimulus_as_predecessor() {
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[2.0, 0.0]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);

        let log: Vec<&Change> = world.log().iter().collect();
        assert_eq!(log.len(), 2);
        // First entry is the stimulus — no predecessors.
        assert!(log[0].is_stimulus());
        // Second entry is the program's response — its predecessor set
        // must contain the stimulus's id (auto-derived).
        assert_eq!(log[1].predecessors, vec![log[0].id]);
        // And it lives in the next batch.
        assert_eq!(log[0].batch, BatchId(0));
        assert_eq!(log[1].batch, BatchId(1));
    }

    #[test]
    fn batch_cap_engages_on_runaway_program() {
        // A pathological program that always produces another change.
        struct InfiniteProgram;
        impl LocusProgram for InfiniteProgram {
            fn process(&self, locus: &Locus, _: &[Change]) -> Vec<ProposedChange> {
                vec![ProposedChange::new(
                    ChangeSubject::Locus(locus.id),
                    InfluenceKindId(1),
                    locus.state.clone(),
                )]
            }
        }
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(InfiniteProgram));
        let influences = InfluenceKindRegistry::new();

        let engine = Engine::new(EngineConfig {
            max_batches_per_tick: 5,
        });
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[0.1]),
        );
        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        assert!(result.hit_batch_cap);
        assert_eq!(result.batches_committed, 5);
    }

    /// A program that, on stimulus, fires a single change at a fixed
    /// "downstream" locus and then stays inert. Used to drive cross-locus
    /// flow without infinite cascade.
    struct ForwarderProgram {
        downstream: LocusId,
    }
    impl LocusProgram for ForwarderProgram {
        fn process(&self, _locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
            // Only react to stimuli; ignore anything internal so the
            // loop quiesces after one hand-off.
            if !incoming.iter().all(|c| c.predecessors.is_empty()) {
                return Vec::new();
            }
            // Forward the magnitude of the first incoming change to the
            // downstream locus.
            let after = incoming[0].after.clone();
            vec![ProposedChange::new(
                ChangeSubject::Locus(self.downstream),
                InfluenceKindId(1),
                after,
            )]
        }
    }

    /// Sink program — accepts incoming, never proposes anything.
    struct SinkProgram;
    impl LocusProgram for SinkProgram {
        fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    #[test]
    fn cross_locus_change_lands_on_downstream_with_correct_predecessor() {
        // Two loci of two different kinds: a forwarder and a sink. A
        // stimulus hits the forwarder; the forwarder's program proposes
        // a change at the sink. After the loop quiesces, the sink's
        // state must equal the stimulus payload, and the cross-locus
        // change must list the stimulus as its causal predecessor.
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        world.insert_locus(Locus::new(
            LocusId(2),
            LocusKindId(2),
            StateVector::zeros(2),
        ));

        let mut loci = LocusKindRegistry::new();
        loci.insert(
            LocusKindId(1),
            Box::new(ForwarderProgram {
                downstream: LocusId(2),
            }),
        );
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("excite"),
        );

        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[0.7, 0.0]),
        );
        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);

        // Two batches: stimulus, then forwarded.
        assert_eq!(result.batches_committed, 2);
        assert_eq!(result.changes_committed, 2);

        // Sink received the payload.
        assert_eq!(
            world.locus(LocusId(2)).unwrap().state.as_slice(),
            &[0.7, 0.0]
        );
        // Forwarder still holds the stimulus payload (the program does
        // not modify itself).
        assert_eq!(
            world.locus(LocusId(1)).unwrap().state.as_slice(),
            &[0.7, 0.0]
        );

        // Causal chain: stimulus on L1 (batch 0, no preds) -> forwarded
        // change on L2 (batch 1, predecessor = stimulus id).
        let log: Vec<&Change> = world.log().iter().collect();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, ChangeSubject::Locus(LocusId(1)));
        assert_eq!(log[0].batch, BatchId(0));
        assert!(log[0].is_stimulus());

        assert_eq!(log[1].subject, ChangeSubject::Locus(LocusId(2)));
        assert_eq!(log[1].batch, BatchId(1));
        assert_eq!(log[1].predecessors, vec![log[0].id]);
    }

    #[test]
    fn changes_to_locus_returns_full_history() {
        // Drive two stimuli through the same locus across separate ticks
        // and confirm the change log preserves both, ordered newest first
        // when queried via the world's per-locus accessor.
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(SinkProgram));
        let influences = InfluenceKindRegistry::new();

        let engine = Engine::default();
        for value in [0.1_f32, 0.2, 0.3] {
            let stimulus = ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[value]),
            );
            engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        }

        let to_locus: Vec<f32> = world
            .log()
            .changes_to_locus(LocusId(1))
            .map(|c| c.after.as_slice()[0])
            .collect();
        assert_eq!(to_locus, vec![0.3, 0.2, 0.1]);
    }
}
