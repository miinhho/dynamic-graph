//! Program unit test fixture.
//!
//! [`ProgramFixture`] lets you test a [`LocusProgram`] in complete isolation —
//! no `Engine`, no `World`, no tick loop. You describe the inbox and a minimal
//! world context, call `run()`, and inspect the structured [`ProgramOutput`].
//!
//! ## Example
//!
//! ```rust,ignore
//! use graph_testkit::fixture::ProgramFixture;
//! use graph_testkit::programs::{ForwardProgram, TEST_KIND};
//! use graph_core::{LocusId, StateVector};
//!
//! let output = ProgramFixture::for_locus(LocusId(0), StateVector::from_slice(&[0.0]))
//!     .incoming_activation(LocusId(1), TEST_KIND, 0.8)
//!     .run(&ForwardProgram { downstream: LocusId(2), gain: 0.5 });
//!
//! assert_eq!(output.proposed.len(), 1);
//! let change = &output.proposed[0];
//! assert!(change.after.as_slice()[0] > 0.3);
//! ```
//!
//! ## What ProgramFixture does NOT cover
//!
//! - Multi-batch interactions (use an integration test with `Simulation`).
//! - Predecessor linking and `ChangeId` assignment (done by the engine).
//! - `auto-emerge` relationship creation (structural proposals are captured
//!   in `output.structural` but not applied; test them explicitly).

use std::collections::HashMap;

use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, Cohere, Entity, EntityId, InfluenceKindId,
    KindObservation, Locus, LocusContext, LocusId, LocusKindId, LocusProgram, ProposedChange,
    Relationship, RelationshipId, RelationshipKindId, RelationshipLineage, StateVector,
    StructuralProposal,
};

// ─── ProgramOutput ────────────────────────────────────────────────────────────

/// The result of running a [`LocusProgram`] through a [`ProgramFixture`].
///
/// Contains both outputs the engine would produce: the list of proposed state
/// changes (`proposed`) and the list of structural proposals (`structural`).
#[derive(Debug, Clone)]
pub struct ProgramOutput {
    /// Changes proposed by `LocusProgram::process`.
    pub proposed: Vec<ProposedChange>,
    /// Structural proposals from `LocusProgram::structural_proposals`.
    pub structural: Vec<StructuralProposal>,
}

impl ProgramOutput {
    /// `true` when the program produced no proposed changes and no structural proposals.
    pub fn is_empty(&self) -> bool {
        self.proposed.is_empty() && self.structural.is_empty()
    }

    /// `true` when at least one `ProposedChange` targets `locus`.
    pub fn has_change_to(&self, locus: LocusId) -> bool {
        self.proposed
            .iter()
            .any(|c| matches!(c.subject, ChangeSubject::Locus(id) if id == locus))
    }

    /// The `ProposedChange`s that target `locus`, in order.
    pub fn changes_to(&self, locus: LocusId) -> Vec<&ProposedChange> {
        self.proposed
            .iter()
            .filter(|c| matches!(c.subject, ChangeSubject::Locus(id) if id == locus))
            .collect()
    }

    /// Sum of slot-0 values across all proposed changes, regardless of target.
    ///
    /// Useful for checking whether a forwarding or broadcasting program
    /// produced the expected total output signal.
    pub fn total_output_signal(&self) -> f32 {
        self.proposed
            .iter()
            .map(|c| c.after.as_slice().first().copied().unwrap_or(0.0))
            .sum()
    }
}

// ─── TestLocusContext ─────────────────────────────────────────────────────────

/// A minimal, controllable [`LocusContext`] for unit tests.
///
/// Backed by in-memory hash maps. Populated by [`ProgramFixture`] before
/// calling the program under test. Programs that call `locus()`,
/// `relationships_for()`, `relationship()`, or `recent_changes()` see
/// exactly what was inserted.
pub struct TestLocusContext {
    loci: HashMap<LocusId, Locus>,
    relationships: HashMap<RelationshipId, Relationship>,
    /// Index: LocusId → Vec of RelationshipIds that involve it.
    by_locus: HashMap<LocusId, Vec<RelationshipId>>,
    current_batch: BatchId,
}

impl TestLocusContext {
    pub(crate) fn new(current_batch: BatchId) -> Self {
        Self {
            loci: HashMap::default(),
            relationships: HashMap::default(),
            by_locus: HashMap::default(),
            current_batch,
        }
    }

    /// Insert a locus into the context.
    pub fn insert_locus(&mut self, locus: Locus) {
        self.loci.insert(locus.id, locus);
    }

