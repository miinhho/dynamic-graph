//! Standard locus programs used by the testkit fixtures.
//!
//! All programs are stateless and `Send + Sync`. Each represents one
//! simple behavioural archetype useful for testing the engine's batch
//! loop, relationship auto-emergence, and entity recognition.

use graph_core::{Change, ChangeSubject, InfluenceKindId, Locus, LocusContext, LocusId, LocusProgram, ProposedChange, StateVector};

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
