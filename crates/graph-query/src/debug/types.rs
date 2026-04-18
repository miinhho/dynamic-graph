use std::fmt;

use graph_core::{BatchId, ChangeId, ChangeSubject, InfluenceKindId, LocusId, StateVector};

use super::render;

#[derive(Debug, Clone)]
pub struct CausalStep {
    pub depth: usize,
    pub change_id: ChangeId,
    pub subject: ChangeSubject,
    pub kind: InfluenceKindId,
    pub before: StateVector,
    pub after: StateVector,
    pub batch: BatchId,
    pub predecessor_ids: Vec<ChangeId>,
}

#[derive(Debug, Clone)]
pub struct CausalTrace {
    pub target: LocusId,
    pub batch: BatchId,
    pub steps: Vec<CausalStep>,
    pub truncated: bool,
}

impl fmt::Display for CausalTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render::render_causal_trace(self, f)
    }
}
