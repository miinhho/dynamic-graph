//! Standard locus programs used by the testkit fixtures.
//!
//! All programs are stateless and `Send + Sync`. Each represents one
//! simple behavioural archetype useful for testing the engine's batch
//! loop, relationship auto-emergence, and entity recognition.

use graph_core::{
    Change, ChangeSubject, InfluenceKindId, Locus, LocusContext, LocusId, LocusProgram,
    ProposedChange, RelationshipId, StateVector, StructuralProposal,
};

/// Influence kind used by all testkit fixtures. Callers must register
/// this id in their `InfluenceKindRegistry`.
pub const TEST_KIND: InfluenceKindId = InfluenceKindId(1);

/// Never emits. Used as a terminal sink in chain/star topologies.
pub struct InertProgram;
impl LocusProgram for InertProgram {
    fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        Vec::new()
    }
}

/// On every incoming change, copies the signal to one downstream locus
/// scaled by `gain`. Produces nothing below a noise floor of 0.001 so
/// that chains quiesce naturally without hitting the batch cap.
pub struct ForwardProgram {
    pub downstream: graph_core::LocusId,
    pub gain: f32,
}
impl LocusProgram for ForwardProgram {
    fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        let total: f32 = incoming.iter().flat_map(|c| c.after.as_slice()).sum();
        if total.abs() < 0.001 {
            return Vec::new();
        }
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.downstream),
            TEST_KIND,
            StateVector::from_slice(&[total * self.gain]),
        )]
    }
}

/// Adds `gain * incoming_signal` to its own current state. Self-reinforcing
/// on each touch — used to test divergence guard rails.
pub struct AccumulatorProgram {
    pub gain: f32,
}

/// Copies the incoming signal to *all* downstreams scaled by `gain`.
/// Used to build fan-out (star) topologies. Same noise floor as
/// `ForwardProgram` (0.001) so the hub quiesces naturally.
pub struct BroadcastProgram {
    pub downstreams: Vec<LocusId>,
    pub gain: f32,
}
impl LocusProgram for BroadcastProgram {
    fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        let total: f32 = incoming.iter().flat_map(|c| c.after.as_slice()).sum();
        if total.abs() < 0.001 {
            return Vec::new();
        }
        self.downstreams
            .iter()
            .map(|&ds| {
                ProposedChange::new(
                    ChangeSubject::Locus(ds),
                    TEST_KIND,
                    StateVector::from_slice(&[total * self.gain]),
                )
            })
            .collect()
    }
}

impl LocusProgram for AccumulatorProgram {
    fn process(&self, locus: &Locus, incoming: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        let total: f32 = incoming.iter().flat_map(|c| c.after.as_slice()).sum();
        if total.abs() < 0.001 {
            return Vec::new();
        }
        let current = locus.state.as_slice().first().copied().unwrap_or(0.0);
        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            TEST_KIND,
            StateVector::from_slice(&[current + total * self.gain]),
        )]
    }
}

/// A locus that models a complex, N-ary real-world event.
///
/// ## State layout
///
/// `[activation_level, severity, confidence]`
///
/// - `activation_level`: accumulated incoming signal (sums over all
///   locus-subject changes in the inbox).
/// - `severity`: updated when a locus change exceeds `activation_threshold`
///   (set to the max incoming signal seen so far).
/// - `confidence`: nudged upward when subscribed relationships change
///   (each notification adds `confidence_per_rel_change`).
///
/// ## Behaviour
///
/// On each batch where the inbox contains **locus-subject** changes, the
/// program accumulates the signal. Once `activation_level` crosses
/// `activation_threshold` the event "fires":
/// - It produces a `ProposedChange` on itself to record the activation.
/// - `structural_proposals` creates directed relationships to all
///   `participants` (idempotent — existing edges just get an activity bump).
///
/// On each batch where the inbox contains **relationship-subject** changes
/// (delivered because this locus subscribed to those relationships), the
/// program updates `confidence` without re-firing the full activation logic.
///
/// ## Subscribing
///
/// Pass `watch_relationships` at construction time. On every
/// `structural_proposals` call the program emits
/// `SubscribeToRelationship` for each watched ID. Because subscriptions
/// are idempotent the cost is O(|watch_relationships|) per batch when
/// the event is active — typically very small.
///
/// ## Use case
///
/// Use this in integration tests to verify:
/// 1. N-ary event participation (multiple participants get relationships).
/// 2. Meta-locus reaction to relationship changes (confidence tracking).
/// 3. End-to-end subscriber delivery within a single batch.
pub struct EventLocusProgram {
    /// Loci that participate in this event. When activated, directed
    /// relationships `EventLocus → participant` are proposed.
    pub participants: Vec<LocusId>,
    /// Incoming signal sum that triggers event activation.
    pub activation_threshold: f32,
    /// Relationship kind used for participant edges.
    pub event_kind: InfluenceKindId,
    /// Relationships whose state changes should be forwarded to this
    /// locus's inbox via the subscriber mechanism.
    pub watch_relationships: Vec<RelationshipId>,
    /// How much `confidence` (slot 2) increases per relationship-change
    /// notification received.
    pub confidence_per_rel_change: f32,
}

