//! Engine controller: owns the `Simulation` and drives the tick loop.
//!
//! [`EngineController`] is the owner of the simulation. It wraps `Simulation`
//! in a shared `Arc<Mutex<...>>` and hands out [`LocalHandle`]s to callers.
//!
//! ## Tick policies
//!
//! The controller supports three tick policies via [`TickPolicy`]:
//!
//! - **`Manual`** — the controller never ticks on its own. Callers drive ticks
//!   explicitly through handles (`handle.step(...)`). Good for tests and
//!   scripts where you want full control of the step cadence.
//!
//! - **`ChangeDriven { heartbeat_ms }`** — ticks fire whenever `pending_stimuli`
//!   is non-empty; a heartbeat tick fires every `heartbeat_ms` milliseconds
//!   even when the queue is empty (for decay and auto-weathering). Requires the
//!   `async` feature and a tokio runtime (not yet implemented).
//!
//! - **`ClockDriven { interval_ms }`** — ticks fire on a fixed wall-clock
//!   interval regardless of pending stimuli. Good for physics-simulation style
//!   loops where time advances uniformly. Requires the `async` feature
//!   (not yet implemented).
//!
//! ## Example
//!
//! ```ignore
//! use graph_engine::{EngineController, TickPolicy};
//!
//! let controller = EngineController::new(sim, TickPolicy::Manual);
//! let handle = controller.handle();
//!
//! // Tick manually:
//! let obs = handle.step(vec![]);
//! println!("batch: {:?}", obs.tick.batches_committed);
//!
//! // Recover the Simulation (only works when no other handles are alive):
//! let sim = controller.into_simulation();
//! ```

use std::sync::{Arc, Mutex};

use crate::handle::LocalHandle;
use crate::simulation::Simulation;

#[cfg(feature = "tokio")]
use std::time::Duration;
#[cfg(feature = "tokio")]
use graph_core::WorldEvent;

// ── TickPolicy ────────────────────────────────────────────────────────────────

/// Controls when the engine ticks.
///
/// See the [module-level documentation](self) for a full description of each
/// variant. `ChangeDriven` and `ClockDriven` describe the intended policy but
/// do not start a background loop on their own — that requires an `async`
/// runtime integration that is not yet implemented.
#[derive(Debug, Clone)]
pub enum TickPolicy {
    /// Caller drives all ticks through handle methods. No background loop.
    Manual,

    /// Tick whenever pending stimuli are present; heartbeat every N ms when idle.
    ///
    /// `heartbeat_ms` must be > 0. A value of 0 is treated as `Manual`.
    ///
    /// **Not yet wired to a background loop.** Constructing this variant records
    /// the intent; attach it to a tokio task with [`needs_background_loop`].
    ///
    /// [`needs_background_loop`]: TickPolicy::needs_background_loop
    ChangeDriven {
        /// How often (in milliseconds) to fire a heartbeat tick when idle.
        heartbeat_ms: u64,
    },

    /// Tick on a fixed wall-clock interval regardless of pending stimuli.
    ///
    /// **Not yet wired to a background loop.** See [`ChangeDriven`] note.
    ///
    /// [`ChangeDriven`]: TickPolicy::ChangeDriven
    ClockDriven {
        /// Tick interval in milliseconds.
        interval_ms: u64,
    },
}

impl TickPolicy {
    /// Returns `true` if this policy requires a background tick loop.
    ///
    /// Callers can use this to decide whether to spawn a tokio task. The loop
    /// itself is not started automatically — see module docs.
    pub fn needs_background_loop(&self) -> bool {
        match self {
            TickPolicy::Manual => false,
            TickPolicy::ChangeDriven { heartbeat_ms } => *heartbeat_ms > 0,
            TickPolicy::ClockDriven { interval_ms } => *interval_ms > 0,
        }
    }
}

// ── EngineController ──────────────────────────────────────────────────────────

/// Owns the simulation and manages handle distribution.
///
/// The controller holds the authoritative `Arc<Mutex<Simulation>>` — all
/// handles share the same underlying mutex. The controller is the only path
/// for obtaining handles and for unwrapping the `Simulation` when done.
///
/// ## Starting the background loop
///
/// For `ChangeDriven` and `ClockDriven` policies, call [`start`] after
/// constructing the controller. The loop is **not** started automatically in
/// [`new`] to avoid requiring a tokio runtime at construction time.
///
/// ```ignore
/// let mut ctrl = EngineController::new(sim, TickPolicy::ChangeDriven { heartbeat_ms: 500 });
/// ctrl.start(); // spawns tokio task; requires an active tokio runtime
/// let handle = ctrl.handle();
/// // … ingest events; background loop wakes and steps automatically …
/// ctrl.stop(); // abort the task (also called on Drop)
/// ```
///
/// [`start`]: EngineController::start
/// [`new`]: EngineController::new
pub struct EngineController {
    sim: Arc<Mutex<Simulation>>,
    policy: TickPolicy,
    /// Shared wake signal — sent by handle ingest calls to wake the background loop.
    #[cfg(feature = "tokio")]
    notify: Arc<tokio::sync::Notify>,
    /// Broadcast channel for world events emitted by ticks and entity operations.
    /// Handles subscribe via `subscribe_world_events()`.
    #[cfg(feature = "tokio")]
    event_tx: Arc<tokio::sync::broadcast::Sender<WorldEvent>>,
    /// Background tick task (present only after `start()` and before `stop()`).
    #[cfg(feature = "tokio")]
    task: Option<tokio::task::JoinHandle<()>>,
}

