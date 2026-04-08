//! Standard locus programs used by the testkit fixtures.
//!
//! All programs are stateless and `Send + Sync`. Each represents one
//! simple behavioural archetype useful for testing the engine's batch
//! loop, relationship auto-emergence, and entity recognition.

use graph_core::{Change, ChangeSubject, InfluenceKindId, Locus, LocusId, LocusProgram, ProposedChange, StateVector};

/// Influence kind used by all testkit fixtures. Callers must register
/// this id in their `InfluenceKindRegistry`.
pub const TEST_KIND: InfluenceKindId = InfluenceKindId(1);

/// Never emits. Used as a terminal sink in chain/star topologies.
pub struct InertProgram;
impl LocusProgram for InertProgram {
    fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
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
    fn process(&self, _: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
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
    fn process(&self, _: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
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
    fn process(&self, locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
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
