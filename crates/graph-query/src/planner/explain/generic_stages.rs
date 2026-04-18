use graph_world::World;

use crate::planner::{CostClass, PlanStep, QueryPlan};

#[derive(Debug, Clone, Copy)]
pub(super) struct WorldStats {
    pub(super) loci: usize,
    pub(super) relationships: usize,
    pub(super) log_entries: usize,
    pub(super) entities: usize,
}

impl WorldStats {
    pub(super) fn from_world(world: &World) -> Self {
        Self {
            loci: world.loci().len(),
            relationships: world.relationships().len(),
            log_entries: world.log().len(),
            entities: world.entities().len(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum GenericQueryDescription {
    SingleStage(StageBlueprint),
    Traversal(TraversalBlueprint),
    Fallback(String),
}

#[derive(Debug, Clone)]
pub(super) struct StageBlueprint {
    pub(super) initial_candidates: usize,
    pub(super) description: String,
    pub(super) cost_class: CostClass,
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct TraversalBlueprint {
    pub(super) initial_candidates: usize,
    pub(super) description: String,
    pub(super) limit: Option<usize>,
}

pub(super) fn assemble_query_plan(description: GenericQueryDescription) -> QueryPlan {
    match description {
        GenericQueryDescription::SingleStage(stage) => assemble_single_stage_plan(stage),
        GenericQueryDescription::Traversal(stage) => assemble_traversal_plan(stage),
        GenericQueryDescription::Fallback(description) => assemble_fallback_plan(description),
    }
}

fn assemble_single_stage_plan(stage: StageBlueprint) -> QueryPlan {
    let estimated_output = limit_output(stage.initial_candidates, stage.limit);
    QueryPlan {
        steps: vec![PlanStep {
            description: stage.description,
            cost_class: stage.cost_class,
            estimated_output,
        }],
        estimated_candidates_initial: stage.initial_candidates,
        estimated_output,
    }
}

fn assemble_traversal_plan(stage: TraversalBlueprint) -> QueryPlan {
    let mut steps = vec![
        PlanStep {
            description: stage.description,
            cost_class: CostClass::Traversal,
            estimated_output: stage.initial_candidates,
        },
        PlanStep {
            description: "sort descending".to_string(),
            cost_class: CostClass::Scan,
            estimated_output: stage.initial_candidates,
        },
    ];
    if let Some(limit) = stage.limit {
        steps.push(PlanStep {
            description: format!("limit {}", limit),
            cost_class: CostClass::Scan,
            estimated_output: stage.initial_candidates.min(limit),
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: stage.initial_candidates,
        estimated_output: stage.limit.unwrap_or(stage.initial_candidates),
    }
}

fn assemble_fallback_plan(description: String) -> QueryPlan {
    QueryPlan {
        estimated_candidates_initial: 1,
        estimated_output: 1,
        steps: vec![PlanStep {
            description,
            cost_class: CostClass::Scan,
            estimated_output: 1,
        }],
    }
}

fn limit_output(initial_candidates: usize, limit: Option<usize>) -> usize {
    limit.map_or(initial_candidates, |limit| initial_candidates.min(limit))
}
