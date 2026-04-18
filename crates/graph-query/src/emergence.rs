//! Entity emergence diagnostics — coherence stability analysis and Ψ estimation.

use graph_core::{CompressedTransition, CompressionLevel, LayerTransition};
use graph_world::World;

mod api;
mod leave_one_out;
mod math;
mod psi;
mod render;
mod report;
mod series;
mod synergy;
mod types;

const DEFAULT_MIN_ACTIVITY_THRESHOLD: f32 = 0.1;

pub use api::{
    coherence_autocorrelation, coherence_dense_series, coherence_dense_series_with_decay,
    coherence_stable_series, emergence_report, emergence_report_synergy,
    emergence_report_synergy_with_decay, emergence_report_with_decay, psi_scalar,
    psi_scalar_with_decay, psi_synergy, psi_synergy_leave_one_out,
    psi_synergy_leave_one_out_with_decay, psi_synergy_with_decay,
};
pub use types::{
    DecayRates, DropResult, EmergenceEntry, EmergenceReport, EmergenceSynergyEntry,
    EmergenceSynergyReport, LeaveOneOutResult, PsiResult, PsiSynergyResult, SynergyPair,
    UnmeasuredEntry, UnmeasuredReason,
};

pub(super) fn coherence_dense_series_inner(
    world: &World,
    entity_id: graph_core::EntityId,
    decay_rates: Option<&DecayRates>,
) -> Vec<(graph_core::BatchId, f32)> {
    series::coherence_dense_series_inner(world, entity_id, decay_rates)
}

pub(super) fn rel_weight_at(
    batch: graph_core::BatchId,
    rel_id: graph_core::RelationshipId,
    world: &World,
) -> f64 {
    series::rel_weight_at(batch, rel_id, world)
}

pub(super) fn gaussian_mi_from_series(a: &[f64], b: &[f64]) -> Option<f64> {
    math::gaussian_mi_from_series(a, b)
}

#[cfg(test)]
fn solve_linear_system(a: Vec<Vec<f64>>, b: Vec<f64>) -> Option<Vec<f64>> {
    math::solve_linear_system(a, b)
}

pub(super) fn gaussian_joint_mi(x: &[Vec<f64>], y: &[f64]) -> Option<f64> {
    math::gaussian_joint_mi(x, y)
}

fn is_lifecycle_transition(layer: &graph_core::EntityLayer) -> bool {
    match &layer.compression {
        CompressionLevel::Full => matches!(
            layer.transition,
            LayerTransition::Born
                | LayerTransition::BecameDormant
                | LayerTransition::Revived
                | LayerTransition::Split { .. }
                | LayerTransition::Merged { .. }
        ),
        CompressionLevel::Compressed {
            transition_kind, ..
        }
        | CompressionLevel::Skeleton {
            transition_kind, ..
        } => matches!(
            transition_kind,
            CompressedTransition::Born
                | CompressedTransition::BecameDormant
                | CompressedTransition::Revived
                | CompressedTransition::Split
                | CompressedTransition::Merged
        ),
    }
}

fn layer_coherence(layer: &graph_core::EntityLayer) -> Option<f32> {
    match &layer.compression {
        CompressionLevel::Full => layer.snapshot.as_ref().map(|snapshot| snapshot.coherence),
        CompressionLevel::Compressed { coherence, .. }
        | CompressionLevel::Skeleton { coherence, .. } => Some(*coherence),
    }
}

