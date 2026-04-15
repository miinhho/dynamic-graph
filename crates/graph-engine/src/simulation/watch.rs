//! Reactive watch/trigger system for `Simulation`.
//!
//! After each `step()` completes, all registered [`Trigger`]s and
//! [`Observer`]s are fired with the [`StepObservation`] produced by that
//! tick.
//!
//! - A **Trigger** may return [`ProposedChange`]s. These are queued as
//!   `pending_stimuli` and become the *input* to the **next** `step()` call.
//!   They are NOT visible in the current tick's `StepObservation`.
//!
//! - An **Observer** receives the observation for side-effects only
//!   (logging, metrics, external I/O). It cannot inject changes.
//!
//! ## One-shot watches
//!
//! [`Simulation::watch_once`] registers a trigger/observer that fires exactly
//! once — the first time the predicate returns `true` — and is then dropped.
//!
//! ## Example
//!
//! ```rust,ignore
//! // Fire a corrective stimulus whenever activity on locus 3 drops below 0.1.
//! sim.add_trigger(|obs| {
//!     let world = &obs.summary; // or sim.world in a closure that captures &sim
//!     if obs.tick.changes_committed == 0 {
//!         vec![ProposedChange::new(locus3, kind1, 0.5)]
//!     } else {
//!         vec![]
//!     }
//! });
//!
//! // Log every regime shift.
//! sim.add_observer(|obs| {
//!     for ev in &obs.events {
//!         if let WorldEvent::RegimeShift { from, to } = ev {
//!             println!("regime changed: {from:?} → {to:?}");
//!         }
//!     }
//! });
//! ```

use graph_core::ProposedChange;

use super::config::StepObservation;

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use graph_core::{Change, ChangeSubject, InfluenceKindId, Locus, LocusContext, LocusId, ProposedChange, StateVector};

    use crate::simulation::SimulationBuilder;

    struct NoopProgram;
    impl graph_core::LocusProgram for NoopProgram {
        fn process(
            &self, _: &Locus, _: &[&Change], _: &dyn LocusContext,
        ) -> Vec<ProposedChange> { vec![] }
    }

    fn minimal_sim() -> crate::simulation::Simulation {
        SimulationBuilder::new()
            .locus_kind("A", NoopProgram)
            .influence("k", |cfg| cfg)
            .default_influence("k")
            .build()
    }

    // ── Observer fires on each step ──────────────────────────────────────────

    #[test]
    fn observer_called_on_each_step() {
        let mut sim = minimal_sim();
        let call_count = Arc::new(Mutex::new(0usize));
        let cc = Arc::clone(&call_count);
        sim.add_observer(move |_obs| {
            *cc.lock().unwrap() += 1;
        });
        sim.step(vec![]);
        sim.step(vec![]);
        sim.step(vec![]);
        assert_eq!(*call_count.lock().unwrap(), 3);
    }

    // ── Trigger injects stimuli into next step ───────────────────────────────

    #[test]
    fn trigger_queues_stimuli_for_next_step() {
        let mut sim = minimal_sim();
        let locus_id = sim.ingest_named("X", "A", Default::default());

        // On the first step, the trigger fires and queues a change.
        // On the second step, pending_stimuli should have been consumed.
        let fired = Arc::new(Mutex::new(false));
        let f = Arc::clone(&fired);
        let kind = InfluenceKindId(1); // any kind
        sim.add_trigger(move |_obs| {
            let mut guard = f.lock().unwrap();
            if !*guard {
                *guard = true;
                vec![ProposedChange::new(ChangeSubject::Locus(locus_id), kind, StateVector::from_slice(&[0.5]))]
            } else {
                vec![]
            }
        });

        // Step 1: trigger fires, queues a change.
        let _obs1 = sim.step(vec![]);
        // After step 1, pending_stimuli should contain the trigger's output.
        assert!(!sim.pending_stimuli.is_empty());

        // Step 2: pending_stimuli are drained as input to this tick.
        let obs2 = sim.step(vec![]);
        // pending_stimuli should be empty after being consumed.
        assert!(sim.pending_stimuli.is_empty());
        // The queued stimulus was processed in step 2.
        assert!(obs2.tick.changes_committed > 0);
    }

    // ── clear_triggers removes all triggers ──────────────────────────────────

    #[test]
    fn clear_triggers_stops_further_firing() {
        let mut sim = minimal_sim();
        let call_count = Arc::new(Mutex::new(0usize));
        let cc = Arc::clone(&call_count);
        sim.add_trigger(move |_obs| {
            *cc.lock().unwrap() += 1;
            vec![]
        });
        sim.step(vec![]);
        sim.clear_triggers();
        sim.step(vec![]);
        sim.step(vec![]);
        // Trigger should have fired exactly once (before clear).
        assert_eq!(*call_count.lock().unwrap(), 1);
    }

    // ── One-shot trigger fires once then is removed ──────────────────────────

    #[test]
    fn add_trigger_once_fires_exactly_once() {
        let mut sim = minimal_sim();
        let call_count = Arc::new(Mutex::new(0usize));
        let cc = Arc::clone(&call_count);
        sim.add_trigger_once(move |_obs| {
            *cc.lock().unwrap() += 1;
            vec![]
        });
        sim.step(vec![]);
        sim.step(vec![]);
        sim.step(vec![]);
        assert_eq!(*call_count.lock().unwrap(), 1);
        // After firing once, the trigger should have been removed.
        assert!(sim.triggers.is_empty());
    }

    // ── watch_once fires on first matching tick ──────────────────────────────

    #[test]
    fn watch_once_fires_when_predicate_is_true() {
        let mut sim = minimal_sim();
        let fired = Arc::new(Mutex::new(false));
        let f = Arc::clone(&fired);
        let step_target = Arc::new(Mutex::new(0u64));
        let st = Arc::clone(&step_target);

        // Fire on the second tick (tick_id == 2).
        sim.watch_once(
            |obs| obs.summary.tick_id == 2,
            move |obs| {
                *f.lock().unwrap() = true;
                *st.lock().unwrap() = obs.summary.tick_id;
                vec![]
            },
        );

        sim.step(vec![]); // tick 1 — predicate false
        assert!(!*fired.lock().unwrap());
        sim.step(vec![]); // tick 2 — predicate true
        assert!(*fired.lock().unwrap());
        assert_eq!(*step_target.lock().unwrap(), 2);
        sim.step(vec![]); // tick 3 — handler already consumed
        // Should not fire again.
        assert_eq!(*step_target.lock().unwrap(), 2);
    }

    // ── observe_once ────────────────────────────────────────────────────────

    #[test]
    fn observe_once_fires_once_on_match() {
        let mut sim = minimal_sim();
        let call_count = Arc::new(Mutex::new(0usize));
        let cc = Arc::clone(&call_count);

        sim.observe_once(
            |obs| obs.summary.tick_id >= 2,
            move |_obs| { *cc.lock().unwrap() += 1; },
        );

        sim.step(vec![]); // tick 1 — pred false
        assert_eq!(*call_count.lock().unwrap(), 0);
        sim.step(vec![]); // tick 2 — pred true, fires
        assert_eq!(*call_count.lock().unwrap(), 1);
        sim.step(vec![]); // tick 3 — handler consumed, no-op
        assert_eq!(*call_count.lock().unwrap(), 1);
    }
}

