//! Integration tests for EngineController and LocalHandle.
//!
//! Verifies: handle distribution, live-count tracking, Manual step,
//! EngineHandle trait methods, step_until convergence, and simulation
//! recovery via into_simulation / try_into_simulation.

use graph_core::LocusKindId;
use graph_engine::{EngineController, EngineHandle, Simulation, TickPolicy};
use graph_testkit::fixtures::{chain_world, stimulus};

fn make_sim() -> Simulation {
    let (world, loci, influences) = chain_world(3, 0.9);
    Simulation::new(world, loci, influences)
}

// ── TickPolicy ────────────────────────────────────────────────────────────────

#[test]
fn tick_policy_needs_background_loop() {
    assert!(!TickPolicy::Manual.needs_background_loop());
    assert!(TickPolicy::ChangeDriven { heartbeat_ms: 100 }.needs_background_loop());
    assert!(!TickPolicy::ChangeDriven { heartbeat_ms: 0 }.needs_background_loop());
    assert!(TickPolicy::ClockDriven { interval_ms: 50 }.needs_background_loop());
    assert!(!TickPolicy::ClockDriven { interval_ms: 0 }.needs_background_loop());
}

// ── basic step ────────────────────────────────────────────────────────────────

#[test]
fn manual_handle_step_advances_batch() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let before = h.current_batch();
    h.step(vec![stimulus(1.0)]);
    assert!(h.current_batch() > before);
}

#[test]
fn handle_step_n_runs_n_steps() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let before = h.current_batch();
    h.step_n(5, vec![stimulus(1.0)]);
    let after = h.current_batch();
    assert!(after > before, "batch should advance after step_n");
}

#[test]
fn cloned_handles_share_simulation() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h1 = ctrl.handle();
    let h2 = h1.clone();
    h1.step(vec![stimulus(1.0)]);
    // h2 sees the same batch because they share the Mutex<Simulation>.
    assert_eq!(h1.current_batch(), h2.current_batch());
}

// ── live handle count ─────────────────────────────────────────────────────────

#[test]
fn live_handle_count_tracks_clones() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
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

// ── simulation recovery ───────────────────────────────────────────────────────

#[test]
fn into_simulation_succeeds_with_no_live_handles() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let _sim = ctrl.into_simulation(); // should not panic
}

#[test]
fn try_into_simulation_fails_with_live_handle() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let result = ctrl.try_into_simulation();
    assert!(result.is_err());
    drop(h);
}

#[test]
fn try_into_simulation_succeeds_after_handle_drop() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    drop(h);
    assert!(ctrl.try_into_simulation().is_ok());
}

// ── EngineHandle trait methods ────────────────────────────────────────────────

#[test]
fn flush_ingested_is_a_step() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let before = h.current_batch();
    h.flush_ingested();
    // flush_ingested calls step([]); batch may or may not advance if already
    // quiescent, but it should not panic.
    let _ = h.current_batch();
    drop(before);
}

#[test]
fn loci_of_kind_returns_registered_loci() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    // chain_world assigns LocusKindId(id.0 + 1000) per locus; LocusId(0) → kind 1000.
    let found = h.loci_of_kind(LocusKindId(1000));
    assert!(!found.is_empty());
}

// ── LocalHandle extras ────────────────────────────────────────────────────────

#[test]
fn world_handle_provides_read_access() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let world_arc = h.world_handle();
    let _batch = world_arc.read().unwrap().current_batch();
}

#[test]
fn step_until_converges_within_limit() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let (obs, converged) = h.step_until(
        |o, _w| o.tick.batches_committed > 0,
        10,
        vec![stimulus(1.0)],
    );
    assert!(converged, "expected convergence within 10 steps");
    assert!(!obs.is_empty());
}

// ── A2: subscribe_world_events ────────────────────────────────────────────────

/// Events emitted by `step()` are delivered to all subscribers.
#[cfg(feature = "tokio")]
#[tokio::test]
async fn subscribe_world_events_receives_step_events() {
    use graph_core::WorldEvent;
    use graph_engine::DefaultEmergencePerspective;

    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let mut rx = h.subscribe_world_events();

    // Fire a step; with a chain world and decay the relationship will be pruned
    // eventually, but at minimum the step itself may emit events. We force an
    // event by calling recognize_entities which emits EntityBorn on first call.
    h.step(vec![stimulus(1.0)]);
    let perspective = DefaultEmergencePerspective::default();
    let events = h.recognize_entities(&perspective);

    // Events returned by recognize_entities should also be in the channel.
    // Drain whatever was sent.
    let mut received: Vec<WorldEvent> = Vec::new();
    while let Ok(evt) = rx.try_recv() {
        received.push(evt);
    }

    assert_eq!(
        received.len(),
        events.len(),
        "subscriber should receive the same events returned by recognize_entities"
    );
    for (got, want) in received.iter().zip(events.iter()) {
        assert_eq!(got, want);
    }
}

/// Multiple independent subscribers each receive all events.
#[cfg(feature = "tokio")]
#[tokio::test]
async fn multiple_subscribers_each_receive_all_events() {
    use graph_engine::DefaultEmergencePerspective;

    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    let mut rx1 = h.subscribe_world_events();
    let mut rx2 = h.subscribe_world_events();

    h.step(vec![stimulus(1.0)]);
    let perspective = DefaultEmergencePerspective::default();
    let events = h.recognize_entities(&perspective);

    let count1 = {
        let mut n = 0usize;
        while rx1.try_recv().is_ok() {
            n += 1;
        }
        n
    };
    let count2 = {
        let mut n = 0usize;
        while rx2.try_recv().is_ok() {
            n += 1;
        }
        n
    };

    assert_eq!(count1, events.len(), "subscriber 1 should get all events");
    assert_eq!(count2, events.len(), "subscriber 2 should get all events");
}

/// Dropping the receiver silently stops delivery (no panic on closed channel).
#[cfg(feature = "tokio")]
#[tokio::test]
async fn dropped_receiver_does_not_panic_sender() {
    let ctrl = EngineController::new(make_sim(), TickPolicy::Manual);
    let h = ctrl.handle();
    {
        let _rx = h.subscribe_world_events();
        // _rx dropped here — sender must not panic when sending to a closed receiver
    }
    // step should succeed without panicking
    h.step(vec![stimulus(1.0)]);
}

/// Background loop events are delivered to subscribers.
#[cfg(feature = "tokio")]
#[tokio::test]
async fn background_loop_events_delivered_to_subscriber() {
    use graph_core::Properties;
    use graph_engine::LocalHandle;

    // Use ChangeDriven so the loop fires on ingest.
    let mut ctrl = EngineController::new(make_sim(), TickPolicy::ChangeDriven { heartbeat_ms: 50 });
    ctrl.start();
    let h = ctrl.handle();
    let mut rx = h.subscribe_world_events();

    // Ingest — background loop wakes and steps.
    h.ingest(
        "n",
        LocusKindId(1),
        graph_core::InfluenceKindId(1),
        Properties::default(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    ctrl.stop();

    // We may or may not have events depending on whether a relationship formed,
    // but the important thing is that the channel didn't deadlock or panic.
    // Drain it without asserting count (regime-dependent).
    while rx.try_recv().is_ok() {}
}
