//! E4 determinism harness.
//!
//! Asserts that a simulation run with P>1 partition fn produces **identical
//! World state** to the same run with no partition fn (single-partition mode).
//!
//! This test is the oracle that must stay green throughout E4 implementation.
//! It currently passes trivially because the partition index has no effect on
//! the engine yet. Once parallel Apply is wired in (step 3 of §10) this test
//! will catch any determinism regressions.

use graph_engine::Simulation;
use graph_testkit::fixtures::{ring_world, stimulus};
use std::sync::Arc;

// ── helpers ───────────────────────────────────────────────────────────────────

fn snapshot(sim: &Simulation) -> WorldSnapshot {
    let w = sim.world();
    let mut locus_states: Vec<(u64, Vec<f32>)> = w
        .loci()
        .iter()
        .map(|l| (l.id.0, l.state.as_slice().to_vec()))
        .collect();
    locus_states.sort_by_key(|(id, _)| *id);

    let mut rel_states: Vec<(u64, Vec<f32>)> = w
        .relationships()
        .iter()
        .map(|r| (r.id.0, r.state.as_slice().to_vec()))
        .collect();
    rel_states.sort_by_key(|(id, _)| *id);

    WorldSnapshot {
        locus_states,
        rel_states,
    }
}

#[derive(Debug, PartialEq)]
struct WorldSnapshot {
    locus_states: Vec<(u64, Vec<f32>)>,
    rel_states: Vec<(u64, Vec<f32>)>,
}

// ── determinism tests ─────────────────────────────────────────────────────────

#[test]
fn ring_p4_matches_single_partition_after_20_ticks() {
    const N: u64 = 32;
    const TICKS: usize = 20;
    const P: u64 = 4;

    // --- baseline: no partition fn ---
    let (world_base, loci, influences) = ring_world(N, 0.5);
    let mut sim_base = Simulation::new(world_base, loci, influences);
    for _ in 0..TICKS {
        sim_base.step(vec![stimulus(1.0)]);
    }
    let snap_base = snapshot(&sim_base);

    // --- with partition fn ---
    let (mut world_p4, loci, influences) = ring_world(N, 0.5);
    world_p4.set_partition_fn(Some(Arc::new(move |locus| locus.id.0 * P / N)));
    let mut sim_p4 = Simulation::new(world_p4, loci, influences);
    for _ in 0..TICKS {
        sim_p4.step(vec![stimulus(1.0)]);
    }
    let snap_p4 = snapshot(&sim_p4);

    assert_eq!(
        snap_base, snap_p4,
        "partition fn must not change simulation outcome"
    );
}

#[test]
fn ring_p1_explicit_matches_no_partition() {
    // Sanity: P=1 range fn (all loci → bucket 0) == no partition fn.
    const N: u64 = 16;
    const TICKS: usize = 10;

    let (world_base, loci, influences) = ring_world(N, 0.5);
    let mut sim_base = Simulation::new(world_base, loci, influences);
    for _ in 0..TICKS {
        sim_base.step(vec![stimulus(1.0)]);
    }

    let (mut world_p1, loci, influences) = ring_world(N, 0.5);
    world_p1.set_partition_fn(Some(Arc::new(|_locus| 0u64)));
    let mut sim_p1 = Simulation::new(world_p1, loci, influences);
    for _ in 0..TICKS {
        sim_p1.step(vec![stimulus(1.0)]);
    }

    assert_eq!(snapshot(&sim_base), snapshot(&sim_p1));
}

#[test]
fn partition_index_tracks_loci_correctly() {
    // Verify the PartitionIndex data model independently of the engine.
    const N: u64 = 20;
    const P: u64 = 4;

    let (mut world, _, _) = ring_world(N, 0.5);
    world.set_partition_fn(Some(Arc::new(move |locus| locus.id.0 * P / N)));

    let idx = world
        .partition_index()
        .expect("partition index should be set");
    assert_eq!(idx.bucket_count(), P as usize);

    // Every locus is assigned to exactly one bucket.
    let total_members: usize = idx.buckets().iter().map(|&b| idx.members_of(b).len()).sum();
    assert_eq!(total_members, N as usize);

    // bucket_of is consistent with members_of.
    for &b in &idx.buckets() {
        for &locus_id in idx.members_of(b) {
            assert_eq!(idx.bucket_of(locus_id), Some(b));
        }
    }
}
