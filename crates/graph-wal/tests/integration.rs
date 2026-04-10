//! End-to-end integration tests for graph-wal.
//!
//! These tests run actual `Simulation` steps, write WAL records after
//! each step, then recover the world from disk and verify it matches
//! the in-memory world.

use graph_engine::Simulation;
use graph_testkit::fixtures::{chain_world, cyclic_pair_world, stimulus};
use graph_wal::{WalConfig, WalSyncWriter, recover, write_checkpoint};
use tempfile::TempDir;

/// Run `steps` ticks of a chain world, writing each tick's batches to the WAL.
/// Returns the final world state + the temp dir (kept alive so files persist).
fn run_chain_with_wal(n: u64, gain: f32, steps: usize) -> (graph_world::World, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = WalConfig::new(dir.path()).sync_writes(true);

    let (world, loci, influences) = chain_world(n, gain);
    let start_batch = world.current_batch();
    let mut sim = Simulation::new(world, loci, influences);
    let mut writer = WalSyncWriter::open(&config, start_batch).unwrap();

    // Initial stimulus.
    let prev = sim.world.current_batch();
    sim.step(vec![stimulus(1.0)]);
    writer.write_tick(&sim.world, prev).unwrap();

    for _ in 1..steps {
        let prev = sim.world.current_batch();
        sim.step(vec![]);
        writer.write_tick(&sim.world, prev).unwrap();
    }

    writer.flush().unwrap();
    (sim.world, dir)
}

#[test]
fn wal_write_and_recover_matches_world() {
    let (original, dir) = run_chain_with_wal(4, 0.8, 10);

    let recovery = recover(dir.path()).unwrap();
    assert!(recovery.warnings.is_empty(), "unexpected warnings: {:?}", recovery.warnings);

    let recovered = recovery.world;

    // Change log length must match.
    let orig_count = original.log().len();
    let rec_count = recovered.log().len();
    assert_eq!(orig_count, rec_count,
        "change log length mismatch: orig={orig_count} recovered={rec_count}");

    // Relationship count must match.
    assert_eq!(
        original.relationships().len(),
        recovered.relationships().len(),
        "relationship count mismatch"
    );

    // World meta counters must match.
    assert_eq!(original.world_meta(), recovered.world_meta());
}

#[test]
fn recover_from_checkpoint_plus_wal() {
    let dir = TempDir::new().unwrap();
    let config = WalConfig::new(dir.path()).sync_writes(true);

    let (world, loci, influences) = chain_world(3, 0.9);
    let start_batch = world.current_batch();
    let mut sim = Simulation::new(world, loci, influences);
    let mut writer = WalSyncWriter::open(&config, start_batch).unwrap();

    // Run 5 steps and write checkpoint at step 5.
    for i in 0..5 {
        let stims = if i == 0 { vec![stimulus(1.0)] } else { vec![] };
        let prev = sim.world.current_batch();
        sim.step(stims);
        writer.write_tick(&sim.world, prev).unwrap();
    }

    // Write a checkpoint mid-run.
    write_checkpoint(dir.path(), &sim.world).unwrap();

    // Run 5 more steps.
    for _ in 0..5 {
        let prev = sim.world.current_batch();
        sim.step(vec![]);
        writer.write_tick(&sim.world, prev).unwrap();
    }
    writer.flush().unwrap();

    let recovery = recover(dir.path()).unwrap();
    let recovered = recovery.world;

    assert_eq!(sim.world.world_meta(), recovered.world_meta());
    assert_eq!(sim.world.relationships().len(), recovered.relationships().len());
}

#[test]
fn recover_from_checkpoint_only_no_wal() {
    let dir = TempDir::new().unwrap();
    let (world, loci, influences) = cyclic_pair_world(0.8);
    let mut sim = Simulation::new(world, loci, influences);

    for _ in 0..8 {
        sim.step(vec![stimulus(1.0)]);
    }

    write_checkpoint(dir.path(), &sim.world).unwrap();

    let recovery = recover(dir.path()).unwrap();
    let recovered = recovery.world;

    assert_eq!(sim.world.world_meta(), recovered.world_meta());
    assert_eq!(sim.world.loci().iter().count(), recovered.loci().iter().count());
    assert_eq!(sim.world.relationships().len(), recovered.relationships().len());
}

#[test]
fn empty_dir_recovers_to_empty_world() {
    let dir = TempDir::new().unwrap();
    let r = recover(dir.path()).unwrap();
    assert_eq!(r.world.loci().iter().count(), 0);
    assert_eq!(r.world.log().len(), 0);
}
