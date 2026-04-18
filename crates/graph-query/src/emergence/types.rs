use graph_core::{EntityId, RelationshipId, RelationshipKindId};
use rustc_hash::FxHashMap;

use super::render;

pub type DecayRates = FxHashMap<RelationshipKindId, f32>;

#[derive(Debug, Clone, PartialEq)]
pub struct PsiResult {
    pub psi: f64,
    pub i_self: f64,
    pub i_sum_components: f64,
    pub n_samples: usize,
    pub n_components: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynergyPair {
    pub a: RelationshipId,
    pub b: RelationshipId,
    pub mi_a: f64,
    pub mi_b: f64,
    pub joint_mi: f64,
    pub redundancy: f64,
    pub synergy: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PsiSynergyResult {
    pub i_self: f64,
    pub i_sum_components: f64,
    pub i_joint_components: f64,
    pub psi_naive: f64,
    pub psi_corrected: f64,
    pub top_pairs: Vec<SynergyPair>,
    pub n_samples: usize,
    pub n_components: usize,
    pub n_pairs_evaluated: usize,
    pub total_pair_synergy: f64,
    pub total_pair_redundancy: f64,
    pub mean_pair_synergy: f64,
    pub psi_pair_top3: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropResult {
    pub dropped: RelationshipId,
    pub psi_corrected: f64,
    pub psi_pair_top3: f64,
    pub psi_corrected_delta: f64,
    pub psi_pair_top3_delta: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeaveOneOutResult {
    pub entity: EntityId,
    pub baseline: PsiSynergyResult,
    pub drops: Vec<DropResult>,
}

impl LeaveOneOutResult {
    pub fn sign_flips_corrected(&self) -> usize {
        let baseline = self.baseline.psi_corrected;
        self.drops
            .iter()
            .filter(|drop| drop.psi_corrected.signum() != baseline.signum())
            .count()
    }

    pub fn sign_flips_pair_top3(&self) -> usize {
        let baseline = self.baseline.psi_pair_top3;
        self.drops
            .iter()
            .filter(|drop| drop.psi_pair_top3.signum() != baseline.signum())
            .count()
    }

    pub fn most_load_bearing_for_pair_top3(&self) -> Option<&DropResult> {
        self.drops.iter().max_by(|a, b| {
            a.psi_pair_top3_delta
                .partial_cmp(&b.psi_pair_top3_delta)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    pub fn render_markdown(&self) -> String {
        render::render_leave_one_out_markdown(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceEntry {
    pub entity: EntityId,
    pub psi: PsiResult,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceSynergyEntry {
    pub entity: EntityId,
    pub psi: PsiSynergyResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmeasuredReason {
    Dormant,
    InsufficientStableWindow { layer_count: usize },
    NoComponentHistory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmeasuredEntry {
    pub entity: EntityId,
    pub reason: UnmeasuredReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceReport {
    pub emergent: Vec<EmergenceEntry>,
    pub spurious: Vec<EmergenceEntry>,
    pub unmeasured: Vec<UnmeasuredEntry>,
    pub n_entities: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceSynergyReport {
    pub emergent: Vec<EmergenceSynergyEntry>,
    pub spurious: Vec<EmergenceSynergyEntry>,
    pub unmeasured: Vec<UnmeasuredEntry>,
    pub n_entities: usize,
}

impl EmergenceReport {
    pub fn n_measured(&self) -> usize {
        self.emergent.len() + self.spurious.len()
    }

    pub fn emergent_fraction(&self) -> Option<f64> {
        let n = self.n_measured();
        if n == 0 {
            None
        } else {
            Some(self.emergent.len() as f64 / n as f64)
        }
    }

    pub fn render_markdown(&self) -> String {
        render::render_emergence_report_markdown(self)
    }
}

impl EmergenceSynergyReport {
    pub fn n_measured(&self) -> usize {
        self.emergent.len() + self.spurious.len()
    }

    pub fn emergent_fraction(&self) -> Option<f64> {
        let n = self.n_measured();
        if n == 0 {
            None
        } else {
            Some(self.emergent.len() as f64 / n as f64)
        }
    }

    pub fn render_markdown(&self) -> String {
        render::render_synergy_report_markdown(self)
    }
}
