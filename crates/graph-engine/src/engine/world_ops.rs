//! On-demand world mutations: entity recognition, cohere extraction,
//! relationship decay flush, entity weathering, and change log trim.
//!
//! These are free functions rather than `Engine` methods because they are
//! stateless with respect to the engine (they read no engine config). Keeping
//! them separate from the batch loop also lets `Simulation` call them
//! directly without routing through a method receiver.

mod decay;
mod entity_mutation;
mod plasticity;

use graph_core::{BatchId, EntityWeatheringPolicy, RelationshipId, WorldEvent};
use graph_world::World;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::engine::batch::PlasticityObs;
use crate::registry::InfluenceKindRegistry;

pub(crate) use plasticity::HebbianEffect;

pub(crate) fn flush_relationship_decay(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
) -> (usize, Vec<WorldEvent>) {
    decay::flush_relationship_decay(world, influence_registry)
}

pub(crate) fn extract_cohere(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    perspective: &dyn CoherePerspective,
) {
    let _ = flush_relationship_decay(world, influence_registry);
    let batch = world.current_batch();
    let store = world.coheres_mut();
    let mut counter = store.mint_id().0;
    let coheres = perspective.cluster(world.entities(), world.relationships(), &mut || {
        let id = graph_core::CohereId(counter);
        counter += 1;
        id
    });
    world
        .coheres_mut()
        .update_at(perspective.name(), coheres, batch);
}

/// Max fixpoint passes per `recognize_entities` call. The HEP-PH diagnosis
/// (`docs/hep-ph-finding.md §4c`) showed the perspective's single-pass
/// proposal set is not self-consistent on accumulative citation graphs —
/// a second pass over the post-apply world collapsed up to 40% of newly
/// Born entities via Merge. Iterating to fixpoint closes that gap.
/// Empirically 2–3 passes converge; 8 is a safe guard against pathology.
pub(crate) const RECOGNIZE_MAX_FIXPOINT_PASSES: usize = 8;

/// Counter used by diagnostics/tests to confirm fixpoint convergence.
pub(crate) static RECOGNIZE_LAST_PASSES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Number of proposals still pending when the fixpoint loop hit its pass
/// cap. 0 means convergence; non-zero means the loop aborted before the
/// perspective's proposal set became empty — the world is potentially
/// mid-consolidation and the next tick will see residue. Surfaces through
/// `last_recognize_unconverged_proposals()` so callers can warn.
pub(crate) static RECOGNIZE_LAST_UNCONVERGED_PROPOSALS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn recognize_entities(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    perspective: &dyn EmergencePerspective,
) -> Vec<WorldEvent> {
    #[cfg(feature = "perf-timing")]
    let t0 = std::time::Instant::now();

    let (_, mut events) = flush_relationship_decay(world, influence_registry);

    #[cfg(feature = "perf-timing")]
    let flush_us = t0.elapsed().as_micros();
    #[cfg(feature = "perf-timing")]
    let t1 = std::time::Instant::now();

    let mut passes = 0usize;
    let mut last_proposals_count = 0usize;
    for _ in 0..RECOGNIZE_MAX_FIXPOINT_PASSES {
        let batch = world.current_batch();
        let proposals = perspective.recognize(world.relationships(), world.entities(), batch);
        passes += 1;
        last_proposals_count = proposals.len();
        if proposals.is_empty() {
            break;
        }
        let proposal_events = entity_mutation::apply_proposals(world, proposals, batch);
        events.extend(proposal_events);
    }
    RECOGNIZE_LAST_PASSES.store(passes, std::sync::atomic::Ordering::Relaxed);
    RECOGNIZE_LAST_UNCONVERGED_PROPOSALS.store(
        if passes == RECOGNIZE_MAX_FIXPOINT_PASSES && last_proposals_count > 0 {
            last_proposals_count
        } else {
            0
        },
        std::sync::atomic::Ordering::Relaxed,
    );

    #[cfg(feature = "perf-timing")]
    let recognize_us = t1.elapsed().as_micros();

    #[cfg(feature = "perf-timing")]
    {
        let rel_count = world.relationships().iter().count();
        eprintln!(
            "[perf-timing] recognize_entities: flush={flush_us}µs recognize+apply={recognize_us}µs passes={passes} rels={rel_count}"
        );
    }

    events
}

