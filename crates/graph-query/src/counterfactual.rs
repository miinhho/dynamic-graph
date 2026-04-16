//! Counterfactual relationship queries.
//!
//! Answers the question: *"which relationships would not exist if stimulus X
//! had never happened?"*
//!
//! The engine uses predecessor links to record causality: every auto-emerged
//! relationship touch includes the `ChangeId`s that triggered it. By walking
//! **forward** from a set of root stimuli (using [`causal_descendants`]) we
//! can find all changes — and thus all relationship creations — that descend
//! from those stimuli.
//!
//! ## Limitation: multi-causal paths
//!
//! A relationship's creation change may have multiple predecessor paths. If
//! relationship R was caused by both stimulus A and stimulus B, removing A
//! alone would not eliminate R — B would still create it. This module does
//! **not** model multi-causal paths. The functions here return the
//! *maximally pessimistic* set: relationships that have *at least one*
//! causal path back to the given stimuli. Callers interested in
//! *exclusively* caused relationships must intersect the result against
//! relationships whose creation changes have *no* other causal predecessors.
//!
//! ## Example
//!
//! ```ignore
//! // Stimulate two sensory neurons and observe which relationships were caused.
//! let root_changes = world.log().batch(batch_id).iter().map(|c| c.id).collect::<Vec<_>>();
//! let caused = graph_query::relationships_caused_by(&world, &root_changes);
//! let absent_without = graph_query::relationships_absent_without(&world, &root_changes);
//! println!("{} relationships would vanish", absent_without.len());
//! ```

use graph_core::{ChangeSubject, ChangeId, RelationshipId};
use graph_world::World;
use rustc_hash::FxHashSet;

use crate::causality::causal_descendants;

// ─── Core query ───────────────────────────────────────────────────────────────

/// Return the set of relationships that have **at least one** creation or touch
/// change that is a causal descendant of any change in `root_changes`.
///
/// This covers two categories:
///
/// 1. **Explicit relationship changes** (`ChangeSubject::Relationship`): e.g.
///    subscription-observer changes committed to the log.
/// 2. **Auto-emerged relationships** whose `lineage.created_by` points to a
///    locus change that is causally downstream of the stimuli. Auto-emergence
///    does *not* write a `ChangeSubject::Relationship` log entry; the only
///    evidence of causality is the `created_by` backlink.
///
/// See the [module-level docs](self) for the multi-causal-path limitation.
///
/// **Complexity**: O(N) to build the forward adjacency index over the full
/// change log, plus O(D) BFS per root change where D is the number of
/// descendants.
pub fn relationships_caused_by(world: &World, root_changes: &[ChangeId]) -> FxHashSet<RelationshipId> {
    let all_descendants = collect_descendants(world, root_changes);

    let mut result: FxHashSet<RelationshipId> = FxHashSet::default();

    // Category 1: explicit ChangeSubject::Relationship entries in the log.
    for &cid in &all_descendants {
        if let Some(change) = world.log().get(cid) {
            if let ChangeSubject::Relationship(rel_id) = change.subject {
                result.insert(rel_id);
            }
        }
    }

    // Category 2: auto-emerged relationships whose created_by locus change is
    // in the causal descendants.  The engine doesn't write a Relationship-subject
    // change for auto-emergence; the only causality evidence is the backlink.
    for rel in world.relationships().iter() {
        if let Some(creation_change) = rel.lineage.created_by {
            if all_descendants.contains(&creation_change) {
                result.insert(rel.id);
            }
        }
    }

    result
}

/// Return relationships that would be **absent from the world** if none of
/// `root_changes` had ever fired.
///
/// A relationship is "absent without" the stimuli when its *creation* change
/// (`lineage.created_by`) is causally downstream of the given root changes.
/// This means the relationship was brought into existence by the causal cascade
/// triggered by those stimuli — it would not exist if they hadn't fired.
///
/// Relationships that received *activity touches* from the cascade but were
/// created before (or independently of) the stimuli are excluded — they would
/// still exist, just with different activity levels.
///
/// See the [module-level docs](self) for the multi-causal-path limitation.
pub fn relationships_absent_without(world: &World, root_changes: &[ChangeId]) -> Vec<RelationshipId> {
    let all_descendants = collect_descendants(world, root_changes);

    world
        .relationships()
        .iter()
        .filter(|rel| {
            match rel.lineage.created_by {
                Some(creation_change) => all_descendants.contains(&creation_change),
                // No creation change recorded — pre-existing or externally
                // inserted relationship. Conservatively exclude.
                None => false,
            }
        })
        .map(|rel| rel.id)
        .collect()
}

// ─── Chained query builder ────────────────────────────────────────────────────

/// Fluent builder for counterfactual queries.
///
/// Created by [`counterfactual`]. Chain methods to narrow the analysis,
/// then call a terminal to retrieve results.
///
/// ## Example
///
/// ```ignore
/// let absent = graph_query::counterfactual(&world)
///     .stimuli_from_batch(batch_id)
///     .relationships_absent_without();
/// ```
pub struct CounterfactualQuery<'w> {
    world: &'w World,
    roots: Vec<ChangeId>,
}

