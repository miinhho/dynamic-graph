//! Cross-crate integration tests for the full simulation stack.
//!
//! These tests exercise the full pipeline from `Simulation::step()` through
//! `World` queries, including relationship emergence, `WorldDiff`, graph
//! traversal, `step_until` convergence, and storage persistence.

use graph_core::LocusId;
use graph_engine::Simulation;
use graph_query::{connected_components, path_between, reachable_from};
use graph_testkit::assertions::{
    assert_bounded_activity, assert_changes_form_dag, assert_settling,
};
use graph_testkit::fixtures::{chain_world, cyclic_pair_world, ring_world, star_world, stimulus};

// ── relationship emergence ────────────────────────────────────────────────────

#[test]
fn chain_step_produces_relationships() {
    let (world, loci, influences) = chain_world(3, 0.9);
    let mut sim = Simulation::new(world, loci, influences);
    let obs = sim.step(vec![stimulus(1.0)]);
    assert!(obs.relationships > 0);
}

#[test]
fn star_step_produces_fan_out_relationships() {
    let (world, loci, influences) = star_world(4, 0.8);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step(vec![stimulus(1.0)]);
    assert_eq!(sim.world().relationships().len(), 4);
}

// ── multi-step and WorldDiff ──────────────────────────────────────────────────

#[test]
fn worlddiff_captures_step_n_changes() {
    let (world, loci, influences) = chain_world(3, 0.9);
    let mut sim = Simulation::new(world, loci, influences);
    let before = sim.world().current_batch();
    sim.step_n(5, vec![stimulus(1.0)]);
    let diff = sim.world().diff_since(before);
    assert!(!diff.change_ids.is_empty());
    assert!(!diff.relationships_created.is_empty());
}

#[test]
fn worlddiff_distinguishes_created_vs_updated() {
    let (world, loci, influences) = cyclic_pair_world(0.5);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step(vec![stimulus(1.0)]);
    let after_first = sim.world().current_batch();
    sim.step(vec![stimulus(0.5)]);
    let after_second = sim.world().current_batch();
    let diff = sim.world().diff_between(after_first, after_second);
    assert!(diff.relationships_created.is_empty());
    assert!(!diff.relationships_updated.is_empty());
}

#[test]
fn worlddiff_empty_for_quiescent_range() {
    let (world, loci, influences) = chain_world(2, 0.5);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step_n(20, vec![stimulus(1.0)]);
    let before = sim.world().current_batch();
    sim.step(vec![]);
    let diff = sim.world().diff_since(before);
    assert!(diff.change_ids.is_empty());
}

// ── graph traversal after emergence ──────────────────────────────────────────

#[test]
fn path_between_found_after_chain_emergence() {
    let (world, loci, influences) = chain_world(3, 0.9);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step(vec![stimulus(1.0)]);
    let path = path_between(&*sim.world(), LocusId(0), LocusId(2));
    assert!(path.is_some());
    assert!(path.unwrap().len() >= 2);
}

#[test]
fn reachable_from_covers_all_chain_loci() {
    let (world, loci, influences) = chain_world(4, 0.9);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step(vec![stimulus(1.0)]);
    let reachable = reachable_from(&*sim.world(), LocusId(0), 10);
    for i in 1..4u64 {
        assert!(reachable.contains(&LocusId(i)));
    }
}

#[test]
fn connected_components_ring_is_one() {
    let (world, loci, influences) = ring_world(4, 0.8);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step(vec![stimulus(1.0)]);
    let components = connected_components(&*sim.world());
    assert_eq!(components.len(), 1);
}

// ── step_until convergence ────────────────────────────────────────────────────

#[test]
fn step_until_fires_predicate_before_max() {
    let (world, loci, influences) = cyclic_pair_world(0.3);
    let mut sim = Simulation::new(world, loci, influences);
    let (observations, converged) = sim.step_until(
        |obs, _world| {
            use graph_engine::DynamicsRegime;
            matches!(
                obs.regime,
                DynamicsRegime::Quiescent | DynamicsRegime::Settling
            )
        },
        100,
        vec![stimulus(1.0)],
    );
    assert!(converged);
    assert!(observations.len() < 100);
}