/// Number of perspective.recognize passes the last `recognize_entities` call
/// took. 1 = converged on the first pass (fully idempotent). >1 indicates
/// the perspective emitted proposals on a pass that were reversed by a
/// subsequent pass — see HEP-PH Finding 5 §4c.
pub fn last_recognize_passes() -> usize {
    RECOGNIZE_LAST_PASSES.load(std::sync::atomic::Ordering::Relaxed)
}

/// If the last `recognize_entities` call exhausted its fixpoint pass cap
/// without emptying the proposal queue, returns the count still pending.
/// 0 means convergence.
pub fn last_recognize_unconverged_proposals() -> usize {
    RECOGNIZE_LAST_UNCONVERGED_PROPOSALS.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn weather_entities(world: &mut World, policy: &dyn EntityWeatheringPolicy) {
    entity_mutation::weather_entities(world, policy);
}

pub(crate) fn trim_change_log(world: &mut World, retention_batches: u64) -> usize {
    let current = world.current_batch().0;
    let retain_from = graph_core::BatchId(current.saturating_sub(retention_batches));
    world.log_mut().trim_before_batch(retain_from)
}

pub(crate) fn apply_demotion_policies(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: BatchId,
) -> Vec<RelationshipId> {
    decay::apply_demotion_policies(world, influence_registry, current_batch)
}

pub(crate) fn compute_hebbian_effects(
    world: &World,
    obs: &[PlasticityObs],
    influence_registry: &InfluenceKindRegistry,
) -> Vec<HebbianEffect> {
    plasticity::compute_hebbian_effects(world, obs, influence_registry)
}

pub(crate) fn apply_hebbian_effects(world: &mut World, effects: &[HebbianEffect]) {
    plasticity::apply_hebbian_effects(world, effects);
}

pub(crate) fn apply_interaction_effects(
    world: &mut World,
    batch_kind_touches: &rustc_hash::FxHashMap<
        graph_core::EndpointKey,
        (
            rustc_hash::FxHashSet<graph_core::InfluenceKindId>,
            rustc_hash::FxHashSet<RelationshipId>,
        ),
    >,
    influence_registry: &InfluenceKindRegistry,
) {
    plasticity::apply_interaction_effects(world, batch_kind_touches, influence_registry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship,
        RelationshipLineage, StateVector,
    };
    use graph_world::World;
    use smallvec::SmallVec;

    use crate::engine::batch::{PlasticityObs, TimingOrder};
    use crate::registry::{
        DemotionPolicy, InfluenceKindConfig, InfluenceKindRegistry, PlasticityConfig,
    };

    fn make_world_with_rel(activity: f32, weight: f32) -> (World, RelationshipId) {
        let mut world = World::default();
        world.loci_mut().insert(Locus::new(
            LocusId(1),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        world.loci_mut().insert(Locus::new(
            LocusId(2),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        let rel = Relationship {
            id: RelationshipId(0),
            kind: InfluenceKindId(1),
            endpoints: Endpoints::symmetric(LocusId(1), LocusId(2)),
            state: StateVector::from_slice(&[activity, weight]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: SmallVec::new(),
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        };
        let rel_id = rel.id;
        world.relationships_mut().insert(rel);
        (world, rel_id)
    }

    fn apply_test_hebbian(
        world: &mut World,
        obs: &[PlasticityObs],
        influence_registry: &InfluenceKindRegistry,
    ) -> Vec<HebbianEffect> {
        let effects = compute_hebbian_effects(world, obs, influence_registry);
        apply_hebbian_effects(world, &effects);
        effects
    }

    fn make_rel(
        world: &mut World,
        id: u64,
        kind: u64,
        activity: f32,
        last_decayed_batch: u64,
    ) -> RelationshipId {
        let rel_id = RelationshipId(id);
        world.loci_mut().insert(Locus::new(
            LocusId(id * 2),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        world.loci_mut().insert(Locus::new(
            LocusId(id * 2 + 1),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        let mut rel = Relationship {
            id: rel_id,
            kind: InfluenceKindId(kind),
            endpoints: Endpoints::symmetric(LocusId(id * 2), LocusId(id * 2 + 1)),
            state: StateVector::from_slice(&[activity, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: SmallVec::new(),
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        };
        rel.last_decayed_batch = last_decayed_batch;
        world.relationships_mut().insert(rel);
        rel_id
    }

    #[test]
    fn demotion_activity_floor_evicts_low_activity() {
        let mut world = World::default();
        let low = make_rel(&mut world, 10, 1, 0.05, 0);
        let high = make_rel(&mut world, 11, 1, 0.9, 0);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::ActivityFloor(0.1)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(5));

        assert!(world.relationships().get(low).is_none());
        assert!(world.relationships().get(high).is_some());
    }

    #[test]
    fn demotion_idle_batches_evicts_stale_rels() {
        let mut world = World::default();
        let stale = make_rel(&mut world, 20, 2, 1.0, 0);
        let fresh = make_rel(&mut world, 21, 2, 1.0, 9);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::IdleBatches(5)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(10));

        assert!(world.relationships().get(stale).is_none());
        assert!(world.relationships().get(fresh).is_some());
    }

    #[test]
    fn demotion_lru_capacity_keeps_most_recent() {
        let mut world = World::default();
        let oldest = make_rel(&mut world, 30, 3, 1.0, 1);
        let middle = make_rel(&mut world, 31, 3, 1.0, 5);
        let newest = make_rel(&mut world, 32, 3, 1.0, 9);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(3),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::LruCapacity(2)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(10));

        assert!(world.relationships().get(oldest).is_none());
        assert!(world.relationships().get(middle).is_some());
        assert!(world.relationships().get(newest).is_some());
    }

    #[test]
    fn demotion_lru_below_capacity_evicts_nothing() {
        let mut world = World::default();
        let a = make_rel(&mut world, 40, 4, 1.0, 5);
        let b = make_rel(&mut world, 41, 4, 1.0, 8);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(4),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::LruCapacity(5)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(10));

        assert!(world.relationships().get(a).is_some());
        assert!(world.relationships().get(b).is_some());
    }

    #[test]
    fn hebbian_delta_weight_formula() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 0.0);
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("t").with_plasticity(PlasticityConfig {
                learning_rate: 0.1,
                weight_decay: 1.0,
                max_weight: 10.0,
                ..Default::default()
            }),
        );

        let _ = apply_test_hebbian(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 0.8,
                post: 0.9,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );

        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!((w - 0.072).abs() < 1e-6, "weight = {w}");
    }

    #[test]
    fn hebbian_clamps_at_max_weight() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 9.9);
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("t").with_plasticity(PlasticityConfig {
                learning_rate: 1.0,
                weight_decay: 1.0,
                max_weight: 10.0,
                ..Default::default()
            }),
        );

        apply_test_hebbian(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 1.0,
                post: 1.0,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );

        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!((w - 10.0).abs() < 1e-6, "weight = {w}");
    }

    #[test]
    fn hebbian_no_op_when_learning_rate_zero() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 3.0);
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));

        apply_test_hebbian(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 1.0,
                post: 1.0,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );

        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!((w - 3.0).abs() < 1e-6);
    }

    fn make_stdp_reg() -> InfluenceKindRegistry {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("stdp").with_plasticity(PlasticityConfig {
                learning_rate: 0.1,
                weight_decay: 1.0,
                max_weight: 10.0,
                ..Default::default()
            }),
        );
        reg
    }

    #[test]
    fn stdp_causal_strengthens() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 0.0);
        let reg = make_stdp_reg();
        apply_test_hebbian(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 0.8,
                post: 0.9,
                timing: TimingOrder::PreFirst,
                post_locus: LocusId(2),
            }],
            &reg,
        );
        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!(w > 0.0);
        assert!((w - 0.072).abs() < 1e-6, "expected 0.072, got {w}");
    }

    #[test]
    fn stdp_simultaneous_strengthens() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 0.0);
        let reg = make_stdp_reg();
        apply_test_hebbian(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 0.8,
                post: 0.9,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );
        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!(w > 0.0);
        assert!((w - 0.072).abs() < 1e-6, "expected 0.072, got {w}");
    }
}