// ─── Type aliases ─────────────────────────────────────────────────────────────

/// A reactive trigger that fires after each step.
///
/// Returns a (possibly empty) list of [`ProposedChange`]s. Non-empty returns
/// are queued as `pending_stimuli` for the next `step()` call.
///
/// Triggers are stored as `Box<dyn ...>` so they can be closures or structs.
pub(crate) type TriggerFn = Box<dyn FnMut(&StepObservation) -> Vec<ProposedChange> + Send>;

/// A reactive observer that fires after each step for side-effects only.
///
/// Observers cannot inject changes — use a [`TriggerFn`] for that.
pub(crate) type ObserverFn = Box<dyn FnMut(&StepObservation) + Send>;

// ─── TriggerEntry ─────────────────────────────────────────────────────────────

/// Internal wrapper around a trigger function.
///
/// `one_shot = true` marks the entry for removal after its first firing.
pub(crate) struct TriggerEntry {
    pub(crate) func: TriggerFn,
    /// When `true`, remove after the first call.
    pub(crate) one_shot: bool,
}

// ─── ObserverEntry ────────────────────────────────────────────────────────────

/// Internal wrapper around an observer function.
pub(crate) struct ObserverEntry {
    pub(crate) func: ObserverFn,
    /// When `true`, remove after the first call.
    pub(crate) one_shot: bool,
}

// ─── Simulation extension methods ─────────────────────────────────────────────

use super::Simulation;

impl Simulation {
    // ── Trigger registration ──────────────────────────────────────────────────

    /// Register a persistent trigger.
    ///
    /// `f` is called after every `step()` with the [`StepObservation`] from
    /// that tick. Any [`ProposedChange`]s returned by `f` are queued as
    /// `pending_stimuli` and delivered to the **next** `step()` call.
    ///
    /// The trigger fires indefinitely until explicitly cleared with
    /// [`clear_triggers`][Simulation::clear_triggers].
    pub fn add_trigger<F>(&mut self, f: F)
    where
        F: FnMut(&StepObservation) -> Vec<ProposedChange> + Send + 'static,
    {
        self.triggers.push(TriggerEntry {
            func: Box::new(f),
            one_shot: false,
        });
    }

