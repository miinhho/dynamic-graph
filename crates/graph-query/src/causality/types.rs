use graph_core::{ChangeId, TrimSummary};

#[derive(Debug, Clone, PartialEq)]
pub enum Trend {
    Rising { slope: f32 },
    Falling { slope: f32 },
    Stable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoarseTrail {
    pub fine: Vec<ChangeId>,
    pub coarse: Vec<TrimSummary>,
}

impl CoarseTrail {
    pub fn is_empty(&self) -> bool {
        self.fine.is_empty() && self.coarse.is_empty()
    }

    pub fn is_exact(&self) -> bool {
        self.coarse.is_empty()
    }
}
