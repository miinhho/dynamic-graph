use super::{
    DropResult, PsiSynergyResult, SynergyPair, World, gaussian_joint_mi, gaussian_mi_from_series,
    rel_weight_at,
};
use graph_core::{BatchId, RelationshipId};

const MAX_TOP_PAIRS: usize = 5;

pub(super) struct ComponentSeries {
    pub(super) x_series: Vec<Vec<f64>>,
    pub(super) rel_ids: Vec<RelationshipId>,
    pub(super) individual_mi: Vec<f64>,
}

pub(super) struct PairSummary {
    pub(super) top_pairs: Vec<SynergyPair>,
    pub(super) n_pairs_evaluated: usize,
    pub(super) total_pair_synergy: f64,
    pub(super) total_pair_redundancy: f64,
    pub(super) mean_pair_synergy: f64,
    pub(super) top3_joint_sum: f64,
}

pub(super) fn build_component_series(
    world: &World,
    member_relationships: &[RelationshipId],
    window: &[(BatchId, f32)],
    v_t1: &[f64],
) -> ComponentSeries {
    let mut x_series = Vec::with_capacity(member_relationships.len());
    let mut rel_ids = Vec::with_capacity(member_relationships.len());
    let mut individual_mi = Vec::with_capacity(member_relationships.len());

    for rel_id in member_relationships {
        let series: Vec<f64> = window[..window.len() - 1]
            .iter()
            .map(|(batch, _)| rel_weight_at(*batch, *rel_id, world))
            .collect();
        if let Some(mi) = gaussian_mi_from_series(&series, v_t1) {
            x_series.push(series);
            rel_ids.push(*rel_id);
            individual_mi.push(mi);
        }
    }

    ComponentSeries {
        x_series,
        rel_ids,
        individual_mi,
    }
}

pub(super) fn summarize_pairs(
    x_series: &[Vec<f64>],
    rel_ids: &[RelationshipId],
    individual_mi: &[f64],
    v_t1: &[f64],
) -> PairSummary {
    let mut pairs = pairwise_synergy_rows(x_series, rel_ids, individual_mi, v_t1);
    let n_pairs_evaluated = pairs.len();
    let total_pair_synergy: f64 = pairs.iter().map(|pair| pair.synergy).sum();
    let total_pair_redundancy: f64 = pairs.iter().map(|pair| pair.redundancy).sum();
    let mean_pair_synergy = if n_pairs_evaluated == 0 {
        0.0
    } else {
        total_pair_synergy / n_pairs_evaluated as f64
    };

    pairs.sort_by(|a, b| {
        b.synergy
            .partial_cmp(&a.synergy)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let top3_joint_sum = pairs.iter().take(3).map(|pair| pair.joint_mi).sum();
    let mut top_pairs = pairs;
    top_pairs.truncate(MAX_TOP_PAIRS);

    PairSummary {
        top_pairs,
        n_pairs_evaluated,
        total_pair_synergy,
        total_pair_redundancy,
        mean_pair_synergy,
        top3_joint_sum,
    }
}

pub(super) fn leave_one_out_drops(
    x_series: &[Vec<f64>],
    rel_ids: &[RelationshipId],
    individual_mi: &[f64],
    v_t1: &[f64],
    baseline: &PsiSynergyResult,
) -> Vec<DropResult> {
    let mut drops = Vec::with_capacity(x_series.len());

    for (drop_idx, rel_id) in rel_ids.iter().enumerate().take(x_series.len()) {
        let kept_series = kept_series_without(x_series, drop_idx);
        let psi_corrected = baseline.i_self
            - gaussian_joint_mi(&kept_series, v_t1).unwrap_or(baseline.i_joint_components);
        let psi_pair_top3 =
            baseline.i_self - top3_joint_sum_without(x_series, individual_mi, v_t1, drop_idx);

        drops.push(DropResult {
            dropped: *rel_id,
            psi_corrected,
            psi_pair_top3,
            psi_corrected_delta: baseline.psi_corrected - psi_corrected,
            psi_pair_top3_delta: baseline.psi_pair_top3 - psi_pair_top3,
        });
    }

    drops
}

fn pairwise_synergy_rows(
    x_series: &[Vec<f64>],
    rel_ids: &[RelationshipId],
    individual_mi: &[f64],
    v_t1: &[f64],
) -> Vec<SynergyPair> {
    let mut pairs = Vec::with_capacity(
        x_series
            .len()
            .saturating_mul(x_series.len().saturating_sub(1))
            / 2,
    );
    for i in 0..x_series.len() {
        for j in (i + 1)..x_series.len() {
            let Some(joint_mi) =
                gaussian_joint_mi(&[x_series[i].clone(), x_series[j].clone()], v_t1)
            else {
                continue;
            };
            let mi_a = individual_mi[i];
            let mi_b = individual_mi[j];
            let redundancy = mi_a.min(mi_b);
            pairs.push(SynergyPair {
                a: rel_ids[i],
                b: rel_ids[j],
                mi_a,
                mi_b,
                joint_mi,
                redundancy,
                synergy: joint_mi - mi_a - mi_b + redundancy,
            });
        }
    }
    pairs
}

fn kept_series_without(x_series: &[Vec<f64>], drop_idx: usize) -> Vec<Vec<f64>> {
    x_series
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != drop_idx)
        .map(|(_, series)| series.clone())
        .collect()
}

fn top3_joint_sum_without(
    x_series: &[Vec<f64>],
    individual_mi: &[f64],
    v_t1: &[f64],
    drop_idx: usize,
) -> f64 {
    let mut pair_scores = Vec::new();
    for i in 0..x_series.len() {
        if i == drop_idx {
            continue;
        }
        for j in (i + 1)..x_series.len() {
            if j == drop_idx {
                continue;
            }
            let Some(joint_mi) =
                gaussian_joint_mi(&[x_series[i].clone(), x_series[j].clone()], v_t1)
            else {
                continue;
            };
            let redundancy = individual_mi[i].min(individual_mi[j]);
            let synergy = joint_mi - individual_mi[i] - individual_mi[j] + redundancy;
            pair_scores.push((synergy, joint_mi));
        }
    }

    pair_scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    pair_scores
        .iter()
        .take(3)
        .map(|(_, joint_mi)| *joint_mi)
        .sum()
}