impl EngineController {
    /// Wrap a `Simulation` in a controller with the given [`TickPolicy`].
    ///
    /// For `ChangeDriven` or `ClockDriven` policies, call [`start`] afterwards
    /// to launch the background tick loop.
    ///
    /// [`start`]: EngineController::start
    pub fn new(sim: Simulation, policy: TickPolicy) -> Self {
        #[cfg(feature = "tokio")]
        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            sim: Arc::new(Mutex::new(sim)),
            policy,
            #[cfg(feature = "tokio")]
            notify: Arc::new(tokio::sync::Notify::new()),
            #[cfg(feature = "tokio")]
            event_tx: Arc::new(event_tx),
            #[cfg(feature = "tokio")]
            task: None,
        }
    }

    /// Issue a [`LocalHandle`] to this controller's simulation.
    ///
    /// All handles share the same underlying `Arc` and therefore the same
    /// `Mutex<Simulation>`. Handles are cheap to clone.
    ///
    /// When the `tokio` feature is enabled and a background loop is running,
    /// the handle's ingest methods will wake the loop immediately after queuing
    /// each stimulus, rather than waiting for the next heartbeat or interval.
    pub fn handle(&self) -> LocalHandle {
        #[cfg(feature = "tokio")]
        return LocalHandle::new_with_notify(
            Arc::clone(&self.sim),
            Arc::clone(&self.notify),
            Arc::clone(&self.event_tx),
        );
        #[cfg(not(feature = "tokio"))]
        return LocalHandle::new(Arc::clone(&self.sim));
    }

    /// The tick policy this controller was constructed with.
    pub fn policy(&self) -> &TickPolicy {
        &self.policy
    }

    /// Number of live [`LocalHandle`] clones outstanding from this controller.
    ///
    /// Returns 0 when only the controller itself holds the Arc.
    pub fn live_handle_count(&self) -> usize {
        // Arc::strong_count includes the controller's own reference, so subtract 1.
        Arc::strong_count(&self.sim).saturating_sub(1)
    }

    /// Consume the controller and return the inner `Simulation`.
    ///
    /// Aborts any running background task before unwrapping.
    ///
    /// # Panics
    ///
    /// Panics if any `LocalHandle` clones are still alive. Drop all handles
    /// before calling this, or use [`try_into_simulation`](Self::try_into_simulation).
    pub fn into_simulation(self) -> Simulation {
        // Clone the Arc before dropping self (which aborts the background task via Drop).
        let arc = Arc::clone(&self.sim);
        drop(self);
        match Arc::try_unwrap(arc) {
            Ok(mutex) => mutex.into_inner().unwrap(),
            Err(_) => panic!("EngineController::into_simulation: live LocalHandle clones still exist; drop all handles before unwrapping"),
        }
    }

    /// Try to consume the controller and return the inner `Simulation`.
    ///
    /// Returns `Err(self)` if any `LocalHandle` clones are still alive.
    /// On success, aborts any running background task before returning.
    pub fn try_into_simulation(self) -> Result<Simulation, Self> {
        if Arc::strong_count(&self.sim) > 1 {
            return Err(self);
        }
        // Clone before dropping so we hold the last reference after Drop runs.
        let arc = Arc::clone(&self.sim);
        drop(self); // aborts background task, releases controller's Arc reference
        Ok(Arc::try_unwrap(arc)
            .unwrap_or_else(|_| panic!("unexpected live handle after strong_count check"))
            .into_inner()
            .unwrap())
    }

    /// Start the background tick loop.
    ///
    /// - **`Manual`**: no-op.
    /// - **`ChangeDriven { heartbeat_ms }`**: spawns a tokio task that wakes
    ///   whenever any handle calls an ingest method *or* the heartbeat interval
    ///   elapses (whichever comes first), then calls `step([])`.
    /// - **`ClockDriven { interval_ms }`**: spawns a tokio task that calls
    ///   `step([])` on a fixed wall-clock interval, skipping missed ticks.
    ///
    /// Calling `start` when a task is already running is a no-op. The method
    /// must be called from within a tokio runtime context (e.g. inside
    /// `#[tokio::main]` or `tokio::spawn`).
    ///
    /// # Panics
    ///
    /// Panics if called with `ChangeDriven { heartbeat_ms: 0 }` or
    /// `ClockDriven { interval_ms: 0 }` — zero-duration intervals are not
    /// meaningful and would busy-loop.
    #[cfg(feature = "tokio")]
    pub fn start(&mut self) {
        if self.task.is_some() {
            return;
        }
        let sim = Arc::clone(&self.sim);
        let notify = Arc::clone(&self.notify);
        let event_tx = Arc::clone(&self.event_tx);
        self.task = match self.policy {
            TickPolicy::Manual => return,
            TickPolicy::ChangeDriven { heartbeat_ms } => {
                assert!(heartbeat_ms > 0, "ChangeDriven heartbeat_ms must be > 0");
                let heartbeat = Duration::from_millis(heartbeat_ms);
                Some(tokio::spawn(async move {
                    loop {
                        let _ = tokio::time::timeout(heartbeat, notify.notified()).await;
                        let obs = sim.lock().unwrap().step(vec![]);
                        for event in obs.events {
                            let _ = event_tx.send(event);
                        }
                    }
                }))
            }
            TickPolicy::ClockDriven { interval_ms } => {
                assert!(interval_ms > 0, "ClockDriven interval_ms must be > 0");
                let period = Duration::from_millis(interval_ms);
                Some(tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(period);
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        ticker.tick().await;
                        let obs = sim.lock().unwrap().step(vec![]);
                        for event in obs.events {
                            let _ = event_tx.send(event);
                        }
                    }
                }))
            }
        };
    }

    /// Abort the background tick loop if one is running.
    ///
    /// No-op when no task is active. Also called automatically on [`Drop`].
    #[cfg(feature = "tokio")]
    pub fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(feature = "tokio")]