    /// Insert a relationship and update the by-locus index.
    pub fn insert_relationship(&mut self, rel: Relationship) {
        let id = rel.id;
        let endpoints = rel.endpoints.clone();
        self.relationships.insert(id, rel);

        match endpoints {
            graph_core::Endpoints::Directed { from, to } => {
                self.by_locus.entry(from).or_default().push(id);
                if from != to {
                    self.by_locus.entry(to).or_default().push(id);
                }
            }
            graph_core::Endpoints::Symmetric { a, b } => {
                self.by_locus.entry(a).or_default().push(id);
                if a != b {
                    self.by_locus.entry(b).or_default().push(id);
                }
            }
        }
    }
}

impl LocusContext for TestLocusContext {
    fn locus(&self, id: LocusId) -> Option<&Locus> {
        self.loci.get(&id)
    }

    fn relationships_for<'a>(
        &'a self,
        locus: LocusId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        let ids = self.by_locus.get(&locus);
        match ids {
            None => Box::new(std::iter::empty::<&Relationship>()),
            Some(ids) => Box::new(ids.iter().filter_map(|id| self.relationships.get(id))),
        }
    }

    fn relationship(&self, id: RelationshipId) -> Option<&Relationship> {
        self.relationships.get(&id)
    }

    fn current_batch(&self) -> BatchId {
        self.current_batch
    }

    fn entity_of(&self, _locus: LocusId) -> Option<&Entity> {
        None
    }

    fn entity(&self, _id: EntityId) -> Option<&Entity> {
        None
    }

    fn coheres(&self, _perspective: &str) -> Option<&[Cohere]> {
        None
    }
}

// ─── IncomingChangeBuilder ────────────────────────────────────────────────────

/// A single synthetic incoming [`Change`] in the program inbox.
struct IncomingSpec {
    from: LocusId,
    kind: InfluenceKindId,
    after: StateVector,
}

// ─── ProgramFixture ───────────────────────────────────────────────────────────

/// Builder for a unit test that exercises one [`LocusProgram`] in isolation.
///
/// Set up the locus under test, add neighbor loci and relationships to the
/// context, build a synthetic inbox, and call `run()`.
pub struct ProgramFixture {
    /// The locus that is the "subject" of the program call.
    target: Locus,
    /// Additional loci visible via the context.
    context_loci: Vec<Locus>,
    /// Relationships visible via the context.
    context_relationships: Vec<Relationship>,
    /// Inbox: synthetic incoming changes to feed to the program.
    inbox: Vec<IncomingSpec>,
    /// Batch id reported by the context.
    current_batch: BatchId,
    /// Next auto-incremented relationship id for `with_relationship`.
    next_rel_id: u64,
    /// Next auto-incremented change id for inbox construction.
    next_change_id: u64,
}

impl ProgramFixture {
    /// Create a fixture for `locus_id` with `initial_state`.
    ///
    /// The locus is created with a sentinel `LocusKindId(1)`. To use a
    /// specific kind, use [`for_locus_with_kind`][Self::for_locus_with_kind].
    pub fn for_locus(locus_id: LocusId, initial_state: StateVector) -> Self {
        Self::for_locus_with_kind(locus_id, LocusKindId(1), initial_state)
    }

    /// Create a fixture for `locus_id` with an explicit `LocusKindId`.
    pub fn for_locus_with_kind(
        locus_id: LocusId,
        kind: LocusKindId,
        initial_state: StateVector,
    ) -> Self {
        Self {
            target: Locus::new(locus_id, kind, initial_state),
            context_loci: Vec::new(),
            context_relationships: Vec::new(),
            inbox: Vec::new(),
            current_batch: BatchId(0),
            next_rel_id: 1,
            next_change_id: 0,
        }
    }

    // ── Context setup ─────────────────────────────────────────────────────────

    /// Set the `current_batch` reported by the context. Default is 0.
    pub fn at_batch(mut self, batch: BatchId) -> Self {
        self.current_batch = batch;
        self
    }

    /// Add a neighbor locus to the test context with zero initial state.
    pub fn with_neighbor(self, id: LocusId) -> Self {
        self.with_neighbor_state(id, StateVector::zeros(1))
    }

    /// Add a neighbor locus to the test context with a specific state.
    pub fn with_neighbor_state(mut self, id: LocusId, state: StateVector) -> Self {
        let dims = state.as_slice().len();
        self.context_loci
            .push(Locus::new(id, LocusKindId(1), state));
        // ensure the target locus is also in the context with the right dimensionality
        let _ = dims;
        self
    }