#[test]
fn step_until_stimuli_applied_only_on_first_step() {
    let (world, loci, influences) = chain_world(3, 0.9);
    let mut sim = Simulation::new(world, loci, influences);
    let (_, _) = sim.step_until(|_, _| false, 3, vec![stimulus(1.0)]);
    let rel_count_before = sim.world().relationships().len();
    let (obs2, _) = sim.step_until(|_, _| false, 3, vec![]);
    assert_eq!(rel_count_before, obs2.last().unwrap().relationships);
}

#[test]
fn step_until_returns_false_when_max_steps_reached() {
    use graph_testkit::fixtures::accumulator_world;
    let (world, loci, influences) = accumulator_world(2.0);
    let mut sim = Simulation::new(world, loci, influences);
    let (obs, converged) = sim.step_until(
        |o, _| {
            use graph_engine::DynamicsRegime;
            matches!(o.regime, DynamicsRegime::Quiescent)
        },
        5,
        vec![stimulus(1.0)],
    );
    assert!(!converged);
    assert_eq!(obs.len(), 5);
}

// ── invariant assertions ──────────────────────────────────────────────────────

#[test]
fn chain_activity_stays_bounded() {
    let (world, loci, influences) = chain_world(5, 0.9);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step_n(30, vec![stimulus(1.0)]);
    assert_bounded_activity(&*sim.world(), 2.0);
}

#[test]
fn change_log_forms_dag_after_ring_simulation() {
    let (world, loci, influences) = ring_world(4, 0.7);
    let mut sim = Simulation::new(world, loci, influences);
    sim.step_n(10, vec![stimulus(1.0)]);
    assert_changes_form_dag(&*sim.world());
}

#[test]
fn chain_settles_with_gain_below_one() {
    let (world, loci, influences) = chain_world(4, 0.5);
    let mut sim = Simulation::new(world, loci, influences);
    let obs = sim.step_n(30, vec![stimulus(1.0)]);
    for o in &obs {
        assert_settling(&o.tick);
    }
}

// ── storage persistence ─────────────────────────────────────────────────────

#[cfg(feature = "storage")]
mod storage {
    use graph_core::BatchId;
    use graph_engine::{Simulation, SimulationConfig};
    use graph_testkit::fixtures::{chain_world, stimulus};
    use tempfile::NamedTempFile;

    fn storage_config(f: &NamedTempFile) -> SimulationConfig {
        SimulationConfig {
            storage_path: Some(f.path().to_path_buf()),
            ..Default::default()
        }
    }

    #[test]
    fn storage_recovery_restores_relationship_count() {
        let f = NamedTempFile::new().unwrap();
        let rel_count;
        {
            let (world, loci, influences) = chain_world(3, 0.9);
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            sim.step_n(5, vec![stimulus(1.0)]);
            rel_count = sim.world().relationships().len();
        }

        let (_, loci2, influences2) = chain_world(3, 0.9);
        let sim2 =
            Simulation::from_storage(f.path(), loci2, influences2, SimulationConfig::default())
                .unwrap();
        assert_eq!(rel_count, sim2.world.relationships().len());
    }

    #[test]
    fn storage_recovery_restores_current_batch() {
        let f = NamedTempFile::new().unwrap();
        let final_batch: BatchId;
        {
            let (world, loci, influences) = chain_world(2, 0.9);
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            sim.step_n(10, vec![stimulus(1.0)]);
            final_batch = sim.world().current_batch();
        }

        let (_, loci2, influences2) = chain_world(2, 0.9);
        let sim2 =
            Simulation::from_storage(f.path(), loci2, influences2, SimulationConfig::default())
                .unwrap();
        assert_eq!(final_batch, sim2.world.current_batch());
    }
}
