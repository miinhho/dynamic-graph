//! Engine handle trait and in-process implementation.
//!
//! [`EngineHandle`] is the shared client interface for the simulation engine.
//! It exposes the subset of [`Simulation`] methods that can be called through
//! a shared reference — all return owned values so the trait works both for
//! in-process use and over a future remote channel.
//!
//! ## Using a handle
//!
//! Obtain a handle from [`EngineController::handle()`]:
//!
//! ```ignore
//! let controller = EngineController::new(sim, TickPolicy::Manual);
//! let handle = controller.handle();
//!
//! // Drive the loop manually:
//! handle.step(vec![]);
//!
//! // Clone and share across threads:
//! let handle2 = handle.clone();
//! std::thread::spawn(move || {
//!     println!("{:?}", handle2.current_batch());
//! });
//! ```
//!
//! For zero-copy world access (in-process only), call [`LocalHandle::world_handle()`].

use std::sync::{Arc, Mutex, RwLock};

use graph_core::{
    BatchId, InfluenceKindId, Locus, LocusId, LocusKindId, Properties, ProposedChange,
    Relationship, RelationshipId, WorldEvent,
};
use graph_world::World;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::simulation::Simulation;
use crate::simulation::{IngestError, StepObservation};

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Shared client interface to the simulation engine.
///
/// All methods take `&self` and return owned values. This allows the same trait
/// to be implemented by both an in-process `LocalHandle` (backed by a `Mutex<Simulation>`)
/// and a future remote handle (backed by a channel to a worker thread or process).
///
/// Methods that require a closure parameter (e.g. `step_until`) or that return
/// borrowed world state (guards) are intentionally excluded from this trait — use
/// [`LocalHandle`] directly when you need those capabilities.
pub trait EngineHandle: Send + Sync + 'static {
    // ── Tick control ──────────────────────────────────────────────────────

    /// Run one step, injecting `stimuli`. Returns the observation for this step.
    fn step(&self, stimuli: Vec<ProposedChange>) -> StepObservation;

    /// Run `n` steps, injecting `stimuli` on the first step only.
    fn step_n(&self, n: usize, stimuli: Vec<ProposedChange>) -> Vec<StepObservation>;

    /// Run one step draining all pending ingested stimuli (no extra stimuli).
    fn flush_ingested(&self) -> StepObservation;

    // ── Ingest ────────────────────────────────────────────────────────────

    /// Ingest a named entity with domain properties, queuing a stimulus.
    ///
    /// See [`Simulation::ingest`] for full semantics.
    fn ingest(
        &self,
        name: &str,
        kind: LocusKindId,
        influence: InfluenceKindId,
        properties: Properties,
    ) -> LocusId;

    /// Ingest a batch of co-occurring entities.
    ///
    /// See [`Simulation::ingest_batch`] for full semantics.
    fn ingest_batch(
        &self,
        entries: Vec<(String, LocusKindId, Properties)>,
        influence: InfluenceKindId,
    ) -> Vec<LocusId>;

    /// String-based ingest: looks up kind name, uses registered default influence.
    ///
    /// Returns `Err` when the kind name is not registered or no default influence is set.
    fn try_ingest_named(
        &self,
        name: &str,
        kind_name: &str,
        properties: Properties,
    ) -> Result<LocusId, IngestError>;

    /// String-based ingest with explicit influence name.
    ///
    /// Returns `Err` when either name is not registered.
    fn try_ingest_named_with(
        &self,
        name: &str,
        kind_name: &str,
        influence_name: &str,
        properties: Properties,
    ) -> Result<LocusId, IngestError>;

    // ── World queries — owned results only ────────────────────────────────

    /// Current batch id (the id that will be assigned to the *next* committed batch).
    fn current_batch(&self) -> BatchId;

    /// Return the locus with the given id (cloned), or `None`.
    fn locus(&self, id: LocusId) -> Option<Locus>;

    /// Return the relationship with the given id (cloned), or `None`.
    fn relationship(&self, id: RelationshipId) -> Option<Relationship>;

    /// Find a relationship between two loci (cloned), or `None`.
    fn relationship_between(&self, a: LocusId, b: LocusId) -> Option<Relationship>;

    /// All loci of a specific kind (cloned).
    fn loci_of_kind(&self, kind: LocusKindId) -> Vec<Locus>;

    // ── Kind registry ─────────────────────────────────────────────────────

    /// Resolve a locus kind name to its `LocusKindId`, or `None`.
    fn locus_kind_id(&self, name: &str) -> Option<LocusKindId>;

    /// Resolve an influence kind name to its `InfluenceKindId`, or `None`.
    fn influence_kind_id(&self, name: &str) -> Option<InfluenceKindId>;

    // ── Emergence / cohere ────────────────────────────────────────────────

    /// Recognize entities using the given perspective. Returns emitted world events.
    fn recognize_entities(&self, perspective: &dyn EmergencePerspective) -> Vec<WorldEvent>;

    /// Extract cohere clusters using the given perspective.
    fn extract_cohere(&self, perspective: &dyn CoherePerspective);
}

