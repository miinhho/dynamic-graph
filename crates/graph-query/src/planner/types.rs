#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CostClass {
    Index,
    Scan,
    Traversal,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlanStep {
    pub description: String,
    pub cost_class: CostClass,
    pub estimated_output: usize,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QueryPlan {
    pub steps: Vec<PlanStep>,
    pub estimated_candidates_initial: usize,
    pub estimated_output: usize,
}
