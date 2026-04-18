use std::collections::HashSet;

use graph_core::{
    BatchId, Change, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId,
    LocusKindId, LocusProgram, ProposedChange, StateVector,
};
use graph_engine::{
    InfluenceKindConfig, InfluenceKindRegistry, PairPredictionObjective, PlasticityConfig,
    PlasticityLearners, PlasticityObservation, Simulation,
};
use graph_world::World;

struct ForwarderProgram {
    downstream: LocusId,
}

impl LocusProgram for ForwarderProgram {
    fn process(
        &self,
        _locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if !incoming.iter().all(|c| c.predecessors.is_empty()) {
            return Vec::new();
        }
        let after = incoming[0].after.clone();
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.downstream),
            InfluenceKindId(1),
            after,
        )]
    }
}

struct SinkProgram;

impl LocusProgram for SinkProgram {
    fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        Vec::new()
    }
}

fn two_locus_simulation(learning_rate: f32) -> Simulation {
    let mut world = World::new();
    world.insert_locus(Locus::new(
        LocusId(1),
        LocusKindId(1),
        StateVector::zeros(1),
    ));
    world.insert_locus(Locus::new(
        LocusId(2),
        LocusKindId(2),
        StateVector::zeros(1),
    ));

    let mut loci = graph_engine::LocusKindRegistry::new();
    loci.insert(
        LocusKindId(1),
        Box::new(ForwarderProgram {
            downstream: LocusId(2),
        }),
    );
    loci.insert(LocusKindId(2), Box::new(SinkProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        InfluenceKindId(1),
        InfluenceKindConfig::new("hebb").with_plasticity(PlasticityConfig {
            learning_rate,
            weight_decay: 1.0,
            max_weight: f32::MAX,
        }),
    );

    Simulation::new(world, loci, influences)
}

#[test]
fn pair_prediction_objective_ranks_by_strength() {
    let mut world = World::new();
    world.insert_locus(Locus::new(
        LocusId(1),
        LocusKindId(1),
        StateVector::zeros(1),
    ));
    world.insert_locus(Locus::new(
        LocusId(2),
        LocusKindId(1),
        StateVector::zeros(1),
    ));
    world.insert_locus(Locus::new(
        LocusId(3),
        LocusKindId(1),
        StateVector::zeros(1),
    ));

    world.add_relationship(
        Endpoints::Symmetric {
            a: LocusId(1),
            b: LocusId(2),
        },
        InfluenceKindId(7),
        StateVector::from_slice(&[0.2, 0.9]),
    );
    world.add_relationship(
        Endpoints::Symmetric {
            a: LocusId(1),
            b: LocusId(3),
        },
        InfluenceKindId(7),
        StateVector::from_slice(&[0.7, 0.1]),
    );

    let objective = PairPredictionObjective {
        kind: InfluenceKindId(7),
        k: 2,
        horizon_batches: 3,
        recall_weight: 0.5,
    };

    let ranked = objective.rank(&world);
    assert_eq!(ranked.entries[0].pair, (LocusId(1), LocusId(2)));
    assert!((ranked.entries[0].strength - 1.1).abs() < 1e-6);
    assert_eq!(ranked.entries[1].pair, (LocusId(1), LocusId(3)));

    let observed_pairs = HashSet::from([(LocusId(1), LocusId(2))]);
    let score = objective.score(
        &[(LocusId(1), LocusId(2))],
        std::iter::empty(),
        &observed_pairs,
    );
    assert_eq!(score.k_used, 1);
    assert!((score.precision_at_k - 1.0).abs() < 1e-6);
    assert!((score.recall - 1.0).abs() < 1e-6);
}

#[test]
fn simulation_applies_plasticity_learning_scale() {
    let mut baseline = two_locus_simulation(0.1);
    baseline.step(vec![ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[2.0]),
    )]);
    let baseline_weight = baseline
        .world()
        .relationships()
        .iter()
        .next()
        .unwrap()
        .weight();

    let mut scaled = two_locus_simulation(0.1);
    let mut learners = PlasticityLearners::new();
    learners.register(InfluenceKindId(1));
    learners.observe(
        InfluenceKindId(1),
        PlasticityObservation {
            loss: 0.05,
            precision_at_k: 1.0,
            recall: 1.0,
            k_used: 20,
            window_batches: 3,
        },
    );
    scaled.set_plasticity_learners(learners);
    let obs = scaled.step(vec![ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[2.0]),
    )]);
    let scaled_weight = scaled
        .world()
        .relationships()
        .iter()
        .next()
        .unwrap()
        .weight();

    assert!(
        scaled.current_plasticity_scale(InfluenceKindId(1)) > 1.0,
        "expected learner scale > 1, got {}",
        scaled.current_plasticity_scale(InfluenceKindId(1))
    );
    assert!(
        scaled_weight > baseline_weight,
        "expected scaled learning rate to increase weight beyond baseline: {scaled_weight} <= {baseline_weight}"
    );
    assert_eq!(
        obs.plasticity_scales.get(&InfluenceKindId(1)).copied(),
        Some(scaled.current_plasticity_scale(InfluenceKindId(1)))
    );
}