impl<'w> CounterfactualQuery<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self { world, roots: Vec::new() }
    }

    /// Add specific change IDs as the stimulus roots.
    pub fn with_stimuli(mut self, changes: &[ChangeId]) -> Self {
        self.roots.extend_from_slice(changes);
        self
    }

    /// Add all changes committed in `batch` as stimulus roots.
    pub fn stimuli_from_batch(mut self, batch: graph_core::BatchId) -> Self {
        let ids: Vec<ChangeId> = world_batch_changes(self.world, batch);
        self.roots.extend(ids);
        self
    }

    /// Terminal: all relationships that have at least one causal path back
    /// to the registered stimuli. See [`relationships_caused_by`].
    pub fn relationships_caused(self) -> FxHashSet<RelationshipId> {
        relationships_caused_by(self.world, &self.roots)
    }

    /// Terminal: relationships that would not exist without the registered
    /// stimuli. See [`relationships_absent_without`].
    pub fn relationships_absent_without(self) -> Vec<RelationshipId> {
        relationships_absent_without(self.world, &self.roots)
    }
}

/// Start a counterfactual query over `world`.
///
/// ```ignore
/// let q = graph_query::counterfactual(&world)
///     .stimuli_from_batch(batch_id)
///     .relationships_absent_without();
/// ```
pub fn counterfactual(world: &World) -> CounterfactualQuery<'_> {
    CounterfactualQuery::new(world)
}

// ─── Helper ───────────────────────────────────────────────────────────────────

/// Build the union of all causal descendants of `root_changes`, including the
/// roots themselves.
fn collect_descendants(world: &World, root_changes: &[ChangeId]) -> FxHashSet<ChangeId> {
    let mut all: FxHashSet<ChangeId> = FxHashSet::default();
    for &root in root_changes {
        all.insert(root);
        for desc in causal_descendants(world, root) {
            all.insert(desc);
        }
    }
    all
}

fn world_batch_changes(world: &World, batch: graph_core::BatchId) -> Vec<ChangeId> {
    world
        .log()
        .batch(batch)
        .map(|c| c.id)
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId,
        Locus, LocusId, LocusKindId, Relationship, RelationshipId, RelationshipLineage,
        StateVector,
    };
    use graph_world::World;

    fn make_world_with_linear_causal_chain() -> (World, RelationshipId, ChangeId) {
        // Creates a minimal world where:
        //   change 0 (locus) → change 1 (relationship created_by=0) → change 2 (locus)
        // The relationship is causally downstream of change 0.
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(0), LocusKindId(1), StateVector::from_slice(&[1.0])));
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::from_slice(&[0.0])));

        let rel_id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[0.5, 0.0]),
            lineage: RelationshipLineage {
                created_by: Some(ChangeId(1)),
                last_touched_by: Some(ChangeId(1)),
                change_count: 1,
                kinds_observed: smallvec::SmallVec::new(),
            },
            created_batch: BatchId(1),
            last_decayed_batch: 0,
            metadata: None,
        });

        // Commit three changes: root (0), relationship touch (1), downstream locus (2).
        let root_change = Change {
            id: ChangeId(0),
            subject: ChangeSubject::Locus(LocusId(0)),
            kind: InfluenceKindId(0),
            predecessors: vec![],
            before: StateVector::from_slice(&[0.0]),
            after: StateVector::from_slice(&[1.0]),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };
        let rel_change = Change {
            id: ChangeId(1),
            subject: ChangeSubject::Relationship(rel_id),
            kind: InfluenceKindId(1),
            predecessors: vec![ChangeId(0)],
            before: StateVector::from_slice(&[0.0, 0.0]),
            after: StateVector::from_slice(&[0.5, 0.0]),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };
        let downstream_change = Change {
            id: ChangeId(2),
            subject: ChangeSubject::Locus(LocusId(1)),
            kind: InfluenceKindId(1),
            predecessors: vec![ChangeId(1)],
            before: StateVector::from_slice(&[0.0]),
            after: StateVector::from_slice(&[0.5]),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };

        world.log_mut().append(root_change);
        world.log_mut().append(rel_change);
        world.log_mut().append(downstream_change);

        (world, rel_id, ChangeId(0))
    }

    #[test]
    fn relationships_caused_by_finds_downstream_relationship() {
        let (world, rel_id, root) = make_world_with_linear_causal_chain();
        let caused = relationships_caused_by(&world, &[root]);
        assert!(caused.contains(&rel_id), "relationship should be in caused set");
    }

    #[test]
    fn relationships_absent_without_finds_created_relationship() {
        let (world, rel_id, root) = make_world_with_linear_causal_chain();
        let absent = relationships_absent_without(&world, &[root]);
        assert!(absent.contains(&rel_id), "relationship should be absent without root");
    }

    #[test]
    fn empty_roots_returns_empty() {
        let (world, _, _) = make_world_with_linear_causal_chain();
        let caused = relationships_caused_by(&world, &[]);
        assert!(caused.is_empty());
        let absent = relationships_absent_without(&world, &[]);
        assert!(absent.is_empty());
    }

    #[test]
    fn unrelated_root_does_not_cause_relationship() {
        let (world, rel_id, _) = make_world_with_linear_causal_chain();
        // ChangeId(2) is downstream of the relationship, not its cause.
        let caused = relationships_caused_by(&world, &[ChangeId(2)]);
        assert!(!caused.contains(&rel_id));
    }
}