fn pearson_autocorr(series: &[f32], lag: usize) -> Option<f64> {
    let n = series.len();
    if n < lag + 2 {
        return None;
    }

    let mean = series.iter().map(|&x| x as f64).sum::<f64>() / n as f64;
    let variance = series
        .iter()
        .map(|&x| {
            let delta = x as f64 - mean;
            delta * delta
        })
        .sum::<f64>();

    if variance < f64::EPSILON {
        return None;
    }

    let cross = series[..n - lag]
        .iter()
        .zip(series[lag..].iter())
        .map(|(&a, &b)| (a as f64 - mean) * (b as f64 - mean))
        .sum::<f64>();

    Some(cross / variance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, Endpoints, Entity, EntityId, EntitySnapshot,
        InfluenceKindId, LayerTransition, LocusId, RelationshipId, RelationshipKindId, StateVector,
    };
    use graph_world::World;

    fn snapshot(coherence: f32) -> EntitySnapshot {
        EntitySnapshot {
            members: vec![LocusId(1), LocusId(2)],
            member_relationships: vec![],
            coherence,
        }
    }

    fn snapshot_with_rels(coherence: f32, rels: Vec<RelationshipId>) -> EntitySnapshot {
        EntitySnapshot {
            members: vec![LocusId(1), LocusId(2)],
            member_relationships: rels,
            coherence,
        }
    }

    fn born_entity(id: u64, batch: u64, coherence: f32) -> Entity {
        Entity::born(EntityId(id), BatchId(batch), snapshot(coherence))
    }

    fn add_rel_with_changes(world: &mut World, changes: &[(u64, f32, f32)]) -> RelationshipId {
        let kind: RelationshipKindId = InfluenceKindId(0);
        let rel_id = world.add_relationship(
            Endpoints::symmetric(LocusId(1), LocusId(2)),
            kind,
            StateVector::from_slice(&[0.0, 0.0]),
        );
        let mut next_change_id = world.log().len() as u64;
        for &(batch, activity, weight) in changes {
            let change = Change {
                id: ChangeId(next_change_id),
                subject: ChangeSubject::Relationship(rel_id),
                kind: InfluenceKindId(0),
                predecessors: vec![],
                before: StateVector::from_slice(&[0.0, 0.0]),
                after: StateVector::from_slice(&[activity, weight]),
                batch: BatchId(batch),
                wall_time: None,
                metadata: None,
            };
            world.append_change(change);
            next_change_id += 1;
        }
        rel_id
    }

    #[test]
    fn stable_series_empty_for_unknown_entity() {
        let world = World::new();
        assert!(coherence_stable_series(&world, EntityId(99)).is_empty());
    }

    #[test]
    fn stable_series_excludes_lifecycle_transitions() {
        let mut world = World::new();
        let mut e = born_entity(0, 1, 0.5);
        e.deposit(
            BatchId(2),
            snapshot(0.6),
            LayerTransition::CoherenceShift { from: 0.5, to: 0.6 },
        );
        e.deposit(BatchId(3), snapshot(0.3), LayerTransition::BecameDormant);
        e.deposit(
            BatchId(4),
            snapshot(0.7),
            LayerTransition::CoherenceShift { from: 0.3, to: 0.7 },
        );
        e.deposit(
            BatchId(5),
            snapshot(0.8),
            LayerTransition::CoherenceShift { from: 0.7, to: 0.8 },
        );
        world.entities_mut().insert(e);

        let series = coherence_stable_series(&world, EntityId(0));
        assert_eq!(series.len(), 2);
        assert_eq!(series[0], (BatchId(4), 0.7));
        assert_eq!(series[1], (BatchId(5), 0.8));
    }

    #[test]
    fn stable_series_born_layer_excluded() {
        let mut world = World::new();
        let mut e = born_entity(0, 1, 0.5);
        e.deposit(
            BatchId(2),
            snapshot(0.6),
            LayerTransition::CoherenceShift { from: 0.5, to: 0.6 },
        );
        e.deposit(
            BatchId(3),
            snapshot(0.7),
            LayerTransition::MembershipDelta {
                added: vec![LocusId(3)],
                removed: vec![],
            },
        );
        world.entities_mut().insert(e);

        let series = coherence_stable_series(&world, EntityId(0));
        assert_eq!(series.len(), 2);
    }

    #[test]
    fn autocorr_lag1_linear_trend_is_positive() {
        let series: Vec<f32> = (0..10).map(|i| i as f32 * 0.1).collect();
        let r = pearson_autocorr(&series, 1).unwrap();
        assert!(r > 0.6, "got {r}");
    }

    #[test]
    fn autocorr_lag1_alternating_is_negative() {
        let series: Vec<f32> = (0..10)
            .map(|i| if i % 2 == 0 { 0.9 } else { 0.1 })
            .collect();
        let r = pearson_autocorr(&series, 1).unwrap();
        assert!(r < -0.85, "got {r}");
    }

    #[test]
    fn autocorr_returns_none_for_short_series() {
        let series = vec![0.5, 0.6];
        assert!(pearson_autocorr(&series, 1).is_none());
        assert!(pearson_autocorr(&[], 0).is_none());
    }

    #[test]
    fn autocorr_returns_none_for_zero_variance() {
        let series = vec![0.5f32; 10];
        assert!(pearson_autocorr(&series, 1).is_none());
    }

    #[test]
    fn emergence_report_empty_world_has_no_entries() {
        let world = World::new();
        let r = emergence_report(&world);
        assert_eq!(r.n_entities, 0);
        assert_eq!(r.n_measured(), 0);
        assert!(r.emergent.is_empty());
        assert!(r.spurious.is_empty());
        assert!(r.unmeasured.is_empty());
        assert!(r.emergent_fraction().is_none());
    }

    #[test]
    fn emergence_report_dormant_entity_is_unmeasured() {
        let mut world = World::new();
        let mut e = born_entity(7, 1, 0.5);
        e.deposit(BatchId(2), snapshot(0.3), LayerTransition::BecameDormant);
        e.status = graph_core::EntityStatus::Dormant;
        world.entities_mut().insert(e);

        let r = emergence_report(&world);
        assert_eq!(r.n_entities, 1);
        assert_eq!(r.n_measured(), 0);
        assert_eq!(r.unmeasured.len(), 1);
        assert_eq!(r.unmeasured[0].entity, EntityId(7));
        assert_eq!(r.unmeasured[0].reason, UnmeasuredReason::Dormant);
    }

    #[test]
    fn emergence_report_short_window_unmeasured() {
        let mut world = World::new();
        world.entities_mut().insert(born_entity(3, 1, 0.5));

        let r = emergence_report(&world);
        assert_eq!(r.n_entities, 1);
        assert_eq!(r.unmeasured.len(), 1);
        match &r.unmeasured[0].reason {
            UnmeasuredReason::InsufficientStableWindow { layer_count } => {
                assert_eq!(*layer_count, 0);
            }
            other => panic!("expected InsufficientStableWindow, got {other:?}"),
        }
    }

    #[test]
    fn emergence_report_missing_member_history_flagged_no_component_history() {
        let mut world = World::new();
        let rel_id =
            add_rel_with_changes(&mut world, &[(2, 0.3, 0.5), (3, 0.4, 0.5), (4, 0.5, 0.5)]);
        let e = Entity::born(
            EntityId(4),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel_id]),
        );
        world.entities_mut().insert(e);

        let r = emergence_report(&world);
        assert_eq!(r.unmeasured.len(), 1);
        assert_eq!(r.unmeasured[0].reason, UnmeasuredReason::NoComponentHistory);
    }

    #[test]
    fn dense_series_empty_for_unknown_entity() {
        let world = World::new();
        assert!(coherence_dense_series(&world, EntityId(99)).is_empty());
    }

    #[test]
    fn dense_series_empty_when_no_member_rels() {
        let mut world = World::new();
        world.entities_mut().insert(born_entity(0, 1, 0.5));
        assert!(coherence_dense_series(&world, EntityId(0)).is_empty());
    }

    #[test]
    fn dense_series_samples_at_change_batches() {
        let mut world = World::new();
        let rel_id =
            add_rel_with_changes(&mut world, &[(2, 0.3, 0.1), (3, 0.5, 0.2), (5, 0.7, 0.3)]);
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel_id]),
        );
        world.entities_mut().insert(e);

        let series = coherence_dense_series(&world, EntityId(0));
        assert_eq!(series.len(), 3);
        assert_eq!(series[0].0, BatchId(2));
        assert_eq!(series[1].0, BatchId(3));
        assert_eq!(series[2].0, BatchId(5));

        let expected_density = (1.0f32 / ((2.0f32 * 3.0f32.ln()) / 2.0)).min(1.0);
        let coh_at = |activity: f32| activity * expected_density;
        for (got, expected_activity) in series.iter().zip([0.3, 0.5, 0.7].iter()) {
            let expected = coh_at(*expected_activity);
            assert!((got.1 - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn dense_series_respects_lifecycle_window() {
        let mut world = World::new();
        let rel_id =
            add_rel_with_changes(&mut world, &[(2, 0.3, 0.1), (4, 0.5, 0.2), (6, 0.7, 0.3)]);
        let mut e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel_id]),
        );
        e.deposit(
            BatchId(3),
            snapshot_with_rels(0.3, vec![rel_id]),
            LayerTransition::BecameDormant,
        );
        world.entities_mut().insert(e);

        let series = coherence_dense_series(&world, EntityId(0));
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].0, BatchId(4));
        assert_eq!(series[1].0, BatchId(6));
    }

    #[test]
    fn emergence_report_measures_entity_with_rich_dense_series() {
        let mut world = World::new();
        let rel_id = add_rel_with_changes(
            &mut world,
            &[(2, 0.3, 0.1), (3, 0.5, 0.2), (4, 0.4, 0.4), (5, 0.7, 0.5)],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel_id]),
        );
        world.entities_mut().insert(e);

        let psi = psi_scalar(&world, EntityId(0)).expect("rich history should produce Ψ");
        assert_eq!(psi.n_samples, 3);
        assert_eq!(psi.n_components, 1);

        let r = emergence_report(&world);
        assert_eq!(r.n_measured(), 1);
    }

    #[test]
    fn emergence_report_mixes_measured_and_unmeasured() {
        let mut world = World::new();
        let mut dormant = born_entity(1, 1, 0.5);
        dormant.status = graph_core::EntityStatus::Dormant;
        world.entities_mut().insert(dormant);
        world.entities_mut().insert(born_entity(2, 1, 0.5));

        let r = emergence_report(&world);
        assert_eq!(r.n_entities, 2);
        assert_eq!(r.unmeasured.len(), 2);
        assert_eq!(r.n_measured(), 0);

        let dormant_found = r
            .unmeasured
            .iter()
            .any(|u| u.entity == EntityId(1) && u.reason == UnmeasuredReason::Dormant);
        assert!(dormant_found);

        let short_found = r.unmeasured.iter().any(|u| {
            u.entity == EntityId(2)
                && matches!(u.reason, UnmeasuredReason::InsufficientStableWindow { .. })
        });
        assert!(short_found);
    }

    #[test]
    fn solve_linear_system_recovers_known_solution() {
        let a = vec![vec![2.0, 3.0], vec![5.0, 4.0]];
        let b = vec![8.0, 13.0];
        let x = solve_linear_system(a, b).expect("non-singular");
        assert!((x[0] - 1.0).abs() < 1e-9);
        assert!((x[1] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn solve_linear_system_detects_singular_matrix() {
        let a = vec![vec![1.0, 2.0], vec![2.0, 4.0]];
        let b = vec![3.0, 6.0];
        assert!(solve_linear_system(a, b).is_none());
    }

    #[test]
    fn joint_mi_equal_to_individual_when_other_predictor_uncorrelated() {
        let y: Vec<f64> = vec![1.0, 2.2, 2.9, 4.1, 5.3, 6.0, 7.2, 7.9, 9.1, 10.3];
        let x1: Vec<f64> = vec![1.2, 1.9, 3.1, 3.8, 5.0, 6.3, 6.9, 8.1, 9.0, 10.1];
        let x2: Vec<f64> = vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];

        let i_x1 = gaussian_mi_from_series(&x1, &y).unwrap();
        let i_joint = gaussian_joint_mi(&[x1, x2], &y).unwrap();
        assert!(i_joint + 1e-6 >= i_x1);
        assert!(i_joint - i_x1 < 0.5);
    }

    #[test]
    fn joint_mi_exceeds_sum_when_predictors_uncorrelated() {
        let x1: Vec<f64> = vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
        let x2: Vec<f64> = vec![1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0];
        let noise: Vec<f64> = vec![
            0.05, -0.03, 0.02, -0.04, 0.01, 0.03, -0.02, 0.04, -0.01, 0.02,
        ];
        let y: Vec<f64> = x1
            .iter()
            .zip(x2.iter())
            .zip(noise.iter())
            .map(|((a, b), n)| a + b + n)
            .collect();

        let i_x1 = gaussian_mi_from_series(&x1, &y).unwrap();
        let i_x2 = gaussian_mi_from_series(&x2, &y).unwrap();
        let i_joint = gaussian_joint_mi(&[x1, x2], &y).unwrap();
        assert!(i_joint > i_x1 + i_x2);
    }

    #[test]
    fn joint_mi_near_individual_when_predictors_identical() {
        let y: Vec<f64> = (0..20).map(|i| (i as f64).sin()).collect();
        let x1: Vec<f64> = y.iter().map(|v| v + 0.01).collect();
        let x2 = x1.clone();
        assert!(gaussian_joint_mi(&[x1, x2], &y).is_none());
    }

    #[test]
    fn psi_synergy_returns_some_on_rich_history() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.20),
                (4, 0.4, 0.35),
                (5, 0.6, 0.48),
                (6, 0.7, 0.55),
                (7, 0.8, 0.72),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.30),
                (3, 0.3, 0.32),
                (4, 0.5, 0.48),
                (5, 0.4, 0.72),
                (6, 0.6, 0.78),
                (7, 0.7, 0.86),
                (8, 0.85, 0.95),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2]),
        );
        world.entities_mut().insert(e);

        let synergy = psi_synergy(&world, EntityId(0)).expect("rich history should produce Ψ");
        assert_eq!(synergy.n_components, 2);
        assert_eq!(synergy.top_pairs.len(), 1);
        let pair = &synergy.top_pairs[0];
        let unique_a = pair.mi_a - pair.redundancy;
        let unique_b = pair.mi_b - pair.redundancy;
        let reconstructed = pair.redundancy + unique_a + unique_b + pair.synergy;
        assert!((reconstructed - pair.joint_mi).abs() < 1e-9);
        assert!(synergy.psi_corrected + 1e-9 >= synergy.psi_naive);
        assert_eq!(synergy.n_pairs_evaluated, 1);
        assert!((synergy.total_pair_synergy - pair.synergy).abs() < 1e-12);
        assert!((synergy.mean_pair_synergy - pair.synergy).abs() < 1e-12);
        assert!((synergy.psi_pair_top3 - synergy.psi_corrected).abs() < 1e-9);
    }

    #[test]
    fn pair_synergy_aggregate_non_negative_on_synergistic_components() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.22),
                (4, 0.4, 0.35),
                (5, 0.6, 0.51),
                (6, 0.7, 0.60),
                (7, 0.8, 0.73),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.32),
                (3, 0.3, 0.31),
                (4, 0.5, 0.49),
                (5, 0.4, 0.71),
                (6, 0.6, 0.79),
                (7, 0.7, 0.85),
                (8, 0.85, 0.97),
            ],
        );
        let rel3 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.5, 0.15),
                (3, 0.4, 0.28),
                (4, 0.6, 0.42),
                (5, 0.5, 0.55),
                (6, 0.7, 0.68),
                (7, 0.6, 0.82),
                (8, 0.8, 0.91),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2, rel3]),
        );
        world.entities_mut().insert(e);

        let synergy = psi_synergy(&world, EntityId(0)).unwrap();
        assert_eq!(synergy.n_components, 3);
        assert_eq!(synergy.n_pairs_evaluated, 3);
        let top_sum: f64 = synergy.top_pairs.iter().map(|p| p.synergy).sum();
        assert!((synergy.total_pair_synergy - top_sum).abs() < 1e-9);
    }

    #[test]
    fn psi_pair_top3_equals_psi_corrected_for_two_components() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.20),
                (4, 0.4, 0.35),
                (5, 0.6, 0.48),
                (6, 0.7, 0.55),
                (7, 0.8, 0.72),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.30),
                (3, 0.3, 0.32),
                (4, 0.5, 0.48),
                (5, 0.4, 0.72),
                (6, 0.6, 0.78),
                (7, 0.7, 0.86),
                (8, 0.85, 0.95),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2]),
        );
        world.entities_mut().insert(e);

        let s = psi_synergy(&world, EntityId(0)).unwrap();
        assert!((s.psi_pair_top3 - s.psi_corrected).abs() < 1e-9);
    }

    #[test]
    fn psi_pair_top3_uses_joint_not_synergy_sum() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.22),
                (4, 0.4, 0.35),
                (5, 0.6, 0.51),
                (6, 0.7, 0.60),
                (7, 0.8, 0.73),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.32),
                (3, 0.3, 0.31),
                (4, 0.5, 0.49),
                (5, 0.4, 0.71),
                (6, 0.6, 0.79),
                (7, 0.7, 0.85),
                (8, 0.85, 0.97),
            ],
        );
        let rel3 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.5, 0.15),
                (3, 0.4, 0.28),
                (4, 0.6, 0.42),
                (5, 0.5, 0.55),
                (6, 0.7, 0.68),
                (7, 0.6, 0.82),
                (8, 0.8, 0.91),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2, rel3]),
        );
        world.entities_mut().insert(e);

        let s = psi_synergy(&world, EntityId(0)).unwrap();
        let joint_sum: f64 = s.top_pairs.iter().take(3).map(|p| p.joint_mi).sum();
        let synergy_sum: f64 = s.top_pairs.iter().take(3).map(|p| p.synergy).sum();
        let expected = s.i_self - joint_sum;
        assert!((s.psi_pair_top3 - expected).abs() < 1e-9);
        assert!(joint_sum - synergy_sum > 1e-9);
    }

    #[test]
    fn leave_one_out_none_with_two_components() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.22),
                (4, 0.4, 0.35),
                (5, 0.6, 0.51),
                (6, 0.7, 0.60),
                (7, 0.8, 0.73),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.32),
                (3, 0.3, 0.31),
                (4, 0.5, 0.49),
                (5, 0.4, 0.71),
                (6, 0.6, 0.79),
                (7, 0.7, 0.85),
                (8, 0.85, 0.97),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2]),
        );
        world.entities_mut().insert(e);

        assert!(psi_synergy_leave_one_out(&world, EntityId(0)).is_none());
    }

    #[test]
    fn leave_one_out_produces_one_drop_per_component() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.22),
                (4, 0.4, 0.35),
                (5, 0.6, 0.51),
                (6, 0.7, 0.60),
                (7, 0.8, 0.73),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.32),
                (3, 0.3, 0.31),
                (4, 0.5, 0.49),
                (5, 0.4, 0.71),
                (6, 0.6, 0.79),
                (7, 0.7, 0.85),
                (8, 0.85, 0.97),
            ],
        );
        let rel3 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.5, 0.15),
                (3, 0.4, 0.28),
                (4, 0.6, 0.42),
                (5, 0.5, 0.55),
                (6, 0.7, 0.68),
                (7, 0.6, 0.82),
                (8, 0.8, 0.91),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2, rel3]),
        );
        world.entities_mut().insert(e);

        let loo = psi_synergy_leave_one_out(&world, EntityId(0)).unwrap();
        assert_eq!(loo.baseline.n_components, 3);
        assert_eq!(loo.drops.len(), 3);
        let mut dropped_ids: Vec<_> = loo.drops.iter().map(|d| d.dropped).collect();
        dropped_ids.sort_by_key(|r| r.0);
        dropped_ids.dedup();
        assert_eq!(dropped_ids.len(), 3);
    }

    #[test]
    fn leave_one_out_delta_invariants() {
        let mut world = World::new();
        let rel1 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.3, 0.10),
                (3, 0.5, 0.22),
                (4, 0.4, 0.35),
                (5, 0.6, 0.51),
                (6, 0.7, 0.60),
                (7, 0.8, 0.73),
                (8, 0.9, 0.88),
            ],
        );
        let rel2 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.4, 0.32),
                (3, 0.3, 0.31),
                (4, 0.5, 0.49),
                (5, 0.4, 0.71),
                (6, 0.6, 0.79),
                (7, 0.7, 0.85),
                (8, 0.85, 0.97),
            ],
        );
        let rel3 = add_rel_with_changes(
            &mut world,
            &[
                (2, 0.5, 0.15),
                (3, 0.4, 0.28),
                (4, 0.6, 0.42),
                (5, 0.5, 0.55),
                (6, 0.7, 0.68),
                (7, 0.6, 0.82),
                (8, 0.8, 0.91),
            ],
        );
        let e = Entity::born(
            EntityId(0),
            BatchId(1),
            snapshot_with_rels(0.5, vec![rel1, rel2, rel3]),
        );
        world.entities_mut().insert(e);

        let loo = psi_synergy_leave_one_out(&world, EntityId(0)).unwrap();
        for d in &loo.drops {
            let eps = 1e-9;
            assert!(
                ((loo.baseline.psi_corrected - d.psi_corrected) - d.psi_corrected_delta).abs()
                    < eps
            );
            assert!(
                ((loo.baseline.psi_pair_top3 - d.psi_pair_top3) - d.psi_pair_top3_delta).abs()
                    < eps
            );
        }
    }

    #[test]
    fn psi_synergy_none_with_single_component() {
        let mut world = World::new();
        let rel = add_rel_with_changes(
            &mut world,
            &[(2, 0.3, 0.1), (3, 0.5, 0.2), (4, 0.4, 0.35), (5, 0.6, 0.5)],
        );
        let e = Entity::born(EntityId(0), BatchId(1), snapshot_with_rels(0.5, vec![rel]));
        world.entities_mut().insert(e);
        assert!(psi_synergy(&world, EntityId(0)).is_none());
    }

    #[test]
    fn rel_activity_at_without_decay_returns_last_change_after() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(2, 0.8, 0.1), (5, 0.6, 0.2)]);
        let activity = series::rel_activity_at(BatchId(7), rel, &world, None);
        assert!((activity - 0.6).abs() < 1e-6, "got {}", activity);
    }

    #[test]
    fn rel_activity_at_with_decay_applies_rate_over_gap() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(5, 0.8, 0.1)]);
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 0.5);
        let activity = series::rel_activity_at(BatchId(8), rel, &world, Some(&rates));
        assert!((activity - 0.1).abs() < 1e-6, "got {}", activity);
    }

    #[test]
    fn rel_activity_at_decay_identity_when_rate_is_one() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(5, 0.8, 0.1)]);
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 1.0);
        assert_eq!(
            series::rel_activity_at(BatchId(8), rel, &world, Some(&rates)),
            series::rel_activity_at(BatchId(8), rel, &world, None),
        );
    }

    #[test]
    fn rel_activity_at_no_decay_for_gap_zero() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(5, 0.8, 0.1)]);
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 0.5);
        let activity = series::rel_activity_at(BatchId(5), rel, &world, Some(&rates));
        assert!((activity - 0.8).abs() < 1e-6, "got {}", activity);
    }

    #[test]
    fn coherence_dense_series_with_decay_differs_from_no_decay() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(2, 0.8, 0.1), (4, 0.6, 0.2), (6, 0.4, 0.3)]);
        let e = Entity::born(EntityId(0), BatchId(1), snapshot_with_rels(0.5, vec![rel]));
        world.entities_mut().insert(e);

        let no_decay = coherence_dense_series(&world, EntityId(0));
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 0.8);
        let with_decay = coherence_dense_series_with_decay(&world, EntityId(0), &rates);

        assert_eq!(no_decay.len(), with_decay.len());
        for ((b1, _), (b2, _)) in no_decay.iter().zip(with_decay.iter()) {
            assert_eq!(b1, b2);
        }
        assert!((no_decay[0].1 - with_decay[0].1).abs() < 1e-6);
        for ((_, c1), (_, c2)) in no_decay.iter().zip(with_decay.iter()) {
            assert!((c1 - c2).abs() < 1e-6);
        }
    }

    #[test]
    fn emergence_report_synergy_mirrors_shape_of_plain_report() {
        let mut world = World::new();
        let mut dormant = born_entity(1, 1, 0.5);
        dormant.status = graph_core::EntityStatus::Dormant;
        world.entities_mut().insert(dormant);
        world.entities_mut().insert(born_entity(2, 1, 0.5));

        let r = emergence_report_synergy(&world);
        assert_eq!(r.n_entities, 2);
        assert_eq!(r.n_measured(), 0);
        assert_eq!(r.unmeasured.len(), 2);
    }
}
