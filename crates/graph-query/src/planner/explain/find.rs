use graph_core::{EndpointKey, LocusId, RelationshipKindId};
use graph_world::World;

use crate::planner::{
    CostClass, PlanStep, QueryPlan, SeedKind, plan_loci_predicates, plan_rel_predicates,
};
use crate::query_api::{LocusPredicate, RelationshipPredicate};

pub(super) fn explain_find_relationships(
    world: &World,
    predicates: &[RelationshipPredicate],
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    let plan = plan_rel_predicates(predicates);
    let (mut steps, initial) = explain_relationship_seed(world, &plan.seed_locus);
    let est = append_predicate_steps(&mut steps, initial, &plan.predicates_ordered, rel_pred_desc);
    finalize_plan(steps, initial, est, has_sort, limit)
}

pub(super) fn explain_find_loci(
    world: &World,
    predicates: &[LocusPredicate],
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    let total = world.loci().len();
    let mut steps = vec![PlanStep {
        description: format!("full locus scan ({} loci)", total),
        cost_class: CostClass::Scan,
        estimated_output: total,
    }];
    let ordered = plan_loci_predicates(predicates);
    let est = append_predicate_steps(&mut steps, total, &ordered, locus_pred_desc);
    finalize_plan(steps, total, est, has_sort, limit)
}