    /// Add a directed relationship from `from` → `to` of `kind` to the context.
    ///
    /// The relationship has activity = `activity` and weight = 1.0.
    pub fn with_relationship(
        mut self,
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
        activity: f32,
    ) -> Self {
        let rel_id = RelationshipId(self.next_rel_id);
        self.next_rel_id += 1;
        self.context_relationships.push(Relationship {
            id: rel_id,
            kind,
            endpoints: graph_core::Endpoints::Directed { from, to },
            state: StateVector::from_slice(&[activity, 1.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        self
    }

    // ── Inbox building ────────────────────────────────────────────────────────

    /// Add a locus-subject incoming change to the inbox.
    ///
    /// Simulates a signal of `value` (slot 0) arriving from `from_locus`
    /// via `kind`. The change targets the fixture's *own* locus (this is the
    /// most common case — programs react to changes *on themselves*).
    pub fn incoming_activation(
        mut self,
        from_locus: LocusId,
        kind: InfluenceKindId,
        value: f32,
    ) -> Self {
        self.inbox.push(IncomingSpec {
            from: from_locus,
            kind,
            after: StateVector::from_slice(&[value]),
        });
        self
    }

    /// Add a multi-dimensional incoming change. Use this when the program's
    /// `process` method reads slots beyond slot 0.
    pub fn incoming_vector(
        mut self,
        from_locus: LocusId,
        kind: InfluenceKindId,
        values: &[f32],
    ) -> Self {
        self.inbox.push(IncomingSpec {
            from: from_locus,
            kind,
            after: StateVector::from_slice(values),
        });
        self
    }

    // ── Execution ────────────────────────────────────────────────────────────

    /// Run the program and return the [`ProgramOutput`].
    ///
    /// Internally this:
    /// 1. Builds a [`TestLocusContext`] from the accumulated context loci and
    ///    relationships.
    /// 2. Converts each `IncomingSpec` into a synthetic [`Change`] with a
    ///    `ChangeSubject::Locus(target)` (so the program sees them as changes
    ///    addressed to its own locus, the common case for inbox delivery).
    /// 3. Calls `program.process(&target, &incoming[..], &ctx)` and
    ///    `program.structural_proposals(&target, &incoming[..], &ctx)`.
    pub fn run<P: LocusProgram>(&self, program: &P) -> ProgramOutput {
        let mut ctx = TestLocusContext::new(self.current_batch);

        // Seed context with the target locus itself so programs that read their
        // own state via ctx.locus(self.target.id) see it.
        ctx.insert_locus(self.target.clone());

        for locus in &self.context_loci {
            ctx.insert_locus(locus.clone());
        }
        for rel in &self.context_relationships {
            ctx.insert_relationship(rel.clone());
        }

        // Build the inbox: each IncomingSpec → synthetic Change addressed to
        // the target locus. The predecessor list is empty (no real batch context).
        let mut change_id = self.next_change_id;
        let owned_changes: Vec<Change> = self
            .inbox
            .iter()
            .map(|spec| {
                let c = Change {
                    id: ChangeId(change_id),
                    subject: ChangeSubject::Locus(self.target.id),
                    kind: spec.kind,
                    predecessors: vec![],
                    before: StateVector::zeros(spec.after.as_slice().len()),
                    after: spec.after.clone(),
                    batch: self.current_batch,
                    wall_time: None,
                    metadata: None,
                };
                change_id += 1;
                c
            })
            .collect();

        let incoming: Vec<&Change> = owned_changes.iter().collect();

        let proposed = program.process(&self.target, &incoming, &ctx);
        let structural = program.structural_proposals(&self.target, &incoming, &ctx);

        ProgramOutput {
            proposed,
            structural,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{ChangeSubject, InfluenceKindId, LocusId, StateVector};

    use crate::programs::{
        AccumulatorProgram, BroadcastProgram, ForwardProgram, InertProgram, TEST_KIND,
    };

    // ── InertProgram produces no output ─────────────────────────────────────

    #[test]
    fn inert_program_produces_no_output() {
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .incoming_activation(LocusId(1), TEST_KIND, 0.5)
            .run(&InertProgram);
        assert!(output.proposed.is_empty());
        assert!(output.structural.is_empty());
        assert!(output.is_empty());
    }

    // ── ForwardProgram forwards to downstream ────────────────────────────────

    #[test]
    fn forward_program_produces_downstream_change() {
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .incoming_activation(LocusId(1), TEST_KIND, 0.8)
            .run(&ForwardProgram {
                downstream: LocusId(2),
                gain: 0.5,
            });

        assert_eq!(output.proposed.len(), 1);
        let change = &output.proposed[0];
        assert!(matches!(change.subject, ChangeSubject::Locus(LocusId(2))));
        let val = change.after.as_slice()[0];
        assert!((val - 0.4).abs() < 1e-5, "expected 0.4, got {val}");
    }

    #[test]
    fn forward_program_silent_below_noise_floor() {
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .incoming_activation(LocusId(1), TEST_KIND, 0.0001)
            .run(&ForwardProgram {
                downstream: LocusId(2),
                gain: 1.0,
            });
        assert!(output.proposed.is_empty());
    }

    // ── AccumulatorProgram adds to own state ─────────────────────────────────

    #[test]
    fn accumulator_program_adds_to_own_state() {
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::from_slice(&[0.3]))
            .incoming_activation(LocusId(1), TEST_KIND, 0.5)
            .run(&AccumulatorProgram { gain: 1.0 });

        assert_eq!(output.proposed.len(), 1);
        let val = output.proposed[0].after.as_slice()[0];
        // 0.3 (current) + 0.5 (incoming) * 1.0 (gain) = 0.8
        assert!((val - 0.8).abs() < 1e-5, "expected 0.8, got {val}");
    }

    // ── BroadcastProgram fans out to all downstreams ─────────────────────────

    #[test]
    fn broadcast_program_fans_out() {
        let downstreams = vec![LocusId(1), LocusId(2), LocusId(3)];
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .incoming_activation(LocusId(9), TEST_KIND, 0.6)
            .run(&BroadcastProgram {
                downstreams: downstreams.clone(),
                gain: 0.5,
            });

        assert_eq!(output.proposed.len(), 3);
        for (i, change) in output.proposed.iter().enumerate() {
            assert!(
                matches!(change.subject, ChangeSubject::Locus(id) if id == downstreams[i]),
                "expected downstream {:?}, got {:?}",
                downstreams[i],
                change.subject
            );
            let val = change.after.as_slice()[0];
            assert!((val - 0.3).abs() < 1e-5, "expected 0.3, got {val}");
        }
    }

    // ── ProgramOutput helpers ─────────────────────────────────────────────────

    #[test]
    fn has_change_to_detects_by_locus() {
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .incoming_activation(LocusId(1), TEST_KIND, 0.5)
            .run(&ForwardProgram {
                downstream: LocusId(7),
                gain: 1.0,
            });

        assert!(output.has_change_to(LocusId(7)));
        assert!(!output.has_change_to(LocusId(0)));
        assert!(!output.has_change_to(LocusId(99)));
    }

    #[test]
    fn total_output_signal_sums_slot_zero() {
        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .incoming_activation(LocusId(1), TEST_KIND, 0.5)
            .run(&BroadcastProgram {
                downstreams: vec![LocusId(1), LocusId(2)],
                gain: 1.0,
            });

        let total = output.total_output_signal();
        assert!((total - 1.0).abs() < 1e-5, "expected 1.0, got {total}");
    }

    // ── Context — locus visible to program ───────────────────────────────────

    #[test]
    fn context_locus_visible_to_program() {
        struct ReadNeighborProgram;
        impl LocusProgram for ReadNeighborProgram {
            fn process(
                &self,
                _: &Locus,
                _: &[&Change],
                ctx: &dyn LocusContext,
            ) -> Vec<ProposedChange> {
                // Reads the neighbor's state via ctx.
                let val = ctx
                    .locus(LocusId(5))
                    .and_then(|l| l.state.as_slice().first().copied());
                if let Some(v) = val {
                    vec![ProposedChange::new(
                        ChangeSubject::Locus(LocusId(0)),
                        TEST_KIND,
                        StateVector::from_slice(&[v]),
                    )]
                } else {
                    vec![]
                }
            }
        }

        let output = ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1))
            .with_neighbor_state(LocusId(5), StateVector::from_slice(&[0.77]))
            .incoming_activation(LocusId(1), TEST_KIND, 0.1)
            .run(&ReadNeighborProgram);

        assert_eq!(output.proposed.len(), 1);
        let val = output.proposed[0].after.as_slice()[0];
        assert!((val - 0.77).abs() < 1e-5, "expected 0.77, got {val}");
    }

    // ── No inbox → empty output for ForwardProgram ────────────────────────────

    #[test]
    fn no_inbox_produces_no_output() {
        let output =
            ProgramFixture::for_locus(LocusId(0), StateVector::zeros(1)).run(&ForwardProgram {
                downstream: LocusId(1),
                gain: 1.0,
            });
        assert!(output.is_empty());
    }
}
