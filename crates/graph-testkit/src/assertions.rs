//! Assertion helpers for engine tests.
//!
//! Each function panics with a descriptive message if the named
//! invariant does not hold. All are free functions so they compose
//! easily inside `#[test]` bodies without a test-harness trait.

use std::collections::{HashMap, HashSet};

use graph_core::ChangeId;
use graph_engine::TickResult;
use graph_world::World;

/// Panics if any relationship's activity slot exceeds `max`.
///
/// Call after a `tick` to verify that the system has not gone super-critical.
pub fn assert_bounded_activity(world: &World, max: f32) {
    for rel in world.relationships().iter() {
        let activity = rel
            .state
            .as_slice()
            .first()
            .copied()
            .unwrap_or(0.0);
        assert!(
            activity <= max,
            "relationship {:?} activity {activity:.4} exceeds bound {max:.4}",
            rel.id
        );
    }
}

/// Panics if the change log's predecessor graph contains a cycle.
///
/// Uses iterative DFS with a three-colour marking scheme (white / grey /
/// black). Because the engine guarantees topological commit order the graph
/// should always be a DAG; this assertion is the runtime check.
pub fn assert_changes_form_dag(world: &World) {
    // Build adjacency: change → its predecessors (all should be in the log).
    let mut pred_map: HashMap<ChangeId, Vec<ChangeId>> = HashMap::new();
    for change in world.log().iter() {
        pred_map.insert(change.id, change.predecessors.clone());
    }

    // Iterative DFS cycle detection (white=0, grey=1, black=2).
    let mut colour: HashMap<ChangeId, u8> = HashMap::new();

    for &start in pred_map.keys() {
        if colour.get(&start).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut stack: Vec<(ChangeId, usize)> = vec![(start, 0)];
        colour.insert(start, 1); // grey

        while let Some((node, idx)) = stack.last_mut() {
            let node = *node;
            let preds = pred_map.get(&node).map(|v| v.as_slice()).unwrap_or(&[]);
            if *idx < preds.len() {
                let neighbour = preds[*idx];
                *idx += 1;
                match colour.get(&neighbour).copied().unwrap_or(0) {
                    1 => panic!(
                        "cycle detected in change DAG: {:?} → {:?}",
                        node, neighbour
                    ),
                    0 => {
                        colour.insert(neighbour, 1);
                        stack.push((neighbour, 0));
                    }
                    _ => {} // black — already fully explored
                }
            } else {
                colour.insert(node, 2); // black
                stack.pop();
            }
        }
    }
}

/// Panics if the world contains no active entity whose member set has at
/// least `min_members` loci.
///
/// Intended for use after `Engine::recognize_entities`.
pub fn assert_entity_active(world: &World, min_members: usize) {
    let found = world
        .entities()
        .active()
        .any(|e| e.current.members.len() >= min_members);
    assert!(
        found,
        "expected an active entity with >= {min_members} members; \
         active entities: {:?}",
        world
            .entities()
            .active()
            .map(|e| (e.id, e.current.members.len()))
            .collect::<Vec<_>>()
    );
}

/// Panics if `result.hit_batch_cap` is true.
///
/// A tick that exhausted the batch cap did not converge — either the
/// topology is incorrectly wired, the gain is too high, or the cap is
/// too low for the test scenario.
pub fn assert_settling(result: &TickResult) {
    assert!(
        !result.hit_batch_cap,
        "tick hit batch cap (max_batches_per_tick reached) — \
         system did not quiesce; check topology and gain"
    );
}

/// Panics if any two changes in the log share the same `ChangeId`.
///
/// The engine mints ids monotonically; duplicates indicate a bug in the
/// id-minting path.
pub fn assert_unique_change_ids(world: &World) {
    let mut seen: HashSet<ChangeId> = HashSet::new();
    for change in world.log().iter() {
        assert!(
            seen.insert(change.id),
            "duplicate ChangeId {:?} found in change log",
            change.id
        );
    }
}

/// Panics if the relationship count is not exactly `expected`.
///
/// Handy for confirming that exactly the right number of causal edges
/// emerged after a controlled stimulus sequence.
pub fn assert_relationship_count(world: &World, expected: usize) {
    let actual = world.relationships().iter().count();
    assert_eq!(
        actual, expected,
        "expected {expected} relationships, found {actual}"
    );
}

/// Panics if the change log contains any change in a batch older than
/// `retain_from_batch`.
///
/// Use after `Engine::trim_change_log` to verify that the trim ran
/// correctly.
pub fn assert_log_bounded(world: &World, retain_from_batch: u64) {
    for change in world.log().iter() {
        assert!(
            change.batch.0 >= retain_from_batch,
            "change {:?} is in batch {} which is older than retention boundary {}",
            change.id,
            change.batch.0,
            retain_from_batch
        );
    }
}