impl Drop for EngineController {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        ChangeSubject, InfluenceKindId, Locus, LocusId, LocusKindId, LocusProgram,
        ProposedChange, StateVector,
    };
    use graph_world::World;
    use crate::registry::{InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry};
    use crate::handle::EngineHandle;

    const KIND: LocusKindId = LocusKindId(1);
    const SIGNAL: InfluenceKindId = InfluenceKindId(1);

    struct Inert;
    impl LocusProgram for Inert {
        fn process(&self, _: &Locus, _: &[&graph_core::Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    fn simple_sim() -> Simulation {
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(0), KIND, StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(KIND, Box::new(Inert));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(SIGNAL, InfluenceKindConfig::new("test").with_decay(0.9));
        Simulation::new(world, loci, influences)
    }

    fn stimulus() -> ProposedChange {
        ProposedChange::new(
            ChangeSubject::Locus(LocusId(0)),
            SIGNAL,
            StateVector::from_slice(&[1.0]),
        )
    }

    #[test]
    fn handle_step_advances_batch() {
        let ctrl = EngineController::new(simple_sim(), TickPolicy::Manual);
        let handle = ctrl.handle();
        let obs = handle.step(vec![stimulus()]);
        assert!(obs.tick.batches_committed > 0);
    }

    #[test]
    fn multiple_handles_share_state() {
        let ctrl = EngineController::new(simple_sim(), TickPolicy::Manual);
        let h1 = ctrl.handle();
        let h2 = ctrl.handle();
        h1.step(vec![stimulus()]);
        assert_eq!(h1.current_batch(), h2.current_batch());
    }

    #[test]
    fn live_handle_count_tracks_clones() {
        let ctrl = EngineController::new(simple_sim(), TickPolicy::Manual);
        assert_eq!(ctrl.live_handle_count(), 0);
        let h1 = ctrl.handle();
        assert_eq!(ctrl.live_handle_count(), 1);
        let h2 = h1.clone();
        assert_eq!(ctrl.live_handle_count(), 2);
        drop(h1);
        assert_eq!(ctrl.live_handle_count(), 1);
        drop(h2);
        assert_eq!(ctrl.live_handle_count(), 0);
    }

    #[test]
    fn try_into_simulation_fails_with_live_handle() {
        let ctrl = EngineController::new(simple_sim(), TickPolicy::Manual);
        let _h = ctrl.handle();
        let ctrl = match ctrl.try_into_simulation() {
            Err(c) => c,
            Ok(_) => panic!("should fail while handle is alive"),
        };
        drop(_h);
        ctrl.try_into_simulation().unwrap_or_else(|_| panic!("should succeed after handle dropped"));
    }

    #[test]
    fn policy_manual_needs_no_background_loop() {
        assert!(!TickPolicy::Manual.needs_background_loop());
    }

    #[test]
    fn policy_clock_driven_needs_background_loop() {
        assert!(TickPolicy::ClockDriven { interval_ms: 100 }.needs_background_loop());
    }

    #[test]
    fn world_handle_accessible_without_locking_sim() {
        let ctrl = EngineController::new(simple_sim(), TickPolicy::Manual);
        let handle = ctrl.handle();
        // Acquire world for reading while the handle is live — no Mutex needed.
        let world = handle.world_handle();
        let _guard = world.read().unwrap();
    }

    /// `ChangeDriven` loop wakes immediately on ingest and processes queued stimuli.
    ///
    /// `step([])` only advances the batch when `pending_stimuli` is non-empty.
    /// `ingest()` queues to `pending_stimuli` AND fires the notify signal, so the
    /// background loop wakes and drains the queue in its next `step([])`.
    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn change_driven_wakes_on_ingest() {
        use graph_core::Properties;

        let mut ctrl = EngineController::new(simple_sim(), TickPolicy::ChangeDriven { heartbeat_ms: 500 });
        ctrl.start();
        let handle = ctrl.handle();

        let batch_before = handle.current_batch();
        // ingest queues to pending_stimuli AND notifies the background loop.
        handle.ingest("test_node", KIND, SIGNAL, Properties::default());
        // Give the background task time to wake and drain the pending queue.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let batch_after = handle.current_batch();

        ctrl.stop();
        assert!(batch_after > batch_before, "ChangeDriven loop did not process ingest notification");
    }

    /// `ClockDriven` loop fires on a fixed wall-clock interval.
    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn clock_driven_ticks_on_interval() {
        use graph_core::Properties;

        let mut ctrl = EngineController::new(simple_sim(), TickPolicy::ClockDriven { interval_ms: 20 });
        ctrl.start();
        let handle = ctrl.handle();

        let batch_before = handle.current_batch();
        // Queue a stimulus so the clock tick has something to commit.
        handle.ingest("node", KIND, SIGNAL, Properties::default());
        // Wait for at least two interval ticks.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let batch_after = handle.current_batch();

        ctrl.stop();
        assert!(batch_after > batch_before, "clock-driven loop did not advance batch");
    }

    /// After `stop()`, no further batches advance even with ingest queued.
    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn stop_halts_background_loop() {
        use graph_core::Properties;

        let mut ctrl = EngineController::new(simple_sim(), TickPolicy::ClockDriven { interval_ms: 10 });
        ctrl.start();
        let handle = ctrl.handle();

        // Let the loop run briefly to confirm it's alive.
        handle.ingest("node", KIND, SIGNAL, Properties::default());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let batch_mid = handle.current_batch();

        ctrl.stop();
        // Queue another stimulus — the loop is now stopped, so this should not be processed.
        handle.ingest("node2", KIND, SIGNAL, Properties::default());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let batch_after = handle.current_batch();

        // Batch must not have advanced further after stop (the pending stimulus stays unprocessed).
        assert_eq!(batch_mid, batch_after, "loop should not tick after stop()");
    }

    /// `start()` is idempotent: calling it twice does not spawn a second loop.
    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn start_is_idempotent() {
        let mut ctrl = EngineController::new(simple_sim(), TickPolicy::ClockDriven { interval_ms: 100 });
        ctrl.start();
        ctrl.start(); // should be a no-op, not a panic or double-spawn
        ctrl.stop();
    }

    /// `ChangeDriven` heartbeat fires even without an ingest signal.
    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn change_driven_heartbeat_fires_without_ingest() {
        use graph_core::Properties;

        // Short heartbeat so we don't wait long in test.
        let mut ctrl = EngineController::new(simple_sim(), TickPolicy::ChangeDriven { heartbeat_ms: 20 });
        ctrl.start();
        let handle = ctrl.handle();

        // Queue a stimulus first so the heartbeat tick has something to commit.
        handle.ingest("node", KIND, SIGNAL, Properties::default());
        // Wait for heartbeat to fire without any notify signal.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let batch = handle.current_batch();
        ctrl.stop();

        assert!(batch.0 > 0, "heartbeat should have fired and committed a batch");
    }

    /// Drop of `EngineController` aborts the background task.
    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn drop_aborts_background_task() {
        use graph_core::Properties;

        let world_arc = {
            let mut ctrl = EngineController::new(simple_sim(), TickPolicy::ClockDriven { interval_ms: 10 });
            ctrl.start();
            let handle = ctrl.handle();
            handle.ingest("node", KIND, SIGNAL, Properties::default());
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let w = handle.world_handle();
            drop(handle);
            drop(ctrl); // Drop aborts background task.
            w
        };
        // After drop, world is still accessible (Arc keeps it alive) but no loop ticks.
        let _batch = world_arc.read().unwrap().current_batch();
    }
}
