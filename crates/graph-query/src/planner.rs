//! Query planner for the serializable Query API.
//!
//! ## What this does
//!
//! `explain()` returns a human-readable [`QueryPlan`] showing which steps
//! `execute()` will take and their cost class.  `execute()` and `explain()`
//! share the same planning logic so there is no drift between the two.
//!
//! ## The key optimisation
//!
//! For `FindRelationships`, if any of `From(id)`, `To(id)`, or `Touching(id)`
//! is present, the planner seeds the candidate set from the adjacency index
//! (`world.relationships_for_locus(id)`) rather than doing a full scan over
//! every relationship.  On a large graph this is O(degree) vs O(total) —
//! typically a 10-100× reduction in candidates before any filtering.
//!
//! Remaining predicates are applied in priority order:
//!
//! | Tier | Cost | Predicates |
//! |------|------|------------|
//! | `Index`     | O(degree) | `From`, `To`, `Touching` — used as seed |
//! | `Scan`      | O(candidates) | `OfKind`, value filters, range filters |
//! | `Traversal` | O(V+E)    | `ReachableFrom`, `DownstreamOf`, `UpstreamOf` |

use graph_core::{LocusId, RelationshipKindId};
use graph_world::World;

use crate::query_api::{LocusPredicate, Query, RelationshipPredicate};

// ─── Public types ─────────────────────────────────────────────────────────────

/// Execution cost tier for a single planning step.
///
/// Steps are always ordered cheapest-first within a plan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CostClass {
    /// O(degree) — seeds candidates from the adjacency index.
    ///
    /// Only `From`, `To`, and `Touching` predicates qualify; the first one
    /// found is promoted to seed the initial candidate list instead of a full
    /// scan.
    Index,
    /// O(candidates) — filters the current candidate list in a single pass.
    Scan,
    /// O(V+E) — BFS/DFS traversal over the full graph.
    Traversal,
}

/// One step in a [`QueryPlan`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlanStep {
    /// Human-readable description of this step.
    pub description: String,
    /// How expensive this step is.
    pub cost_class: CostClass,
    /// Estimated number of candidates remaining after this step.
    ///
    /// This is a static heuristic estimate, not a cardinality statistic.
    pub estimated_output: usize,
}

/// A complete, ordered execution plan for a [`Query`].
///
/// Returned by [`explain`].  Call `execute` to actually run the query; the two
/// functions share the same planning logic so the plan is always accurate.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QueryPlan {
    /// Ordered steps showing how the query will execute.
    pub steps: Vec<PlanStep>,
    /// Estimated size of the raw candidate set before any filtering.
    pub estimated_candidates_initial: usize,
    /// Estimated output size after all filtering / sorting / limiting.
    pub estimated_output: usize,
}

// ─── Public: explain ─────────────────────────────────────────────────────────

