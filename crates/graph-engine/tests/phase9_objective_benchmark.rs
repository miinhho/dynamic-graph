use graph_engine::PlasticityConfig;
use graph_testkit::sociopatterns::{
    RankSignal, SocioPatternsProfile, evaluate_next_block_prediction,
};

const TOP_KS: [usize; 3] = [20, 50, 100];
const SEED: u64 = 0x50c10_5ca77e5d;

#[test]
fn larger_profiles_retain_prediction_signal() {
    let cases = [
        (SocioPatternsProfile::medium(), 90usize, 30usize),
        (SocioPatternsProfile::school_scale(), 120usize, 60usize),
    ];

    for (profile, train_blocks, test_blocks) in cases {
        let eval = evaluate_next_block_prediction(
            profile,
            SEED,
            train_blocks,
            test_blocks,
            PlasticityConfig::default(),
            RankSignal::Activity,
            &TOP_KS,
        );
        let top20 = eval.metric_at(20).expect("k=20 metric");

        println!(
            "[{}] threshold={:.3} base_rate={:.4} candidates={} rels={} precision@20={:.3} lift={:.2} recall={:.3}",
            profile.name,
            eval.threshold,
            eval.base_rate,
            eval.candidate_count,
            eval.relationship_count,
            top20.precision,
            top20.lift,
            eval.recall,
        );

        assert!(
            top20.lift >= 1.25,
            "expected {} activity ranking to beat random by a useful margin; got lift {:.2}",
            profile.name,
            top20.lift
        );
    }
}

#[test]
fn activity_ranking_is_invariant_to_plasticity_learning_rate() {
    let profile = SocioPatternsProfile::medium();
    let no_learning = evaluate_next_block_prediction(
        profile,
        SEED,
        90,
        30,
        PlasticityConfig::default(),
        RankSignal::Activity,
        &TOP_KS,
    );
    let aggressive_learning = evaluate_next_block_prediction(
        profile,
        SEED,
        90,
        30,
        PlasticityConfig {
            learning_rate: 0.2,
            weight_decay: 0.995,
            max_weight: 5.0,
        },
        RankSignal::Activity,
        &TOP_KS,
    );

    assert_eq!(
        no_learning.top_pairs(50),
        aggressive_learning.top_pairs(50),
        "activity-ranked predictions changed under plasticity, which means Phase 9 assumptions changed"
    );

    let p0 = no_learning.metric_at(20).expect("k=20 without plasticity");
    let p1 = aggressive_learning
        .metric_at(20)
        .expect("k=20 with plasticity");
    assert!(
        (p0.precision - p1.precision).abs() < 1e-6,
        "activity precision moved from {:.6} to {:.6}",
        p0.precision,
        p1.precision
    );
}