// ── LocalHandle ───────────────────────────────────────────────────────────────

/// In-process implementation of [`EngineHandle`] backed by a `Mutex<Simulation>`.
///
/// `LocalHandle` is cheap to clone — clones share the same underlying `Simulation`.
/// For zero-copy world access between steps, use [`LocalHandle::world_handle()`]
/// to obtain the inner `Arc<RwLock<World>>`.
///
/// Obtain a `LocalHandle` from [`EngineController::handle()`].
#[derive(Clone)]
pub struct LocalHandle {
    sim: Arc<Mutex<Simulation>>,
    /// Cached Arc clone from `Simulation::world_handle()` — avoids locking `sim`
    /// on every `world_handle()` call.
    world: Arc<RwLock<World>>,
    /// Wake signal sent to the background tick loop on each ingest call.
    /// `None` when the controller was constructed without a tokio policy or when
    /// `LocalHandle::new` is called directly (outside of `EngineController`).
    #[cfg(feature = "tokio")]
    notify: Option<Arc<tokio::sync::Notify>>,
    /// Broadcast sender for world events emitted by ticks and entity operations.
    /// Shared with `EngineController` and all sibling handles.
    #[cfg(feature = "tokio")]
    event_tx: Arc<tokio::sync::broadcast::Sender<WorldEvent>>,
}

impl LocalHandle {
    // When the `tokio` feature is active, `EngineController::handle()` uses
    // `new_with_notify` exclusively; `new` is still kept for direct construction
    // (e.g. tests that build a LocalHandle without a controller).
    #[cfg_attr(feature = "tokio", allow(dead_code))]
    pub(crate) fn new(sim: Arc<Mutex<Simulation>>) -> Self {
        let world = sim.lock().unwrap().world_handle();
        #[cfg(feature = "tokio")]
        let (event_tx, _) = tokio::sync::broadcast::channel(1);
        Self {
            sim,
            world,
            #[cfg(feature = "tokio")]
            notify: None,
            #[cfg(feature = "tokio")]
            event_tx: Arc::new(event_tx),
        }
    }

    /// Construct a handle wired to a background-loop wake signal and event channel.
    ///
    /// Called by [`EngineController::handle()`]. Each ingest call wakes the
    /// background task; each step/recognize call broadcasts emitted `WorldEvent`s
    /// to all subscribers obtained via [`subscribe_world_events`].
    ///
    /// [`subscribe_world_events`]: LocalHandle::subscribe_world_events
    #[cfg(feature = "tokio")]
    pub(crate) fn new_with_notify(
        sim: Arc<Mutex<Simulation>>,
        notify: Arc<tokio::sync::Notify>,
        event_tx: Arc<tokio::sync::broadcast::Sender<WorldEvent>>,
    ) -> Self {
        let world = sim.lock().unwrap().world_handle();
        Self {
            sim,
            world,
            notify: Some(notify),
            event_tx,
        }
    }

    /// Signal the background loop that new stimuli are queued.
    ///
    /// No-op when no notify is attached (Manual policy or direct construction).
    #[cfg(feature = "tokio")]
    #[inline]
    fn wake(&self) {
        if let Some(n) = &self.notify {
            n.notify_one();
        }
    }

    /// Clone the underlying world handle for background query threads.
    ///
    /// Callers can call `.read()` on the returned `Arc` between `step()` calls.
    /// The write lock inside `Simulation` is held for the duration of each step,
    /// so reads during a step will block until the step completes.
    ///
    /// ```ignore
    /// let world = handle.world_handle();
    /// std::thread::spawn(move || {
    ///     let w = world.read().unwrap();
    ///     // query w ...
    /// });
    /// ```
    #[inline]
    pub fn world_handle(&self) -> Arc<RwLock<World>> {
        Arc::clone(&self.world)
    }

    /// Run steps until `pred(observation, world)` returns `true` or
    /// `max_steps` is reached. Stimuli are injected on the first step only.
    ///
    /// Returns `(observations, converged)`.
    ///
    /// This method is only available on `LocalHandle` because the closure
    /// cannot be expressed in the trait without `dyn Fn` boxing — for
    /// single-machine use the Mutex overhead is minimal; for remote use,
    /// prefer polling from the client side.
    pub fn step_until(
        &self,
        pred: impl FnMut(&StepObservation, &World) -> bool,
        max_steps: usize,
        stimuli: Vec<ProposedChange>,
    ) -> (Vec<StepObservation>, bool) {
        self.sim
            .lock()
            .unwrap()
            .step_until(pred, max_steps, stimuli)
    }

    /// Read a named extra slot value from a relationship's current state.
    pub fn rel_slot_value(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        slot_name: &str,
    ) -> Option<f32> {
        self.sim
            .lock()
            .unwrap()
            .rel_slot_value(rel_id, kind, slot_name)
    }

    /// Trim the change log, dropping batches older than `retention_batches`.
    pub fn trim_change_log(&self, retention_batches: u64) -> usize {
        self.sim.lock().unwrap().trim_change_log(retention_batches)
    }