/// Describe how [`execute`](crate::api::execute) will run `query` against `world`.
///
/// The returned [`QueryPlan`] lists steps in the order they will execute,
/// annotated with cost class and size estimates.  You can call this before
/// `execute` to understand performance characteristics without running the
/// query.
///
/// ```ignore
/// let plan = graph_query::api::explain(&world, &query);
/// for step in &plan.steps {
///     println!("[{:?}] {} (~{} candidates)", step.cost_class, step.description, step.estimated_output);
/// }
/// ```
pub fn explain(world: &World, query: &Query) -> QueryPlan {
    match query {
        Query::FindRelationships {
            predicates,
            sort_by,
            limit,
        } => explain_find_relationships(world, predicates, sort_by.is_some(), *limit),
        Query::FindLoci {
            predicates,
            sort_by,
            limit,
        } => explain_find_loci(world, predicates, sort_by.is_some(), *limit),
        Query::FindEntities {
            predicates,
            sort_by,
            limit,
        } => explain_find_entities(world, predicates.len(), sort_by.is_some(), *limit),
        // Causal-strength queries: O(R) scan over all directed relationships of the kind.
        Query::CausalDirection { kind, .. } => single_scan_plan(
            world.relationships().len(),
            &format!(
                "causal_direction scan over relationships of kind {:?}",
                kind
            ),
            Some(1),
        ),
        Query::DominantCauses { kind, n, .. } => single_scan_plan(
            world.relationships().len(),
            &format!("dominant_causes scan for kind {:?}, top {}", kind, n),
            Some(*n),
        ),
        Query::DominantEffects { kind, n, .. } => single_scan_plan(
            world.relationships().len(),
            &format!("dominant_effects scan for kind {:?}, top {}", kind, n),
            Some(*n),
        ),
        Query::CausalInStrength { kind, .. } | Query::CausalOutStrength { kind, .. } => {
            single_scan_plan(
                world.relationships().len(),
                &format!("causal strength scan over relationships of kind {:?}", kind),
                Some(1),
            )
        }
        Query::FeedbackPairs { kind, .. } => single_scan_plan(
            world.relationships().len(),
            &format!("feedback_pairs scan for kind {:?} (two passes)", kind),
            None,
        ),

        // D2: Granger-style causality — O(ChangeLog) scan per locus pair.
        Query::GrangerScore { kind, .. } => single_scan_plan(
            world.log().len(),
            &format!("granger_score ChangeLog scan for kind {:?}", kind),
            Some(1),
        ),
        Query::GrangerDominantCauses { kind, n, .. } => single_scan_plan(
            world.log().len(),
            &format!(
                "granger_dominant_causes ChangeLog scan for kind {:?}, top {}",
                kind, n
            ),
            Some(*n),
        ),
        Query::GrangerDominantEffects { kind, n, .. } => single_scan_plan(
            world.log().len(),
            &format!(
                "granger_dominant_effects ChangeLog scan for kind {:?}, top {}",
                kind, n
            ),
            Some(*n),
        ),

        // B3: Time-travel — O(k + R + E·L_avg), same as WorldDiff::compute.
        Query::TimeTravel { .. } => single_scan_plan(
            world.log().len() + world.relationships().len() + world.entities().len(),
            "time_travel: O(changes + R + E·layers) — WorldDiff inversion",
            None,
        ),

        // D3: Structural counterfactual replay — O(D×descendants + R).
        Query::CounterfactualReplay { remove_changes } => single_scan_plan(
            world.log().len() + world.relationships().len(),
            &format!(
                "counterfactual_replay: O(descendants of {} roots + R={})",
                remove_changes.len(),
                world.relationships().len()
            ),
            None,
        ),

        // D4: Entity-level causality — O(layers + ancestors).
        Query::EntityTransitionCause { .. } => single_scan_plan(
            world.entities().len(),
            "entity_transition_cause: O(entity layers)",
            None,
        ),
        Query::EntityUpstreamTransitions { .. } => single_scan_plan(
            world.log().len() + world.entities().len(),
            "entity_upstream_transitions: O(ChangeLog ancestors + entity scan)",
            None,
        ),
        Query::EntityLayersInRange { entity_id, .. } => single_scan_plan(
            world
                .entities()
                .get(*entity_id)
                .map_or(0, |e| e.layers.len()),
            "entity_layers_in_range: O(entity layer count)",
            None,
        ),

        Query::AllBetweenness { limit } => single_traversal_plan(
            world.relationships().len(),
            "Brandes betweenness centrality over all loci",
            *limit,
        ),
        Query::AllCloseness { limit } => single_traversal_plan(
            world.loci().len(),
            "Harmonic closeness centrality over all loci",
            *limit,
        ),
        Query::AllConstraints { limit } => single_traversal_plan(
            world.loci().len(),
            "Burt structural constraint over all loci",
            *limit,
        ),
        Query::PageRank { limit, .. } => {
            single_traversal_plan(world.loci().len(), "PageRank over all loci", *limit)
        }
        Query::Louvain | Query::LouvainWithResolution(_) => single_traversal_plan(
            world.relationships().len(),
            "Louvain community detection",
            None,
        ),
        _ => {
            // Single-operation queries — one step, no planning needed.
            QueryPlan {
                estimated_candidates_initial: 1,
                estimated_output: 1,
                steps: vec![PlanStep {
                    description: format!("{:?}", std::mem::discriminant(query)),
                    cost_class: CostClass::Scan,
                    estimated_output: 1,
                }],
            }
        }
    }
}