#[test]
fn poor_plasticity_observation_reduces_learning_scale() {
    let mut learners = PlasticityLearners::new();
    learners.register(InfluenceKindId(9));
    let before = learners.current(InfluenceKindId(9));
    learners.observe(
        InfluenceKindId(9),
        PlasticityObservation {
            loss: 1.0,
            precision_at_k: 0.0,
            recall: 0.0,
            k_used: 20,
            window_batches: 4,
        },
    );
    let after = learners.current(InfluenceKindId(9));
    assert!(
        after < before,
        "expected poor observation to reduce learning scale: {after} !< {before}"
    );
}

#[test]
fn plasticity_learner_smooths_single_observation_shocks() {
    let mut learners = PlasticityLearners::new();
    let kind = InfluenceKindId(11);
    learners.register(kind);

    let strong = PlasticityObservation {
        loss: 0.05,
        precision_at_k: 1.0,
        recall: 1.0,
        k_used: 20,
        window_batches: 4,
    };
    let weak = PlasticityObservation {
        loss: 0.05,
        precision_at_k: 1.0,
        recall: 1.0,
        k_used: 2,
        window_batches: 1,
    };

    learners.observe(kind, weak);
    let weak_after = learners.current(kind);
    learners.reset(kind);

    learners.observe(kind, strong);
    let strong_after = learners.current(kind);

    assert!(
        weak_after > 1.0,
        "expected even weak positive evidence to increase scale slightly: {weak_after}"
    );
    assert!(
        strong_after > weak_after,
        "expected stronger evidence to move scale more than weak evidence: {strong_after} <= {weak_after}"
    );
}

#[test]
fn simulation_can_evaluate_and_observe_pair_prediction() {
    let mut sim = Simulation::new(
        World::new(),
        graph_engine::LocusKindRegistry::new(),
        InfluenceKindRegistry::new(),
    );
    {
        let mut world = sim.world_mut();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        world.insert_locus(Locus::new(
            LocusId(2),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        world.insert_locus(Locus::new(
            LocusId(3),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        world.add_relationship(
            Endpoints::Symmetric {
                a: LocusId(1),
                b: LocusId(2),
            },
            InfluenceKindId(7),
            StateVector::from_slice(&[0.8, 0.4]),
        );
        world.add_relationship(
            Endpoints::Symmetric {
                a: LocusId(1),
                b: LocusId(3),
            },
            InfluenceKindId(7),
            StateVector::from_slice(&[0.2, 0.1]),
        );
    }

    let mut learners = PlasticityLearners::new();
    learners.register(InfluenceKindId(7));
    sim.set_plasticity_learners(learners);

    let objective = PairPredictionObjective {
        kind: InfluenceKindId(7),
        k: 1,
        horizon_batches: 2,
        recall_weight: 0.5,
    };
    let future_events = vec![vec![vec![1, 2]]];

    let before = sim.current_plasticity_scale(InfluenceKindId(7));
    let obs = sim
        .evaluate_and_observe_pair_prediction(&objective, &future_events, BatchId(1), BatchId(2))
        .expect("learners installed");
    let after = sim.current_plasticity_scale(InfluenceKindId(7));

    assert_eq!(obs.k_used, 1);
    assert!((obs.precision_at_k - 1.0).abs() < 1e-6);
    assert!(
        after > before,
        "expected learner scale to increase on low loss"
    );
}