pub(super) fn explain_find_entities(
    world: &World,
    pred_count: usize,
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    let total = world.entities().active().count();
    let mut est = total;
    let mut steps = vec![PlanStep {
        description: format!("active entity scan ({} entities)", total),
        cost_class: CostClass::Scan,
        estimated_output: total,
    }];
    for i in 0..pred_count {
        est = (est as f32 * 0.6) as usize;
        steps.push(PlanStep {
            description: format!("filter predicate[{}]", i),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    finalize_plan(steps, total, est, has_sort, limit)
}

fn explain_relationship_seed(world: &World, seed: &Option<SeedKind>) -> (Vec<PlanStep>, usize) {
    let (step, initial) = relationship_seed_step(world, seed);
    (vec![step], initial)
}

fn relationship_seed_step(world: &World, seed: &Option<SeedKind>) -> (PlanStep, usize) {
    let total_rels = world.relationships().len();
    match seed {
        Some(SeedKind::DirectLookup { from, to, kind }) => {
            direct_lookup_seed_step(world, *from, *to, *kind)
        }
        Some(SeedKind::Between { a, b }) => degree_seed_step(
            format!(
                "between-loci scan: ({}, {}) → O(min_degree={})",
                a.0,
                b.0,
                world.degree(*a).min(world.degree(*b))
            ),
            world.degree(*a).min(world.degree(*b)),
        ),
        Some(SeedKind::From(id)) => adjacency_seed_step("From", id, world.degree(*id)),
        Some(SeedKind::To(id)) => adjacency_seed_step("To", id, world.degree(*id)),
        Some(SeedKind::Touching(id)) => adjacency_seed_step("Touching", id, world.degree(*id)),
        None => full_relationship_scan_step(total_rels),
    }
}

fn direct_lookup_seed_step(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: RelationshipKindId,
) -> (PlanStep, usize) {
    let key = EndpointKey::Directed(from, to);
    let found = world.relationships().lookup(&key, kind).is_some() as usize;
    (
        PlanStep {
            description: format!(
                "direct lookup: From({}) To({}) OfKind({}) → O(1)",
                from.0, to.0, kind.0
            ),
            cost_class: CostClass::Index,
            estimated_output: found,
        },
        found,
    )
}

fn degree_seed_step(description: String, estimated_output: usize) -> (PlanStep, usize) {
    (
        PlanStep {
            description,
            cost_class: CostClass::Index,
            estimated_output,
        },
        estimated_output,
    )
}

fn adjacency_seed_step(label: &str, id: &LocusId, degree: usize) -> (PlanStep, usize) {
    degree_seed_step(
        format!("adjacency seed: {}({}) → O(degree={})", label, id.0, degree),
        degree,
    )
}

fn full_relationship_scan_step(total_rels: usize) -> (PlanStep, usize) {
    (
        PlanStep {
            description: format!("full relationship scan ({} edges)", total_rels),
            cost_class: CostClass::Scan,
            estimated_output: total_rels,
        },
        total_rels,
    )
}

fn append_predicate_steps<P>(
    steps: &mut Vec<PlanStep>,
    initial_estimate: usize,
    predicates: &[&P],
    describe: fn(&P) -> (String, CostClass, f32),
) -> usize {
    predicates.iter().fold(initial_estimate, |estimate, pred| {
        let (description, cost_class, selectivity) = describe(pred);
        let next_estimate = apply_selectivity(estimate, selectivity);
        steps.push(PlanStep {
            description,
            cost_class,
            estimated_output: next_estimate,
        });
        next_estimate
    })
}

fn apply_selectivity(estimate: usize, selectivity: f32) -> usize {
    (estimate as f32 * selectivity) as usize
}

fn finalize_plan(
    mut steps: Vec<PlanStep>,
    initial_candidates: usize,
    mut estimated_output: usize,
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    if has_sort {
        steps.push(PlanStep {
            description: "sort".to_string(),
            cost_class: CostClass::Scan,
            estimated_output,
        });
    }
    if let Some(n) = limit {
        estimated_output = estimated_output.min(n);
        steps.push(PlanStep {
            description: format!("limit {}", n),
            cost_class: CostClass::Scan,
            estimated_output,
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: initial_candidates,
        estimated_output,
    }
}

fn rel_pred_desc(pred: &RelationshipPredicate) -> (String, CostClass, f32) {
    match pred {
        RelationshipPredicate::From(id) => (format!("filter From({})", id.0), CostClass::Scan, 0.3),
        RelationshipPredicate::To(id) => (format!("filter To({})", id.0), CostClass::Scan, 0.3),
        RelationshipPredicate::Touching(id) => {
            (format!("filter Touching({})", id.0), CostClass::Scan, 0.5)
        }
        RelationshipPredicate::OfKind(k) => {
            (format!("filter OfKind({})", k.0), CostClass::Scan, 0.4)
        }
        RelationshipPredicate::ActivityAbove(v) => {
            (format!("filter activity > {:.3}", v), CostClass::Scan, 0.5)
        }
        RelationshipPredicate::StrengthAbove(v) => {
            (format!("filter strength > {:.3}", v), CostClass::Scan, 0.5)
        }
        RelationshipPredicate::SlotAbove { slot, min } => (
            format!("filter slot[{}] >= {:.3}", slot, min),
            CostClass::Scan,
            0.5,
        ),
        RelationshipPredicate::MinChangeCount(n) => (
            format!("filter change_count >= {}", n),
            CostClass::Scan,
            0.4,
        ),
        RelationshipPredicate::CreatedInRange { .. } => (
            "filter created_batch in range".to_string(),
            CostClass::Scan,
            0.3,
        ),
        RelationshipPredicate::OlderThan { .. } => {
            ("filter older_than".to_string(), CostClass::Scan, 0.4)
        }
    }
}

fn locus_pred_desc(pred: &LocusPredicate) -> (String, CostClass, f32) {
    match pred {
        LocusPredicate::OfKind(k) => (format!("filter OfKind({})", k.0), CostClass::Scan, 0.4),
        LocusPredicate::StateAbove { slot, min } => (
            format!("filter state[{}] >= {:.3}", slot, min),
            CostClass::Scan,
            0.5,
        ),
        LocusPredicate::StateBelow { slot, max } => (
            format!("filter state[{}] <= {:.3}", slot, max),
            CostClass::Scan,
            0.5,
        ),
        LocusPredicate::F64PropertyAbove { key, min } => (
            format!("filter {}(f64) > {:.3}", key, min),
            CostClass::Scan,
            0.4,
        ),
        LocusPredicate::StrPropertyEq { key, value } => {
            (format!("filter {}=={}", key, value), CostClass::Scan, 0.3)
        }
        LocusPredicate::MinDegree(n) => (format!("filter degree >= {}", n), CostClass::Scan, 0.4),
        LocusPredicate::ReachableFrom { start, depth } => (
            format!("BFS reachable_from({}, depth={})", start.0, depth),
            CostClass::Traversal,
            0.3,
        ),
        LocusPredicate::DownstreamOf { start, depth } => (
            format!("BFS downstream_of({}, depth={})", start.0, depth),
            CostClass::Traversal,
            0.3,
        ),
        LocusPredicate::UpstreamOf { start, depth } => (
            format!("BFS upstream_of({}, depth={})", start.0, depth),
            CostClass::Traversal,
            0.3,
        ),
        LocusPredicate::ReachableFromActive {
            start,
            depth,
            min_activity,
        } => (
            format!(
                "active BFS reachable_from({}, depth={}, min_act={:.3})",
                start.0, depth, min_activity
            ),
            CostClass::Traversal,
            0.25,
        ),
        LocusPredicate::DownstreamOfActive {
            start,
            depth,
            min_activity,
        } => (
            format!(
                "active BFS downstream_of({}, depth={}, min_act={:.3})",
                start.0, depth, min_activity
            ),
            CostClass::Traversal,
            0.25,
        ),
        LocusPredicate::UpstreamOfActive {
            start,
            depth,
            min_activity,
        } => (
            format!(
                "active BFS upstream_of({}, depth={}, min_act={:.3})",
                start.0, depth, min_activity
            ),
            CostClass::Traversal,
            0.25,
        ),
    }
}