// ─── Internal: relationship predicate planning ────────────────────────────────

/// Classified information about how to execute a `FindRelationships` query.
///
/// - `seed_locus`: if `Some`, seed from the adjacency index for this locus.
/// - `predicates_ordered`: remaining predicates sorted cheapest-first.
pub(crate) struct RelPlan<'a> {
    /// Locus to use as the adjacency-index seed, if any.
    pub seed_locus: Option<SeedKind>,
    /// Remaining predicates in priority order (cheapest first).
    pub predicates_ordered: Vec<&'a RelationshipPredicate>,
}

/// Which adjacency index variant to use as the seed.
pub(crate) enum SeedKind {
    From(LocusId),
    To(LocusId),
    Touching(LocusId),
    /// O(1) — `From(a) + To(b) + OfKind(k)` combined into a single
    /// `(EndpointKey, kind)` hash lookup via the `by_key` index.
    DirectLookup {
        from: LocusId,
        to: LocusId,
        kind: RelationshipKindId,
    },
    /// O(min_degree) — `From(a) + To(b)` without an `OfKind` constraint.
    /// Uses `relationships_between(a, b)` which scans the shorter adjacency list.
    Between {
        a: LocusId,
        b: LocusId,
    },
}

/// Plan predicate execution order for `FindRelationships`.
///
/// Scans all predicates in two passes to find the best possible seed:
///
/// | Priority | Seed | Cost |
/// |----------|------|------|
/// | 1st | `From(a) + To(b) + OfKind(k)` → `DirectLookup` | O(1) |
/// | 2nd | `From(a) + To(b)` → `Between` | O(min_degree) |
/// | 3rd | `From(a)` / `To(b)` / `Touching(c)` | O(degree) |
/// | fallback | full scan | O(edges) |
///
/// Remaining predicates (those not consumed by the seed) are sorted
/// cheapest-first for efficient sequential filtering.
pub(crate) fn plan_rel_predicates(predicates: &[RelationshipPredicate]) -> RelPlan<'_> {
    // Pass 1: find the first occurrence index of each seed-eligible predicate.
    let mut first_from: Option<(usize, LocusId)> = None;
    let mut first_to: Option<(usize, LocusId)> = None;
    let mut first_touching: Option<(usize, LocusId)> = None;
    let mut first_of_kind: Option<(usize, RelationshipKindId)> = None;

    for (i, pred) in predicates.iter().enumerate() {
        match pred {
            RelationshipPredicate::From(id) if first_from.is_none() => {
                first_from = Some((i, *id));
            }
            RelationshipPredicate::To(id) if first_to.is_none() => {
                first_to = Some((i, *id));
            }
            RelationshipPredicate::Touching(id) if first_touching.is_none() => {
                first_touching = Some((i, *id));
            }
            RelationshipPredicate::OfKind(k) if first_of_kind.is_none() => {
                first_of_kind = Some((i, *k));
            }
            _ => {}
        }
    }

    // Pass 2: choose the best seed and record which predicate indices it consumed.
    let mut consumed: Vec<usize> = Vec::with_capacity(3);

    let seed: Option<SeedKind> = match (first_from, first_to, first_of_kind) {
        // Best: From + To + OfKind → single O(1) hash lookup.
        (Some((fi, from)), Some((ti, to)), Some((ki, kind))) => {
            consumed.extend_from_slice(&[fi, ti, ki]);
            Some(SeedKind::DirectLookup { from, to, kind })
        }
        // Good: From + To without kind → relationships_between, O(min_degree).
        (Some((fi, a)), Some((ti, b)), _) => {
            consumed.extend_from_slice(&[fi, ti]);
            Some(SeedKind::Between { a, b })
        }
        // Standard adjacency seeds: first qualifying predicate wins.
        (Some((fi, id)), _, _) => {
            consumed.push(fi);
            Some(SeedKind::From(id))
        }
        (_, Some((ti, id)), _) => {
            consumed.push(ti);
            Some(SeedKind::To(id))
        }
        _ => match first_touching {
            Some((ti, id)) => {
                consumed.push(ti);
                Some(SeedKind::Touching(id))
            }
            None => None,
        },
    };

    // Pass 3: remaining predicates (not consumed by seed), sorted cheapest-first.
    let mut rest: Vec<(&RelationshipPredicate, u8)> = predicates
        .iter()
        .enumerate()
        .filter(|(i, _)| !consumed.contains(i))
        .map(|(_, pred)| (pred, rel_pred_priority(pred)))
        .collect();
    rest.sort_unstable_by_key(|(_, p)| *p);

    RelPlan {
        seed_locus: seed,
        predicates_ordered: rest.into_iter().map(|(p, _)| p).collect(),
    }
}