    /// Subscribe to the world event stream for this handle's simulation.
    ///
    /// Returns a `broadcast::Receiver<WorldEvent>` that receives every
    /// `WorldEvent` emitted by subsequent `step()`, `recognize_entities()`,
    /// and background-loop ticks. Each subscriber gets an independent copy of
    /// every event — subscribe before the operations you want to observe.
    ///
    /// The receiver is automatically closed (dropped) when it goes out of scope.
    /// Lagging receivers may miss events; the channel capacity is 256. Use
    /// `try_recv` or `recv` in a background task to drain promptly.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let mut rx = handle.subscribe_world_events();
    /// handle.step(stimuli);
    /// while let Ok(event) = rx.try_recv() {
    ///     println!("event: {event:?}");
    /// }
    /// ```
    #[cfg(feature = "tokio")]
    pub fn subscribe_world_events(&self) -> tokio::sync::broadcast::Receiver<WorldEvent> {
        self.event_tx.subscribe()
    }

    /// Helper: broadcast all events in a vec through the event channel.
    #[cfg(feature = "tokio")]
    #[inline]
    fn broadcast_events(&self, events: &[WorldEvent]) {
        for event in events {
            let _ = self.event_tx.send(event.clone());
        }
    }
}

impl EngineHandle for LocalHandle {
    fn step(&self, stimuli: Vec<ProposedChange>) -> StepObservation {
        let obs = self.sim.lock().unwrap().step(stimuli);
        #[cfg(feature = "tokio")]
        self.broadcast_events(&obs.events);
        obs
    }

    fn step_n(&self, n: usize, stimuli: Vec<ProposedChange>) -> Vec<StepObservation> {
        self.sim.lock().unwrap().step_n(n, stimuli)
    }

    fn flush_ingested(&self) -> StepObservation {
        self.sim.lock().unwrap().flush_ingested()
    }

    fn ingest(
        &self,
        name: &str,
        kind: LocusKindId,
        influence: InfluenceKindId,
        properties: Properties,
    ) -> LocusId {
        let id = self
            .sim
            .lock()
            .unwrap()
            .ingest(name, kind, influence, properties);
        #[cfg(feature = "tokio")]
        self.wake();
        id
    }

    fn ingest_batch(
        &self,
        entries: Vec<(String, LocusKindId, Properties)>,
        influence: InfluenceKindId,
    ) -> Vec<LocusId> {
        // Borrow each String as &str to reuse Simulation::ingest_batch.
        let borrowed: Vec<(&str, LocusKindId, Properties)> = entries
            .iter()
            .map(|(n, k, p)| (n.as_str(), *k, p.clone()))
            .collect();
        let ids = self.sim.lock().unwrap().ingest_batch(borrowed, influence);
        #[cfg(feature = "tokio")]
        self.wake();
        ids
    }

    fn try_ingest_named(
        &self,
        name: &str,
        kind_name: &str,
        properties: Properties,
    ) -> Result<LocusId, IngestError> {
        let result = self
            .sim
            .lock()
            .unwrap()
            .try_ingest_named(name, kind_name, properties);
        #[cfg(feature = "tokio")]
        if result.is_ok() {
            self.wake();
        }
        result
    }

    fn try_ingest_named_with(
        &self,
        name: &str,
        kind_name: &str,
        influence_name: &str,
        properties: Properties,
    ) -> Result<LocusId, IngestError> {
        let result = self.sim.lock().unwrap().try_ingest_named_with(
            name,
            kind_name,
            influence_name,
            properties,
        );
        #[cfg(feature = "tokio")]
        if result.is_ok() {
            self.wake();
        }
        result
    }

    fn current_batch(&self) -> BatchId {
        self.sim.lock().unwrap().current_batch()
    }

    fn locus(&self, id: LocusId) -> Option<Locus> {
        self.sim.lock().unwrap().locus(id)
    }

    fn relationship(&self, id: RelationshipId) -> Option<Relationship> {
        self.sim.lock().unwrap().relationship(id)
    }

    fn relationship_between(&self, a: LocusId, b: LocusId) -> Option<Relationship> {
        self.sim.lock().unwrap().relationship_between(a, b)
    }

    fn loci_of_kind(&self, kind: LocusKindId) -> Vec<Locus> {
        self.sim.lock().unwrap().loci_of_kind(kind)
    }

    fn locus_kind_id(&self, name: &str) -> Option<LocusKindId> {
        self.sim.lock().unwrap().locus_kind_id(name)
    }

    fn influence_kind_id(&self, name: &str) -> Option<InfluenceKindId> {
        self.sim.lock().unwrap().influence_kind_id(name)
    }

    fn recognize_entities(&self, perspective: &dyn EmergencePerspective) -> Vec<WorldEvent> {
        let events = self.sim.lock().unwrap().recognize_entities(perspective);
        #[cfg(feature = "tokio")]
        self.broadcast_events(&events);
        events
    }

    fn extract_cohere(&self, perspective: &dyn CoherePerspective) {
        self.sim.lock().unwrap().extract_cohere(perspective)
    }
}