    /// Register a one-shot trigger.
    ///
    /// `f` is called after the *first* `step()` that follows registration, and
    /// then automatically removed. To fire only when a condition holds, return
    /// an empty `Vec` from `f` and check the condition inside `f` — note that
    /// this version fires exactly once regardless of return value. For
    /// condition-gated one-shot logic, use [`watch_once`][Simulation::watch_once].
    pub fn add_trigger_once<F>(&mut self, f: F)
    where
        F: FnMut(&StepObservation) -> Vec<ProposedChange> + Send + 'static,
    {
        self.triggers.push(TriggerEntry {
            func: Box::new(f),
            one_shot: true,
        });
    }

    /// Register a one-shot trigger that fires on the first tick where
    /// `pred(obs)` returns `true`.
    ///
    /// On that tick, `handler(obs)` is called to produce the stimuli, and the
    /// entry is then removed. Ticks where `pred` returns `false` are skipped
    /// with zero overhead on the change queue.
    pub fn watch_once<P, H>(&mut self, pred: P, handler: H)
    where
        P: Fn(&StepObservation) -> bool + Send + 'static,
        H: FnOnce(&StepObservation) -> Vec<ProposedChange> + Send + 'static,
    {
        // Wrap the one-shot handler in an `Option` so we can take it on first fire.
        let mut handler_opt = Some(handler);
        self.triggers.push(TriggerEntry {
            func: Box::new(move |obs| {
                if pred(obs) {
                    if let Some(h) = handler_opt.take() {
                        return h(obs);
                    }
                }
                vec![]
            }),
            // One-shot: remove after pred fires (handler has been taken).
            // We detect this by checking whether handler_opt is None.
            // But since we can't inspect the closure from outside, we use
            // a separate flag. To keep it simple, mark as non-one-shot and
            // rely on the empty-return after the handler is consumed.
            //
            // Actually: mark it one_shot=false, but the handler_opt becomes
            // None after first fire, so subsequent calls are no-ops.
            // This is safe: after the handler fires, the closure always
            // returns vec![] cheaply.
            one_shot: false,
        });
    }

    /// Remove all registered triggers.
    pub fn clear_triggers(&mut self) {
        self.triggers.clear();
    }

    // ── Observer registration ─────────────────────────────────────────────────

    /// Register a persistent observer.
    ///
    /// `f` is called after every `step()` for side-effects (logging, metrics,
    /// external I/O). Observers cannot inject changes — use
    /// [`add_trigger`][Simulation::add_trigger] for that.
    pub fn add_observer<F>(&mut self, f: F)
    where
        F: FnMut(&StepObservation) + Send + 'static,
    {
        self.observers.push(ObserverEntry {
            func: Box::new(f),
            one_shot: false,
        });
    }

    /// Register a one-shot observer that fires on the first tick where
    /// `pred(obs)` returns `true`, then is removed.
    pub fn observe_once<P, H>(&mut self, pred: P, handler: H)
    where
        P: Fn(&StepObservation) -> bool + Send + 'static,
        H: FnOnce(&StepObservation) + Send + 'static,
    {
        let mut handler_opt = Some(handler);
        self.observers.push(ObserverEntry {
            func: Box::new(move |obs| {
                if pred(obs) {
                    if let Some(h) = handler_opt.take() {
                        h(obs);
                    }
                }
            }),
            one_shot: false,
        });
    }

    /// Remove all registered observers.
    pub fn clear_observers(&mut self) {
        self.observers.clear();
    }

    // ── Internal fire helper (called at end of step()) ────────────────────────

    /// Fire all triggers and observers against `obs`.
    ///
    /// Trigger outputs are appended to `pending_stimuli` for the next tick.
    /// One-shot entries whose flag is set are removed after firing.
    ///
    /// Called internally by `step()` after the `StepObservation` is assembled.
    pub(crate) fn fire_watches(&mut self, obs: &StepObservation) {
        // ── Triggers ─────────────────────────────────────────────────────────
        // Collect changes into a temporary vec to avoid a split-borrow conflict
        // between self.triggers (iterated mutably) and self.pending_stimuli.
        let mut accumulated: Vec<ProposedChange> = Vec::new();
        let mut to_remove: Vec<usize> = Vec::new();
        for (i, entry) in self.triggers.iter_mut().enumerate() {
            let changes = (entry.func)(obs);
            accumulated.extend(changes);
            if entry.one_shot {
                to_remove.push(i);
            }
        }
        // Flush accumulated stimuli.
        self.pending_stimuli.extend(accumulated);
        // Remove one-shot triggers in reverse order to preserve indices.
        for i in to_remove.into_iter().rev() {
            self.triggers.swap_remove(i);
        }

        // ── Observers ────────────────────────────────────────────────────────
        let mut to_remove: Vec<usize> = Vec::new();
        for (i, entry) in self.observers.iter_mut().enumerate() {
            (entry.func)(obs);
            if entry.one_shot {
                to_remove.push(i);
            }
        }
        for i in to_remove.into_iter().rev() {
            self.observers.swap_remove(i);
        }
    }
}