impl EventLocusProgram {
    pub fn new(
        participants: Vec<LocusId>,
        activation_threshold: f32,
        event_kind: InfluenceKindId,
    ) -> Self {
        Self {
            participants,
            activation_threshold,
            event_kind,
            watch_relationships: Vec::new(),
            confidence_per_rel_change: 0.1,
        }
    }

    pub fn watching(mut self, rel_ids: Vec<RelationshipId>) -> Self {
        self.watch_relationships = rel_ids;
        self
    }
}

/// Slot indices for `EventLocusProgram` state.
impl EventLocusProgram {
    pub const ACTIVATION_SLOT: usize = 0;
    pub const SEVERITY_SLOT: usize = 1;
    pub const CONFIDENCE_SLOT: usize = 2;
}

impl LocusProgram for EventLocusProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let current = locus.state.as_slice();
        let mut activation = current.get(Self::ACTIVATION_SLOT).copied().unwrap_or(0.0);
        let mut severity = current.get(Self::SEVERITY_SLOT).copied().unwrap_or(0.0);
        let mut confidence = current.get(Self::CONFIDENCE_SLOT).copied().unwrap_or(0.0);

        let mut any_locus_change = false;
        let mut any_rel_change = false;

        for change in incoming {
            match change.subject {
                ChangeSubject::Locus(_) => {
                    let signal = change.after.as_slice().first().copied().unwrap_or(0.0);
                    activation += signal;
                    if signal > severity {
                        severity = signal;
                    }
                    any_locus_change = true;
                }
                ChangeSubject::Relationship(_) => {
                    // Delivered by the subscriber mechanism — a relationship
                    // this event is monitoring just changed state.
                    confidence += self.confidence_per_rel_change;
                    any_rel_change = true;
                }
            }
        }

        if !any_locus_change && !any_rel_change {
            return Vec::new();
        }

        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            self.event_kind,
            StateVector::from_slice(&[activation, severity, confidence]),
        )]
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        let mut proposals = Vec::new();

        // Re-subscribe to watched relationships every batch (idempotent).
        for &rel_id in &self.watch_relationships {
            proposals.push(StructuralProposal::subscribe(locus.id, rel_id));
        }

        // When the activation threshold is crossed, create participant edges.
        let incoming_activation: f32 = incoming
            .iter()
            .filter(|c| matches!(c.subject, ChangeSubject::Locus(_)))
            .flat_map(|c| c.after.as_slice().first().copied())
            .sum();

        let current_activation = locus.state.as_slice().first().copied().unwrap_or(0.0);

        if incoming_activation + current_activation >= self.activation_threshold {
            for &participant in &self.participants {
                proposals.push(StructuralProposal::create_directed(locus.id, participant, self.event_kind));
            }
        }

        proposals
    }
}

/// Aggregates all incoming `dims`-dimensional state vectors by weighted
/// sum, then forwards the result to one downstream locus. Represents a
/// realistic neuron-like computation: multiple inputs, multi-dimensional
/// state, non-trivial per-change work.
///
/// Used by `fan_in_world` to produce a benchmark workload where
/// `process()` cost grows with `dims` × number of incoming changes.
pub struct MultiDimAggregatorProgram {
    pub downstream: LocusId,
    pub dims: usize,
    pub gain: f32,
}

impl LocusProgram for MultiDimAggregatorProgram {
    fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return Vec::new();
        }
        let mut result = vec![0.0f32; self.dims];
        for c in incoming {
            let src = c.after.as_slice();
            for (r, &v) in result.iter_mut().zip(src.iter()) {
                *r += v * self.gain;
            }
        }
        // L2-normalise so the signal doesn't blow up across batches.
        let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-6 {
            return Vec::new();
        }
        for v in &mut result {
            *v /= norm;
        }
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.downstream),
            TEST_KIND,
            StateVector::from_slice(&result),
        )]
    }
}