/// Static priority within a cost tier (lower = apply first).
fn rel_pred_priority(pred: &RelationshipPredicate) -> u8 {
    match pred {
        // Already-used index predicates that appear as secondary filters:
        RelationshipPredicate::From(_)
        | RelationshipPredicate::To(_)
        | RelationshipPredicate::Touching(_) => 5,
        // Cheap kind filter — hash lookup
        RelationshipPredicate::OfKind(_) => 10,
        // Numeric filters — arithmetic comparison
        RelationshipPredicate::ActivityAbove(_)
        | RelationshipPredicate::StrengthAbove(_)
        | RelationshipPredicate::SlotAbove { .. }
        | RelationshipPredicate::MinChangeCount(_) => 20,
        // Range / age filters — two comparisons
        RelationshipPredicate::CreatedInRange { .. } | RelationshipPredicate::OlderThan { .. } => {
            30
        }
    }
}

// ─── Internal: locus predicate planning ──────────────────────────────────────

/// Sorted locus predicates — cheapest first.
pub(crate) fn plan_loci_predicates(predicates: &[LocusPredicate]) -> Vec<&LocusPredicate> {
    let mut ranked: Vec<(&LocusPredicate, u8)> = predicates
        .iter()
        .map(|p| (p, locus_pred_priority(p)))
        .collect();
    ranked.sort_unstable_by_key(|(_, p)| *p);
    ranked.into_iter().map(|(p, _)| p).collect()
}

fn locus_pred_priority(pred: &LocusPredicate) -> u8 {
    match pred {
        LocusPredicate::OfKind(_) => 10,
        LocusPredicate::StateAbove { .. } | LocusPredicate::StateBelow { .. } => 20,
        LocusPredicate::F64PropertyAbove { .. } | LocusPredicate::StrPropertyEq { .. } => 30,
        LocusPredicate::MinDegree(_) => 40, // degree() call
        // Active BFS — cheaper than full BFS because dormant edges are pruned.
        LocusPredicate::ReachableFromActive { .. }
        | LocusPredicate::DownstreamOfActive { .. }
        | LocusPredicate::UpstreamOfActive { .. } => 85,
        // BFS over full graph — most expensive.
        LocusPredicate::ReachableFrom { .. }
        | LocusPredicate::DownstreamOf { .. }
        | LocusPredicate::UpstreamOf { .. } => 90,
    }
}

// ─── explain helpers ─────────────────────────────────────────────────────────

fn explain_find_relationships(
    world: &World,
    predicates: &[RelationshipPredicate],
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    let total_rels = world.relationships().len();
    let plan = plan_rel_predicates(predicates);
    let mut steps: Vec<PlanStep> = Vec::new();

    let initial = match &plan.seed_locus {
        Some(SeedKind::DirectLookup { from, to, kind }) => {
            // Perform the actual lookup to get a precise 0-or-1 estimate.
            use graph_core::EndpointKey;
            let key = EndpointKey::Directed(*from, *to);
            let found = world.relationships().lookup(&key, *kind).is_some() as usize;
            steps.push(PlanStep {
                description: format!(
                    "direct lookup: From({}) To({}) OfKind({}) → O(1)",
                    from.0, to.0, kind.0
                ),
                cost_class: CostClass::Index,
                estimated_output: found,
            });
            found
        }
        Some(SeedKind::Between { a, b }) => {
            let est = world.degree(*a).min(world.degree(*b));
            steps.push(PlanStep {
                description: format!(
                    "between-loci scan: ({}, {}) → O(min_degree={})",
                    a.0, b.0, est
                ),
                cost_class: CostClass::Index,
                estimated_output: est,
            });
            est
        }
        Some(SeedKind::From(id)) => {
            let degree = world.degree(*id);
            steps.push(PlanStep {
                description: format!("adjacency seed: From({}) → O(degree={})", id.0, degree),
                cost_class: CostClass::Index,
                estimated_output: degree,
            });
            degree
        }
        Some(SeedKind::To(id)) => {
            let degree = world.degree(*id);
            steps.push(PlanStep {
                description: format!("adjacency seed: To({}) → O(degree={})", id.0, degree),
                cost_class: CostClass::Index,
                estimated_output: degree,
            });
            degree
        }
        Some(SeedKind::Touching(id)) => {
            let degree = world.degree(*id);
            steps.push(PlanStep {
                description: format!("adjacency seed: Touching({}) → O(degree={})", id.0, degree),
                cost_class: CostClass::Index,
                estimated_output: degree,
            });
            degree
        }
        None => {
            steps.push(PlanStep {
                description: format!("full relationship scan ({} edges)", total_rels),
                cost_class: CostClass::Scan,
                estimated_output: total_rels,
            });
            total_rels
        }
    };

    let mut est = initial;
    for pred in &plan.predicates_ordered {
        let (desc, cost, selectivity) = rel_pred_desc(pred);
        est = ((est as f32 * selectivity) as usize).max(0);
        steps.push(PlanStep {
            description: desc,
            cost_class: cost,
            estimated_output: est,
        });
    }
    if has_sort {
        steps.push(PlanStep {
            description: "sort".to_string(),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    if let Some(n) = limit {
        est = est.min(n);
        steps.push(PlanStep {
            description: format!("limit {}", n),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: initial,
        estimated_output: est,
    }
}

fn explain_find_loci(
    world: &World,
    predicates: &[LocusPredicate],
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    let total = world.loci().len();
    let mut steps: Vec<PlanStep> = vec![PlanStep {
        description: format!("full locus scan ({} loci)", total),
        cost_class: CostClass::Scan,
        estimated_output: total,
    }];
    let ordered = plan_loci_predicates(predicates);
    let mut est = total;
    for pred in ordered {
        let (desc, cost, selectivity) = locus_pred_desc(pred);
        est = ((est as f32 * selectivity) as usize).max(0);
        steps.push(PlanStep {
            description: desc,
            cost_class: cost,
            estimated_output: est,
        });
    }
    if has_sort {
        steps.push(PlanStep {
            description: "sort".to_string(),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    if let Some(n) = limit {
        est = est.min(n);
        steps.push(PlanStep {
            description: format!("limit {}", n),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: total,
        estimated_output: est,
    }
}

fn explain_find_entities(
    world: &World,
    pred_count: usize,
    has_sort: bool,
    limit: Option<usize>,
) -> QueryPlan {
    let total = world.entities().active().count();
    let mut est = total;
    let mut steps: Vec<PlanStep> = vec![PlanStep {
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
    if has_sort {
        steps.push(PlanStep {
            description: "sort".to_string(),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    if let Some(n) = limit {
        est = est.min(n);
        steps.push(PlanStep {
            description: format!("limit {}", n),
            cost_class: CostClass::Scan,
            estimated_output: est,
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: total,
        estimated_output: est,
    }
}

fn single_scan_plan(initial: usize, desc: &str, limit: Option<usize>) -> QueryPlan {
    let est = limit.map_or(initial, |n| n.min(initial));
    QueryPlan {
        steps: vec![PlanStep {
            description: desc.to_string(),
            cost_class: CostClass::Scan,
            estimated_output: est,
        }],
        estimated_candidates_initial: initial,
        estimated_output: est,
    }
}

fn single_traversal_plan(initial: usize, desc: &str, limit: Option<usize>) -> QueryPlan {
    let est = limit.unwrap_or(initial);
    let mut steps = vec![
        PlanStep {
            description: desc.to_string(),
            cost_class: CostClass::Traversal,
            estimated_output: initial,
        },
        PlanStep {
            description: "sort descending".to_string(),
            cost_class: CostClass::Scan,
            estimated_output: initial,
        },
    ];
    if let Some(n) = limit {
        steps.push(PlanStep {
            description: format!("limit {}", n),
            cost_class: CostClass::Scan,
            estimated_output: n.min(initial),
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: initial,
        estimated_output: est,
    }
}

// ─── Predicate descriptions for explain ──────────────────────────────────────

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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_api::{LocusSort, Query, RelSort};
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipId, RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn three_node_world() -> World {
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        for id in 0u64..3 {
            w.insert_locus(Locus::new(
                graph_core::LocusId(id),
                LocusKindId(1),
                StateVector::from_slice(&[0.5]),
            ));
        }
        for (from, to) in [(0u64, 1u64), (1, 2), (0, 2)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::directed(graph_core::LocusId(from), graph_core::LocusId(to)),
                state: StateVector::from_slice(&[0.5, 0.5]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn plan_rel_predicates_promotes_direct_lookup_when_from_to_kind_present() {
        let preds = vec![
            RelationshipPredicate::ActivityAbove(0.3),
            RelationshipPredicate::From(graph_core::LocusId(1)),
            RelationshipPredicate::To(graph_core::LocusId(2)),
            RelationshipPredicate::OfKind(InfluenceKindId(7)),
        ];
        let plan = plan_rel_predicates(&preds);
        assert!(
            matches!(plan.seed_locus, Some(SeedKind::DirectLookup { from, to, kind })
                if from.0 == 1 && to.0 == 2 && kind.0 == 7),
            "expected DirectLookup, got {:?}",
            plan.seed_locus.as_ref().map(|_| "other")
        );
        // Only ActivityAbove remains
        assert_eq!(plan.predicates_ordered.len(), 1);
        assert!(matches!(
            plan.predicates_ordered[0],
            RelationshipPredicate::ActivityAbove(_)
        ));
    }

    #[test]
    fn plan_rel_predicates_promotes_between_when_from_to_no_kind() {
        let preds = vec![
            RelationshipPredicate::From(graph_core::LocusId(3)),
            RelationshipPredicate::To(graph_core::LocusId(4)),
            RelationshipPredicate::ActivityAbove(0.5),
        ];
        let plan = plan_rel_predicates(&preds);
        assert!(
            matches!(plan.seed_locus, Some(SeedKind::Between { a, b })
                if a.0 == 3 && b.0 == 4),
            "expected Between"
        );
        // ActivityAbove remains; no OfKind was present
        assert_eq!(plan.predicates_ordered.len(), 1);
        assert!(matches!(
            plan.predicates_ordered[0],
            RelationshipPredicate::ActivityAbove(_)
        ));
    }

    #[test]
    fn explain_direct_lookup_reports_index_cost_and_exact_output() {
        let w = three_node_world();
        // three_node_world has edges (0→1), (1→2), (0→2) all of kind 1.
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(graph_core::LocusId(0)),
                RelationshipPredicate::To(graph_core::LocusId(1)),
                RelationshipPredicate::OfKind(InfluenceKindId(1)),
            ],
            sort_by: None,
            limit: None,
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.steps[0].cost_class, CostClass::Index);
        // DirectLookup does the actual lookup → exactly 1 result.
        assert_eq!(plan.steps[0].estimated_output, 1);
        assert_eq!(plan.estimated_output, 1);
    }

    #[test]
    fn explain_direct_lookup_reports_zero_for_nonexistent_edge() {
        let w = three_node_world();
        // No edge 1→0 in three_node_world.
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(graph_core::LocusId(1)),
                RelationshipPredicate::To(graph_core::LocusId(0)),
                RelationshipPredicate::OfKind(InfluenceKindId(1)),
            ],
            sort_by: None,
            limit: None,
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.steps[0].estimated_output, 0);
    }

    #[test]
    fn explain_full_scan_when_no_index_pred() {
        let w = three_node_world();
        let q = Query::FindRelationships {
            predicates: vec![RelationshipPredicate::ActivityAbove(0.3)],
            sort_by: None,
            limit: None,
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.estimated_candidates_initial, 3);
        assert_eq!(plan.steps[0].cost_class, CostClass::Scan); // full scan
    }

    #[test]
    fn explain_index_seed_when_from_pred_present() {
        let w = three_node_world();
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(graph_core::LocusId(0)),
                RelationshipPredicate::ActivityAbove(0.3),
            ],
            sort_by: Some(RelSort::ActivityDesc),
            limit: Some(1),
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.steps[0].cost_class, CostClass::Index);
        // LocusId(0) has 2 outgoing edges
        assert_eq!(plan.steps[0].estimated_output, 2);
        // final output capped by limit
        assert_eq!(plan.estimated_output, 1);
    }

    #[test]
    fn plan_rel_predicates_promotes_from_to_seed() {
        let preds = vec![
            RelationshipPredicate::ActivityAbove(0.5),
            RelationshipPredicate::From(graph_core::LocusId(1)),
            RelationshipPredicate::OfKind(InfluenceKindId(1)),
        ];
        let plan = plan_rel_predicates(&preds);
        // From(1) promoted to seed
        assert!(matches!(plan.seed_locus, Some(SeedKind::From(id)) if id.0 == 1));
        // remaining predicates: OfKind before ActivityAbove
        assert_eq!(plan.predicates_ordered.len(), 2);
        assert!(matches!(
            plan.predicates_ordered[0],
            RelationshipPredicate::OfKind(_)
        ));
        assert!(matches!(
            plan.predicates_ordered[1],
            RelationshipPredicate::ActivityAbove(_)
        ));
    }

    #[test]
    fn plan_loci_predicates_traversal_last() {
        use graph_core::LocusId;
        let preds = vec![
            LocusPredicate::ReachableFrom {
                start: LocusId(0),
                depth: 2,
            },
            LocusPredicate::OfKind(LocusKindId(1)),
            LocusPredicate::StateAbove { slot: 0, min: 0.3 },
        ];
        let ordered = plan_loci_predicates(&preds);
        // OfKind first (10), StateAbove second (20), ReachableFrom last (90)
        assert!(matches!(ordered[0], LocusPredicate::OfKind(_)));
        assert!(matches!(ordered[1], LocusPredicate::StateAbove { .. }));
        assert!(matches!(ordered[2], LocusPredicate::ReachableFrom { .. }));
    }

    #[test]
    fn explain_find_loci_structure() {
        let w = three_node_world();
        let q = Query::FindLoci {
            predicates: vec![LocusPredicate::OfKind(LocusKindId(1))],
            sort_by: Some(LocusSort::StateDesc(0)),
            limit: Some(2),
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.estimated_candidates_initial, 3);
        // should have: scan step + filter step + sort step + limit step
        assert!(plan.steps.len() >= 3);
        // estimated_output is a heuristic — just verify it's capped by limit
        assert!(plan.estimated_output <= 2);
    }

    #[test]
    fn no_index_pred_when_only_of_kind() {
        let preds = vec![RelationshipPredicate::OfKind(InfluenceKindId(1))];
        let plan = plan_rel_predicates(&preds);
        assert!(plan.seed_locus.is_none());
        assert_eq!(plan.predicates_ordered.len(), 1);
    }
}
