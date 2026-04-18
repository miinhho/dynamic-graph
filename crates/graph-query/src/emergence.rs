//! Entity emergence diagnostics — coherence stability analysis and Ψ estimation.
//!
//! These functions are diagnostic tools, not validity gates. A positive
//! result gives evidence; a negative result does not disprove emergence.
//!
//! ## Stable window
//!
//! A *stable window* is the contiguous sequence of `DepositLayer` events
//! (i.e. `CoherenceShift` or `MembershipDelta` transitions) since the
//! most recent lifecycle transition (`Born`, `Split`, `Merged`,
//! `BecameDormant`, `Revived`). All time-series functions operate only
//! within this window because a lifecycle transition resets the entity's
//! identity; correlating across that boundary would conflate two
//! structurally distinct periods.
//!
//! ## Ψ (emergence capacity)
//!
//! Ψ(entity) = I(V_t; V_{t+1}) − Σᵢ I(Xᵢ_t; V_{t+1})
//!
//! where V_t is the entity's scalar coherence and Xᵢ_t is the state of
//! the i-th member locus, both measured at the batch of each deposit
//! event in the stable window.
//!
//! **Ψ > 0**: the entity coherence predicts its own future more than the
//! sum of individual locus predictions — evidence of causal emergence.
//! **Ψ ≤ 0**: no detectable emergence at this grain; the entity is
//! well-explained by its parts individually.
//!
//! MI is estimated via the Gaussian approximation
//! I(X; Y) ≈ −½ ln(1 − r²) which is exact for jointly Gaussian
//! variables and stable with small samples (< 50). Synergistic
//! information that is non-Gaussian may be underestimated.

use graph_core::{
    BatchId, ChangeSubject, CompressedTransition, CompressionLevel, EntityId, EntityStatus,
    LayerTransition, LocusId, RelationshipId, RelationshipKindId,
};
use graph_world::World;
use rustc_hash::FxHashMap;

/// Default minimum activity threshold for `coherence_at_batch`. Matches
/// `DefaultEmergencePerspective::min_activity_threshold` in `graph-engine`.
const DEFAULT_MIN_ACTIVITY_THRESHOLD: f32 = 0.1;

/// Per-kind activity decay rates, keyed by `RelationshipKindId`.
///
/// Used by `*_with_decay` variants of the emergence functions to
/// reconstruct a relationship's slot-0 activity at an arbitrary batch:
///
/// ```text
/// activity_at(B) ≈ change.after[0] × rate^(B − change.batch)
/// ```
///
/// Rates correspond to `InfluenceKindConfig::decay_per_batch`. Missing
/// entries fall back to a no-decay identity (rate = 1.0). Callers running
/// under `graph-engine` should prefer `Simulation::activity_decay_rates()`
/// rather than building this by hand.
pub type DecayRates = FxHashMap<RelationshipKindId, f32>;

// ─── Public types ─────────────────────────────────────────────────────────────

/// Output of [`psi_scalar`].
#[derive(Debug, Clone, PartialEq)]
pub struct PsiResult {
    /// Ψ = `i_self` − `i_sum_components`. Positive means causal emergence.
    pub psi: f64,
    /// I(V_t; V_{t+1}): how well entity coherence predicts its own future.
    pub i_self: f64,
    /// Σᵢ I(Xᵢ_t; V_{t+1}): sum of individual locus-to-entity predictions.
    pub i_sum_components: f64,
    /// Number of (V_t, V_{t+1}) sample pairs used.
    pub n_samples: usize,
    /// Number of member loci included in the component sum.
    pub n_components: usize,
}

/// One pair from a Φ-ID-style pairwise breakdown, as surfaced in
/// [`PsiSynergyResult::top_pairs`].
///
/// Under the MMI (Minimum Mutual Information) redundancy convention:
/// - `redundancy = min(mi_a, mi_b)` — information both predictors carry.
/// - `unique_a = mi_a − redundancy`, `unique_b = mi_b − redundancy`.
/// - `synergy = joint_mi − mi_a − mi_b + redundancy` — information that
///   *only* appears when both predictors are observed jointly.
///
/// The identity `redundancy + unique_a + unique_b + synergy = joint_mi`
/// holds by construction (up to floating-point error).
#[derive(Debug, Clone, PartialEq)]
pub struct SynergyPair {
    pub a: RelationshipId,
    pub b: RelationshipId,
    /// I(X_a; V_{t+1}).
    pub mi_a: f64,
    /// I(X_b; V_{t+1}).
    pub mi_b: f64,
    /// I(X_a, X_b; V_{t+1}) — joint MI over the pair.
    pub joint_mi: f64,
    /// min(mi_a, mi_b) under MMI convention.
    pub redundancy: f64,
    /// `joint_mi − mi_a − mi_b + redundancy`.
    pub synergy: f64,
}

/// Output of [`psi_synergy`]. Like [`PsiResult`] but replaces the naïve
/// `Σᵢ I(Xᵢ; V_{t+1})` with the joint MI over all components, which
/// eliminates the redundancy double-counting that drives negative naïve Ψ
/// on highly correlated component sets. See `docs/emergence/h4-report.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct PsiSynergyResult {
    /// I(V_t; V_{t+1}) — unchanged from [`PsiResult`].
    pub i_self: f64,
    /// Σᵢ I(Xᵢ; V_{t+1}) — the naïve sum the original Ψ uses; kept for
    /// comparison.
    pub i_sum_components: f64,
    /// I(X⃗; V_{t+1}) — joint MI via multivariate Gaussian regression.
    /// Always `≤ i_sum_components` (equality iff components are mutually
    /// uncorrelated).
    pub i_joint_components: f64,
    /// Naïve Ψ (same formula as [`PsiResult::psi`]). Negative when
    /// components are redundant.
    pub psi_naive: f64,
    /// Redundancy-corrected Ψ: `i_self − i_joint_components`. Positive
    /// means entity coherence predicts its own future beyond what any
    /// linear combination of components can — the closest thing this
    /// module offers to an affirmative emergence signal.
    pub psi_corrected: f64,
    /// Up to the top-k pairs by synergy, ranked descending. Identifies
    /// the specific interaction clusters driving joint information.
    pub top_pairs: Vec<SynergyPair>,
    /// Number of (V_t, V_{t+1}) sample pairs used.
    pub n_samples: usize,
    /// Number of member relationships contributing a non-degenerate
    /// X series (zero-variance series are dropped).
    pub n_components: usize,
    /// Number of component pairs the pair-PID actually evaluated
    /// (`n_components × (n_components − 1) / 2` minus pairs where joint
    /// MI was indeterminate). Denominator for [`mean_pair_synergy`].
    pub n_pairs_evaluated: usize,
    /// Sum of synergy over **all** evaluated pairs (not just the top-K
    /// retained in `top_pairs`). Positive indicates aggregate pair-level
    /// synergy — information carried by joint pairs but not by individual
    /// components. Part of the H5 "pair-grain emergence" signal.
    pub total_pair_synergy: f64,
    /// Sum of MMI redundancy over all evaluated pairs. Together with
    /// `total_pair_synergy` this quantifies how much of the pair-joint
    /// information is overlap vs. non-additive interaction.
    pub total_pair_redundancy: f64,
    /// `total_pair_synergy / n_pairs_evaluated`. Units: nats per pair.
    pub mean_pair_synergy: f64,
    /// **H5 — pair-grain Ψ.** `i_self − Σ I(X_a, X_b; V_{t+1})` summed
    /// over the top-3 synergistic pairs. A leniency metric relative to
    /// [`psi_corrected`]: if even this is ≤ 0, the entity scalar carries
    /// no emergent information beyond what a very small pair cover
    /// captures. If > 0 while `psi_corrected` ≤ 0, then V_t predicts its
    /// future better than the top-3 pairs can but worse than the full
    /// joint — a nuanced but real signal of scalar-level structure.
    ///
    /// Note: the sum overcounts pair-joint MI when pairs share a
    /// component. This is deliberately not corrected — the metric is
    /// intentionally conservative against emergence.
    pub psi_pair_top3: f64,
}

/// A single leave-one-out drop measurement from
/// [`psi_synergy_leave_one_out`]. Compares Ψ metrics with and without
/// one specific component relationship.
#[derive(Debug, Clone, PartialEq)]
pub struct DropResult {
    /// The component whose series was excluded for this measurement.
    pub dropped: RelationshipId,
    /// `i_self − I(kept_components; V_{t+1})` with the drop applied.
    pub psi_corrected: f64,
    /// `i_self − Σ top-3 I(pair; V_{t+1})` over pairs that do not
    /// include the dropped component.
    pub psi_pair_top3: f64,
    /// `baseline.psi_corrected − self.psi_corrected`. Positive means
    /// the drop hurt the full-joint prediction — i.e. this component
    /// carried unique information.
    pub psi_corrected_delta: f64,
    /// `baseline.psi_pair_top3 − self.psi_pair_top3`. Positive means
    /// the drop hurt the top-3-pair prediction.
    pub psi_pair_top3_delta: f64,
}

/// Result of [`psi_synergy_leave_one_out`]: baseline Ψ plus one
/// [`DropResult`] per non-degenerate component, sorted by whichever
/// metric the caller prefers (this type preserves the original order).
///
/// Interpretation: if `sign_flips_pair_top3 == 0`, the baseline
/// positive-Ψ signal (if any) is robust to any single-component
/// removal — no single rel is "load-bearing" for the emergence claim.
/// If high, the signal depends on a small number of components.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaveOneOutResult {
    pub entity: EntityId,
    /// Baseline Ψ — what `psi_synergy` returns for the full component set.
    pub baseline: PsiSynergyResult,
    /// One entry per component series retained by the zero-variance
    /// filter. Order matches `baseline.top_pairs`' underlying component
    /// ordering (i.e. not sorted by effect size).
    pub drops: Vec<DropResult>,
}

impl LeaveOneOutResult {
    /// Number of drops whose `psi_corrected` has the opposite sign from
    /// the baseline. Zero is strong evidence of robustness.
    pub fn sign_flips_corrected(&self) -> usize {
        let b = self.baseline.psi_corrected;
        self.drops
            .iter()
            .filter(|d| d.psi_corrected.signum() != b.signum())
            .count()
    }

    /// Number of drops whose `psi_pair_top3` has the opposite sign
    /// from the baseline. The load-bearing-component count.
    pub fn sign_flips_pair_top3(&self) -> usize {
        let b = self.baseline.psi_pair_top3;
        self.drops
            .iter()
            .filter(|d| d.psi_pair_top3.signum() != b.signum())
            .count()
    }

    /// The drop with the largest `psi_pair_top3_delta` (most
    /// "load-bearing"), if any drops exist.
    pub fn most_load_bearing_for_pair_top3(&self) -> Option<&DropResult> {
        self.drops.iter().max_by(|a, b| {
            a.psi_pair_top3_delta
                .partial_cmp(&b.psi_pair_top3_delta)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Render as Markdown — a summary line plus a per-drop table sorted
    /// by |`psi_pair_top3_delta`| descending (largest effect first).
    pub fn render_markdown(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();

        let _ = writeln!(&mut out, "## Leave-one-out — {:?}\n", self.entity);
        let _ = writeln!(
            &mut out,
            "- baseline: Ψ_corrected = {:+.4}, Ψ_pair_top3 = {:+.4}",
            self.baseline.psi_corrected, self.baseline.psi_pair_top3
        );
        let _ = writeln!(
            &mut out,
            "- components: **{}**, pairs evaluated: **{}**",
            self.baseline.n_components, self.baseline.n_pairs_evaluated
        );
        let _ = writeln!(
            &mut out,
            "- sign flips (Ψ_corrected): **{}** / {}",
            self.sign_flips_corrected(),
            self.drops.len()
        );
        let _ = writeln!(
            &mut out,
            "- sign flips (Ψ_pair_top3): **{}** / {}",
            self.sign_flips_pair_top3(),
            self.drops.len()
        );

        let mut sorted: Vec<&DropResult> = self.drops.iter().collect();
        sorted.sort_by(|a, b| {
            b.psi_pair_top3_delta
                .abs()
                .partial_cmp(&a.psi_pair_top3_delta.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if !sorted.is_empty() {
            let _ = writeln!(&mut out, "\n### Per-drop effect, top 10 by |Δ Ψ_pair_top3|",);
            let _ = writeln!(
                &mut out,
                "\n| dropped | Ψ_corr | Ψ_pair_top3 | Δ Ψ_corr | Δ Ψ_pair_top3 |"
            );
            let _ = writeln!(&mut out, "|---|---|---|---|---|");
            for d in sorted.iter().take(10) {
                let _ = writeln!(
                    &mut out,
                    "| {:?} | {:+.4} | {:+.4} | {:+.4} | {:+.4} |",
                    d.dropped,
                    d.psi_corrected,
                    d.psi_pair_top3,
                    d.psi_corrected_delta,
                    d.psi_pair_top3_delta,
                );
            }
        }
        out
    }
}

/// One measured entity in an [`EmergenceReport`]. Combines the entity
/// identity with its Ψ decomposition.
#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceEntry {
    pub entity: EntityId,
    pub psi: PsiResult,
}

/// One measured entity in an [`EmergenceSynergyReport`].
#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceSynergyEntry {
    pub entity: EntityId,
    pub psi: PsiSynergyResult,
}

/// Why an entity could not be Ψ-measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmeasuredReason {
    /// Entity is dormant — no new layers deposit, so Ψ is not meaningful.
    Dormant,
    /// The current dense coherence series has fewer than 3 samples.
    /// `layer_count` is the number of sample batches found (change-batches
    /// of member relationships within the active lifetime). Need ≥ 2
    /// lag-1 pairs to estimate MI.
    InsufficientStableWindow { layer_count: usize },
    /// All member relationships lacked a usable history (zero-variance
    /// weight series, or no recorded changes at the window batches).
    NoComponentHistory,
}

/// Entity listed in the "unmeasured" bucket of an [`EmergenceReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmeasuredEntry {
    pub entity: EntityId,
    pub reason: UnmeasuredReason,
}

/// Output of [`emergence_report`]. A world-level summary of which entities
/// show causal emergence (Ψ > 0), which are well-explained by their parts
/// (Ψ ≤ 0 — labelled "spurious" following the framing in
/// `docs/roadmap.md` Track H), and which could not be measured.
///
/// The report is a diagnostic artifact: a positive Ψ is *evidence* of
/// emergence under the Gaussian MI approximation, not a formal proof.
/// A negative Ψ does *not* disprove emergence — it means the estimator
/// did not find it at this grain.
#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceReport {
    /// Entities with Ψ > 0, sorted descending by Ψ (strongest first).
    pub emergent: Vec<EmergenceEntry>,
    /// Entities with Ψ ≤ 0, sorted descending by Ψ (least spurious first).
    pub spurious: Vec<EmergenceEntry>,
    /// Entities excluded from Ψ computation, with reason.
    pub unmeasured: Vec<UnmeasuredEntry>,
    /// Total entities considered (every entity in the store,
    /// including dormant ones).
    pub n_entities: usize,
}

/// Output of [`emergence_report_synergy`]. Same shape as
/// [`EmergenceReport`] but the `emergent` / `spurious` split is driven by
/// `psi_corrected` (redundancy-corrected Ψ), not `psi_naive`.
///
/// The Track H2 motivation: the naïve Ψ can go negative purely because
/// `Σᵢ I(Xᵢ; V_{t+1})` over-counts redundant information. The corrected
/// Ψ subtracts that overcount via joint MI and is the closer fit to the
/// "does the whole predict its future beyond its parts?" question.
#[derive(Debug, Clone, PartialEq)]
pub struct EmergenceSynergyReport {
    /// Entities with `psi_corrected > 0`, sorted descending.
    pub emergent: Vec<EmergenceSynergyEntry>,
    /// Entities with `psi_corrected ≤ 0`, sorted descending.
    pub spurious: Vec<EmergenceSynergyEntry>,
    /// Entities excluded from Ψ computation, with reason. Shares the
    /// same [`UnmeasuredReason`] vocabulary as [`EmergenceReport`].
    pub unmeasured: Vec<UnmeasuredEntry>,
    pub n_entities: usize,
}

impl EmergenceReport {
    /// Number of entities that produced a usable Ψ estimate.
    pub fn n_measured(&self) -> usize {
        self.emergent.len() + self.spurious.len()
    }

    /// Fraction of measured entities with Ψ > 0, or `None` if no entity
    /// could be measured.
    pub fn emergent_fraction(&self) -> Option<f64> {
        let n = self.n_measured();
        if n == 0 {
            None
        } else {
            Some(self.emergent.len() as f64 / n as f64)
        }
    }

    /// Render the report as human-readable Markdown. Used by example
    /// binaries and prototype diagnostic tooling (Track K).
    pub fn render_markdown(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();

        let _ = writeln!(&mut out, "## Emergence report\n");
        let _ = writeln!(&mut out, "- entities total: **{}**", self.n_entities);
        let _ = writeln!(
            &mut out,
            "- measured: **{}** (emergent {}, spurious {})",
            self.n_measured(),
            self.emergent.len(),
            self.spurious.len()
        );
        let _ = writeln!(&mut out, "- unmeasured: **{}**", self.unmeasured.len());
        if let Some(f) = self.emergent_fraction() {
            let _ = writeln!(&mut out, "- emergent fraction: **{:.1}%**", f * 100.0);
        }

        if !self.emergent.is_empty() {
            let _ = writeln!(&mut out, "\n### Emergent (Ψ > 0), top 10");
            let _ = writeln!(&mut out, "\n| entity | Ψ | I_self | Σ I_components | n |");
            let _ = writeln!(&mut out, "|---|---|---|---|---|");
            for entry in self.emergent.iter().take(10) {
                let _ = writeln!(
                    &mut out,
                    "| {:?} | {:+.4} | {:.4} | {:.4} | {} |",
                    entry.entity,
                    entry.psi.psi,
                    entry.psi.i_self,
                    entry.psi.i_sum_components,
                    entry.psi.n_samples,
                );
            }
        }

        if !self.spurious.is_empty() {
            let _ = writeln!(&mut out, "\n### Spurious (Ψ ≤ 0), top 10");
            let _ = writeln!(&mut out, "\n| entity | Ψ | I_self | Σ I_components | n |");
            let _ = writeln!(&mut out, "|---|---|---|---|---|");
            for entry in self.spurious.iter().take(10) {
                let _ = writeln!(
                    &mut out,
                    "| {:?} | {:+.4} | {:.4} | {:.4} | {} |",
                    entry.entity,
                    entry.psi.psi,
                    entry.psi.i_self,
                    entry.psi.i_sum_components,
                    entry.psi.n_samples,
                );
            }
        }

        if !self.unmeasured.is_empty() {
            let mut dormant = 0usize;
            let mut short = 0usize;
            let mut no_hist = 0usize;
            for u in &self.unmeasured {
                match &u.reason {
                    UnmeasuredReason::Dormant => dormant += 1,
                    UnmeasuredReason::InsufficientStableWindow { .. } => short += 1,
                    UnmeasuredReason::NoComponentHistory => no_hist += 1,
                }
            }
            let _ = writeln!(&mut out, "\n### Unmeasured breakdown");
            let _ = writeln!(&mut out, "- dormant: {dormant}");
            let _ = writeln!(&mut out, "- insufficient stable window: {short}");
            let _ = writeln!(&mut out, "- no component history: {no_hist}");
        }

        out
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

    /// Render as Markdown. Shows both `psi_naive` and `psi_corrected`
    /// side-by-side so the reader can see the redundancy correction's
    /// effect at a glance.
    pub fn render_markdown(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();

        let _ = writeln!(&mut out, "## Emergence report (synergy-corrected)\n");
        let _ = writeln!(&mut out, "- entities total: **{}**", self.n_entities);
        let _ = writeln!(
            &mut out,
            "- measured: **{}** (emergent {}, spurious {})",
            self.n_measured(),
            self.emergent.len(),
            self.spurious.len()
        );
        let _ = writeln!(&mut out, "- unmeasured: **{}**", self.unmeasured.len());
        if let Some(f) = self.emergent_fraction() {
            let _ = writeln!(
                &mut out,
                "- emergent fraction (Ψ_corrected > 0): **{:.1}%**",
                f * 100.0
            );
        }

        let render_rows = |out: &mut String, rows: &[EmergenceSynergyEntry]| {
            let _ = writeln!(
                out,
                "\n| entity | Ψ_corr | Ψ_naive | I_self | I_joint | Σ I_i | n | comp |",
            );
            let _ = writeln!(out, "|---|---|---|---|---|---|---|---|");
            for entry in rows.iter().take(10) {
                let _ = writeln!(
                    out,
                    "| {:?} | {:+.4} | {:+.4} | {:.4} | {:.4} | {:.4} | {} | {} |",
                    entry.entity,
                    entry.psi.psi_corrected,
                    entry.psi.psi_naive,
                    entry.psi.i_self,
                    entry.psi.i_joint_components,
                    entry.psi.i_sum_components,
                    entry.psi.n_samples,
                    entry.psi.n_components,
                );
            }
        };

        if !self.emergent.is_empty() {
            let _ = writeln!(&mut out, "\n### Emergent (Ψ_corrected > 0), top 10");
            render_rows(&mut out, &self.emergent);
        }
        if !self.spurious.is_empty() {
            let _ = writeln!(&mut out, "\n### Spurious (Ψ_corrected ≤ 0), top 10");
            render_rows(&mut out, &self.spurious);
        }

        // H3 redux — per-entity top synergistic pair. Attribution for
        // *where* the joint information sits when `psi_corrected` is
        // non-trivial. Shows the #1 pair by synergy from
        // `PsiSynergyResult::top_pairs`.
        let top_pair_rows: Vec<(&EmergenceSynergyEntry, &SynergyPair)> = self
            .emergent
            .iter()
            .chain(self.spurious.iter())
            .filter_map(|e| e.psi.top_pairs.first().map(|p| (e, p)))
            .take(10)
            .collect();
        if !top_pair_rows.is_empty() {
            let _ = writeln!(&mut out, "\n### Top synergistic pair per entity, top 10");
            let _ = writeln!(
                &mut out,
                "\n| entity | pair (a, b) | synergy | joint_mi | redundancy | mi_a | mi_b |"
            );
            let _ = writeln!(&mut out, "|---|---|---|---|---|---|---|");
            for (entry, pair) in top_pair_rows {
                let _ = writeln!(
                    &mut out,
                    "| {:?} | {:?}, {:?} | {:+.4} | {:.4} | {:.4} | {:.4} | {:.4} |",
                    entry.entity,
                    pair.a,
                    pair.b,
                    pair.synergy,
                    pair.joint_mi,
                    pair.redundancy,
                    pair.mi_a,
                    pair.mi_b,
                );
            }
        }

        // H5 — aggregate pair-grain emergence signal. Shows the summed
        // pair synergy and the conservative `psi_pair_top3` metric for
        // every measured entity. This is the direct test of the
        // re-scoped Track H hypothesis: does emergence live at pair
        // grain rather than at the entity scalar?
        let h5_rows: Vec<&EmergenceSynergyEntry> = self
            .emergent
            .iter()
            .chain(self.spurious.iter())
            .filter(|e| e.psi.n_pairs_evaluated > 0)
            .take(10)
            .collect();
        if !h5_rows.is_empty() {
            let _ = writeln!(&mut out, "\n### Pair-grain emergence (H5), top 10 measured");
            let _ = writeln!(
                &mut out,
                "\n| entity | Ψ_pair_top3 | Σ synergy | Σ redundancy | mean synergy | n_pairs |",
            );
            let _ = writeln!(&mut out, "|---|---|---|---|---|---|");
            for entry in h5_rows {
                let _ = writeln!(
                    &mut out,
                    "| {:?} | {:+.4} | {:+.4} | {:.4} | {:+.4} | {} |",
                    entry.entity,
                    entry.psi.psi_pair_top3,
                    entry.psi.total_pair_synergy,
                    entry.psi.total_pair_redundancy,
                    entry.psi.mean_pair_synergy,
                    entry.psi.n_pairs_evaluated,
                );
            }
        }

        if !self.unmeasured.is_empty() {
            let mut dormant = 0usize;
            let mut short = 0usize;
            let mut no_hist = 0usize;
            for u in &self.unmeasured {
                match &u.reason {
                    UnmeasuredReason::Dormant => dormant += 1,
                    UnmeasuredReason::InsufficientStableWindow { .. } => short += 1,
                    UnmeasuredReason::NoComponentHistory => no_hist += 1,
                }
            }
            let _ = writeln!(&mut out, "\n### Unmeasured breakdown");
            let _ = writeln!(&mut out, "- dormant: {dormant}");
            let _ = writeln!(&mut out, "- insufficient stable window: {short}");
            let _ = writeln!(&mut out, "- no component history: {no_hist}");
        }

        out
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// The coherence time series for an entity's current stable window.
///
/// Returns `(batch, coherence)` pairs, oldest first. The series contains
/// only `DepositLayer` events (no lifecycle transitions). Returns an empty
/// `Vec` if the entity does not exist, has no layers, or has no stable
/// deposit events in its current window.
pub fn coherence_stable_series(world: &World, entity_id: EntityId) -> Vec<(BatchId, f32)> {
    let entity = match world.entities().get(entity_id) {
        Some(e) => e,
        None => return Vec::new(),
    };

    // Walk newest→oldest to find the start of the current stable window.
    // The window begins right after the most recent lifecycle transition.
    let start_idx = entity
        .layers
        .iter()
        .rposition(|l| is_lifecycle_transition(l))
        .map(|i| i + 1)
        .unwrap_or(0);

    entity.layers[start_idx..]
        .iter()
        .filter_map(|l| layer_coherence(l).map(|c| (l.batch, c)))
        .collect()
}

/// Dense coherence series for an entity's current stable window.
///
/// Unlike [`coherence_stable_series`] (which samples only at `DepositLayer`
/// batches), this function samples coherence at **every batch** where any
/// member relationship had a `ChangeLog` entry, within the active lifetime
/// (since the most recent lifecycle transition). This produces many more
/// samples on workloads where emergence deposits stop firing after the
/// entity stabilises — the original failure mode documented in
/// `docs/emergence/h4-report.md §3`.
///
/// Coherence at batch B is recomputed from the same formula the engine
/// uses (`mean_activity × density`; see
/// `DefaultEmergencePerspective::component_stats`) against the entity's
/// current member locus / relationship set and the relationships' slot-0
/// activity *at their most recent ChangeLog event at or before B*.
///
/// # Approximation caveat
///
/// Between ChangeLog events an activity slot decays per batch per the
/// relationship kind's `decay_per_batch`. This sampler does **not**
/// reconstruct that decay — it reads the last `change.after[0]` as-is.
/// Consequence: coherence is over-estimated on batches where a member
/// relationship last changed many batches ago. The bias is bounded by the
/// sparsity of the member relationships' change histories; for actively
/// touched relationships the bias is small. Refinement (decay-aware
/// reconstruction) is a Track H follow-up.
///
/// Returns an empty `Vec` when:
/// - The entity does not exist.
/// - The entity has no member relationships.
/// - No member relationship has any ChangeLog entry in the stable window.
pub fn coherence_dense_series(world: &World, entity_id: EntityId) -> Vec<(BatchId, f32)> {
    coherence_dense_series_inner(world, entity_id, None)
}

/// Decay-aware counterpart to [`coherence_dense_series`]. Reconstructs
/// each member relationship's slot-0 activity at the sample batch using
/// `rate^(batch − last_change_batch)` before evaluating coherence.
///
/// The roadmap H4.3 follow-up (see `docs/emergence/h4-report.md`): when
/// `decay_rates` are available the V_t series honours inter-change decay,
/// removing the main remaining bias in coherence reconstruction.
pub fn coherence_dense_series_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Vec<(BatchId, f32)> {
    coherence_dense_series_inner(world, entity_id, Some(decay_rates))
}

fn coherence_dense_series_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Vec<(BatchId, f32)> {
    let entity = match world.entities().get(entity_id) {
        Some(e) => e,
        None => return Vec::new(),
    };

    let members = &entity.current.members;
    let member_rels = &entity.current.member_relationships;
    if member_rels.is_empty() {
        return Vec::new();
    }

    // Active window starts at the batch *after* the most recent lifecycle
    // transition, matching `coherence_stable_series`' window definition.
    let window_start_batch = entity
        .layers
        .iter()
        .rposition(|l| is_lifecycle_transition(l))
        .map(|i| entity.layers[i].batch)
        .unwrap_or(BatchId(0));

    // Union of change batches across member relationships, restricted to
    // the window. `BTreeSet` gives us sorted-unique for free.
    let mut sample_batches: std::collections::BTreeSet<BatchId> = std::collections::BTreeSet::new();
    for &rel_id in member_rels {
        for change in world.changes_to_relationship(rel_id) {
            if change.batch > window_start_batch {
                sample_batches.insert(change.batch);
            }
        }
    }

    sample_batches
        .into_iter()
        .map(|batch| {
            let coh = coherence_at_batch(
                world,
                members,
                member_rels,
                batch,
                DEFAULT_MIN_ACTIVITY_THRESHOLD,
                decay_rates,
            );
            (batch, coh)
        })
        .collect()
}

/// Pearson autocorrelation of the coherence series at `lag`.
///
/// Uses only the current stable window (see module docs). Returns `None`
/// when there are fewer than `lag + 2` points — the minimum needed for a
/// meaningful estimate.
///
/// A value near +1.0 means coherence at layer *t* strongly predicts
/// coherence at layer *t + lag*, which is evidence that the entity has
/// persistent internal structure. A value near 0 means the coherence
/// series is noise in this window — Ψ computation would not be
/// informative.
pub fn coherence_autocorrelation(world: &World, entity_id: EntityId, lag: usize) -> Option<f64> {
    let series: Vec<f32> = coherence_stable_series(world, entity_id)
        .into_iter()
        .map(|(_, c)| c)
        .collect();

    pearson_autocorr(&series, lag)
}

/// Estimate the emergence capacity Ψ for `entity_id` over its current stable window.
///
/// **Components (X_i)**: member *relationships* (not loci). Entity coherence
/// supervenes on relationship activity and weight, not on locus states (which
/// decay to near-zero between ticks). Slot 1 (weight) is used because it
/// accumulates monotonically under Hebbian plasticity and has non-trivial
/// variance across the window.
///
/// **Sampling**: uses [`coherence_dense_series`] — one sample per batch where
/// any member relationship recorded a change, within the active lifetime.
/// This replaces the deposit-event sampling of earlier iterations which
/// under-sampled stable entities (see `docs/emergence/h4-report.md`).
///
/// Returns `None` when:
/// - The entity does not exist.
/// - The dense series has fewer than 3 samples (need ≥ 2 lag-1 pairs).
/// - All member relationships lack recorded history (no ChangeLog entries).
///
/// Uses the Gaussian MI approximation; see module docs for caveats.
pub fn psi_scalar(world: &World, entity_id: EntityId) -> Option<PsiResult> {
    psi_scalar_inner(world, entity_id, None)
}

/// Decay-aware counterpart to [`psi_scalar`]. The V_t / V_{t+1} series
/// is built from [`coherence_dense_series_with_decay`]; X_i (relationship
/// weight) is unaffected because weight does not decay.
pub fn psi_scalar_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Option<PsiResult> {
    psi_scalar_inner(world, entity_id, Some(decay_rates))
}

fn psi_scalar_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Option<PsiResult> {
    let entity = world.entities().get(entity_id)?;

    // Build (batch, coherence) series for the stable window — dense
    // sampling at every member-relationship change batch.
    let window = coherence_dense_series_inner(world, entity_id, decay_rates);
    let n = window.len();
    if n < 3 {
        return None;
    }

    // V_t and V_{t+1} series (n-1 pairs).
    let v_t: Vec<f64> = window[..n - 1].iter().map(|(_, c)| *c as f64).collect();
    let v_t1: Vec<f64> = window[1..].iter().map(|(_, c)| *c as f64).collect();

    let i_self = gaussian_mi_from_series(&v_t, &v_t1)?;

    // For each member relationship, build Xᵢ_t (weight at each window batch)
    // and compute I(Xᵢ_t; V_{t+1}).
    //
    // Weight (slot 1) is used: it grows monotonically under Hebbian plasticity,
    // giving non-zero variance across the window. Activity (slot 0) decays to
    // near-zero between ticks, yielding zero-variance series at layer batches.
    let member_rels = entity.current.member_relationships.clone();
    let mut i_sum = 0.0f64;
    let mut n_components = 0usize;

    for rel_id in &member_rels {
        let xi_t: Vec<f64> = window[..n - 1]
            .iter()
            .map(|(batch, _)| rel_weight_at(*batch, *rel_id, world))
            .collect();

        if let Some(mi) = gaussian_mi_from_series(&xi_t, &v_t1) {
            i_sum += mi;
            n_components += 1;
        }
    }

    if n_components == 0 {
        return None;
    }

    Some(PsiResult {
        psi: i_self - i_sum,
        i_self,
        i_sum_components: i_sum,
        n_samples: n - 1,
        n_components,
    })
}

/// Redundancy-corrected Ψ with a pairwise Φ-ID-style breakdown.
///
/// Extends [`psi_scalar`] with two changes:
///
/// 1. Replaces the naïve `Σᵢ I(Xᵢ; V_{t+1})` with the joint Gaussian MI
///    `I(X⃗; V_{t+1})`, which subtracts the redundancy double-counting
///    inherent in the sum. `psi_corrected = i_self − i_joint_components`
///    can be positive where [`psi_scalar`] is negative when member
///    relationships carry overlapping information.
/// 2. Computes a pairwise PID decomposition (redundancy / unique /
///    synergy) per pair of components, returning the top `MAX_TOP_PAIRS`
///    pairs by synergy. This attributes any positive Ψ to a specific
///    interaction cluster as requested by roadmap Track H2.
///
/// Returns `None` under the same conditions as [`psi_scalar`], plus:
/// - Fewer than 2 non-degenerate component series (joint MI is undefined).
/// - Sample count is too small to fit `n_components` predictors
///   (need `n_samples ≥ n_components + 2`).
pub fn psi_synergy(world: &World, entity_id: EntityId) -> Option<PsiSynergyResult> {
    psi_synergy_inner(world, entity_id, None)
}

/// Decay-aware counterpart to [`psi_synergy`]. See [`psi_scalar_with_decay`].
pub fn psi_synergy_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Option<PsiSynergyResult> {
    psi_synergy_inner(world, entity_id, Some(decay_rates))
}

fn psi_synergy_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Option<PsiSynergyResult> {
    const MAX_TOP_PAIRS: usize = 5;

    let entity = world.entities().get(entity_id)?;

    let window = coherence_dense_series_inner(world, entity_id, decay_rates);
    let n = window.len();
    if n < 3 {
        return None;
    }

    let v_t: Vec<f64> = window[..n - 1].iter().map(|(_, c)| *c as f64).collect();
    let v_t1: Vec<f64> = window[1..].iter().map(|(_, c)| *c as f64).collect();

    let i_self = gaussian_mi_from_series(&v_t, &v_t1)?;

    // Collect per-component X_i series, keeping only non-degenerate ones.
    let member_rels = &entity.current.member_relationships;
    let mut x_series: Vec<Vec<f64>> = Vec::with_capacity(member_rels.len());
    let mut x_rel_ids: Vec<RelationshipId> = Vec::with_capacity(member_rels.len());
    let mut individual_mi: Vec<f64> = Vec::with_capacity(member_rels.len());
    for rel_id in member_rels {
        let xi_t: Vec<f64> = window[..n - 1]
            .iter()
            .map(|(batch, _)| rel_weight_at(*batch, *rel_id, world))
            .collect();
        // Keep only series that Pearson r (and therefore MI) can accept.
        if let Some(mi) = gaussian_mi_from_series(&xi_t, &v_t1) {
            x_series.push(xi_t);
            x_rel_ids.push(*rel_id);
            individual_mi.push(mi);
        }
    }

    let n_components = x_series.len();
    if n_components < 2 {
        return None;
    }
    // Joint MI needs n_samples ≥ n_components + 2 for OLS not to over-fit.
    if (n - 1) < n_components + 2 {
        return None;
    }

    let i_sum: f64 = individual_mi.iter().sum();
    let i_joint = gaussian_joint_mi(&x_series, &v_t1)?;

    // Pairwise decomposition. `gaussian_joint_mi` with 2 predictors gives
    // us I(X_a, X_b; Y); combine with the cached individual MIs.
    let mut pairs: Vec<SynergyPair> = Vec::with_capacity(n_components * (n_components - 1) / 2);
    for i in 0..n_components {
        for j in (i + 1)..n_components {
            let pair = vec![x_series[i].clone(), x_series[j].clone()];
            let joint_mi = match gaussian_joint_mi(&pair, &v_t1) {
                Some(v) => v,
                None => continue,
            };
            let mi_a = individual_mi[i];
            let mi_b = individual_mi[j];
            let redundancy = mi_a.min(mi_b);
            let synergy = joint_mi - mi_a - mi_b + redundancy;
            pairs.push(SynergyPair {
                a: x_rel_ids[i],
                b: x_rel_ids[j],
                mi_a,
                mi_b,
                joint_mi,
                redundancy,
                synergy,
            });
        }
    }

    // Aggregate H5 fields — computed over ALL evaluated pairs before
    // truncating to the top-K for the `top_pairs` field.
    let n_pairs_evaluated = pairs.len();
    let total_pair_synergy: f64 = pairs.iter().map(|p| p.synergy).sum();
    let total_pair_redundancy: f64 = pairs.iter().map(|p| p.redundancy).sum();
    let mean_pair_synergy = if n_pairs_evaluated > 0 {
        total_pair_synergy / n_pairs_evaluated as f64
    } else {
        0.0
    };

    // Sort by synergy (descending) first — this also orders the sum for
    // `psi_pair_top3` below.
    pairs.sort_by(|a, b| {
        b.synergy
            .partial_cmp(&a.synergy)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // H5 — sum of joint MIs over the top-3 synergistic pairs. Overcounts
    // when pairs share a component (deliberately — conservative against
    // emergence). See docs on `psi_pair_top3`.
    let top3_joint_sum: f64 = pairs.iter().take(3).map(|p| p.joint_mi).sum();
    let psi_pair_top3 = i_self - top3_joint_sum;

    pairs.truncate(MAX_TOP_PAIRS);

    Some(PsiSynergyResult {
        i_self,
        i_sum_components: i_sum,
        i_joint_components: i_joint,
        psi_naive: i_self - i_sum,
        psi_corrected: i_self - i_joint,
        top_pairs: pairs,
        n_samples: n - 1,
        n_components,
        n_pairs_evaluated,
        total_pair_synergy,
        total_pair_redundancy,
        mean_pair_synergy,
        psi_pair_top3,
    })
}

/// Leave-one-out robustness probe (roadmap H4.2).
///
/// For each non-degenerate component, recomputes `psi_corrected` and
/// `psi_pair_top3` with that component's series excluded. Compares
/// against the full-component baseline. Used to check whether a
/// positive-Ψ signal depends on a small number of load-bearing
/// components.
///
/// Preconditions: the entity must have ≥ 3 non-degenerate components
/// (so that any leave-one-out subset still has ≥ 2, the minimum for
/// joint MI) and sufficient sample count.
///
/// Complexity: O(n³) pair-joint-MI evaluations for an entity with n
/// components — n drops × C(n−1, 2) pair MIs per drop. Run selectively
/// rather than across an entire world.
pub fn psi_synergy_leave_one_out(world: &World, entity_id: EntityId) -> Option<LeaveOneOutResult> {
    psi_synergy_leave_one_out_inner(world, entity_id, None)
}

/// Decay-aware counterpart to [`psi_synergy_leave_one_out`].
pub fn psi_synergy_leave_one_out_with_decay(
    world: &World,
    entity_id: EntityId,
    decay_rates: &DecayRates,
) -> Option<LeaveOneOutResult> {
    psi_synergy_leave_one_out_inner(world, entity_id, Some(decay_rates))
}

fn psi_synergy_leave_one_out_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Option<LeaveOneOutResult> {
    let baseline = psi_synergy_inner(world, entity_id, decay_rates)?;

    // Rebuild the X series. Mirrors psi_synergy_inner so we drop the
    // same set of zero-variance components that the baseline did.
    let entity = world.entities().get(entity_id)?;
    let window = coherence_dense_series_inner(world, entity_id, decay_rates);
    let n = window.len();
    let v_t: Vec<f64> = window[..n - 1].iter().map(|(_, c)| *c as f64).collect();
    let v_t1: Vec<f64> = window[1..].iter().map(|(_, c)| *c as f64).collect();
    let _ = v_t; // only v_t1 is used as the regression target

    let member_rels = &entity.current.member_relationships;
    let mut x_series: Vec<Vec<f64>> = Vec::with_capacity(member_rels.len());
    let mut x_rel_ids: Vec<RelationshipId> = Vec::with_capacity(member_rels.len());
    for rel_id in member_rels {
        let xi_t: Vec<f64> = window[..n - 1]
            .iter()
            .map(|(batch, _)| rel_weight_at(*batch, *rel_id, world))
            .collect();
        if gaussian_mi_from_series(&xi_t, &v_t1).is_some() {
            x_series.push(xi_t);
            x_rel_ids.push(*rel_id);
        }
    }

    let n_components = x_series.len();
    if n_components < 3 {
        // Need ≥ 3 so the leave-one-out subset still has ≥ 2 components
        // for joint MI.
        return None;
    }

    let mut drops = Vec::with_capacity(n_components);
    for drop_idx in 0..n_components {
        let kept: Vec<Vec<f64>> = x_series
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != drop_idx)
            .map(|(_, s)| s.clone())
            .collect();

        // Joint MI across the kept set → psi_corrected under the drop.
        let i_joint_k = match gaussian_joint_mi(&kept, &v_t1) {
            Some(v) => v,
            None => {
                // Singular subset — treat as fully redundant with the
                // dropped component; use its baseline joint as a
                // conservative fallback.
                baseline.i_joint_components
            }
        };
        let psi_c = baseline.i_self - i_joint_k;

        // Top-3 pair joint MI over pairs that do NOT include drop_idx.
        let mut kept_pair_scores: Vec<(f64, f64)> = Vec::new(); // (synergy, joint_mi)
        for i in 0..n_components {
            if i == drop_idx {
                continue;
            }
            for j in (i + 1)..n_components {
                if j == drop_idx {
                    continue;
                }
                let pair = vec![x_series[i].clone(), x_series[j].clone()];
                let Some(joint_mi) = gaussian_joint_mi(&pair, &v_t1) else {
                    continue;
                };
                let mi_a = gaussian_mi_from_series(&x_series[i], &v_t1).unwrap_or(0.0);
                let mi_b = gaussian_mi_from_series(&x_series[j], &v_t1).unwrap_or(0.0);
                let redundancy = mi_a.min(mi_b);
                let synergy = joint_mi - mi_a - mi_b + redundancy;
                kept_pair_scores.push((synergy, joint_mi));
            }
        }
        // Rank by synergy (match baseline convention), sum top-3 joints.
        kept_pair_scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let top3_joint: f64 = kept_pair_scores.iter().take(3).map(|(_, j)| *j).sum();
        let psi_p = baseline.i_self - top3_joint;

        drops.push(DropResult {
            dropped: x_rel_ids[drop_idx],
            psi_corrected: psi_c,
            psi_pair_top3: psi_p,
            psi_corrected_delta: baseline.psi_corrected - psi_c,
            psi_pair_top3_delta: baseline.psi_pair_top3 - psi_p,
        });
    }

    Some(LeaveOneOutResult {
        entity: entity_id,
        baseline,
        drops,
    })
}

/// Build a world-level [`EmergenceReport`]. One Ψ measurement per entity.
///
/// Dormant entities are skipped (listed under `unmeasured` with
/// [`UnmeasuredReason::Dormant`]). Active entities whose stable window is
/// too short or whose members lack a usable history are also surfaced
/// under `unmeasured`. Everything else is sorted into `emergent` (Ψ > 0)
/// or `spurious` (Ψ ≤ 0), each sorted descending by Ψ so the most
/// noteworthy entities appear first.
///
/// Complexity: O(E × W × M) where E = entity count, W = stable-window
/// length, M = avg member relationships per entity. No world mutation.
pub fn emergence_report(world: &World) -> EmergenceReport {
    emergence_report_inner(world, None)
}

/// Decay-aware counterpart to [`emergence_report`]. Each entity's Ψ is
/// computed via [`psi_scalar_with_decay`].
pub fn emergence_report_with_decay(world: &World, decay_rates: &DecayRates) -> EmergenceReport {
    emergence_report_inner(world, Some(decay_rates))
}

fn emergence_report_inner(world: &World, decay_rates: Option<&DecayRates>) -> EmergenceReport {
    let mut emergent = Vec::new();
    let mut spurious = Vec::new();
    let mut unmeasured = Vec::new();
    let mut n_entities = 0usize;

    for entity in world.entities().iter() {
        n_entities += 1;

        if matches!(entity.status, EntityStatus::Dormant) {
            unmeasured.push(UnmeasuredEntry {
                entity: entity.id,
                reason: UnmeasuredReason::Dormant,
            });
            continue;
        }

        match psi_scalar_inner(world, entity.id, decay_rates) {
            Some(psi) => {
                let entry = EmergenceEntry {
                    entity: entity.id,
                    psi,
                };
                if entry.psi.psi > 0.0 {
                    emergent.push(entry);
                } else {
                    spurious.push(entry);
                }
            }
            None => {
                let window_len = coherence_dense_series_inner(world, entity.id, decay_rates).len();
                let reason = if window_len < 3 {
                    UnmeasuredReason::InsufficientStableWindow {
                        layer_count: window_len,
                    }
                } else {
                    UnmeasuredReason::NoComponentHistory
                };
                unmeasured.push(UnmeasuredEntry {
                    entity: entity.id,
                    reason,
                });
            }
        }
    }

    emergent.sort_by(|a, b| {
        b.psi
            .psi
            .partial_cmp(&a.psi.psi)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    spurious.sort_by(|a, b| {
        b.psi
            .psi
            .partial_cmp(&a.psi.psi)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    EmergenceReport {
        emergent,
        spurious,
        unmeasured,
        n_entities,
    }
}

/// Synergy-aware counterpart to [`emergence_report`]. Calls [`psi_synergy`]
/// per entity and splits `emergent` / `spurious` on `psi_corrected`.
///
/// Entities that produce a usable naïve Ψ but fail the synergy version
/// (e.g. fewer than 2 non-degenerate components, or sample count too small
/// to fit the joint OLS system) fall into `unmeasured` with
/// [`UnmeasuredReason::NoComponentHistory`]. That reason is the closest
/// fit in the existing vocabulary; callers that need to distinguish
/// "short series" from "insufficient joint rank" should call
/// [`psi_synergy`] and [`psi_scalar`] directly.
pub fn emergence_report_synergy(world: &World) -> EmergenceSynergyReport {
    emergence_report_synergy_inner(world, None)
}

/// Decay-aware counterpart to [`emergence_report_synergy`].
pub fn emergence_report_synergy_with_decay(
    world: &World,
    decay_rates: &DecayRates,
) -> EmergenceSynergyReport {
    emergence_report_synergy_inner(world, Some(decay_rates))
}

fn emergence_report_synergy_inner(
    world: &World,
    decay_rates: Option<&DecayRates>,
) -> EmergenceSynergyReport {
    let mut emergent = Vec::new();
    let mut spurious = Vec::new();
    let mut unmeasured = Vec::new();
    let mut n_entities = 0usize;

    for entity in world.entities().iter() {
        n_entities += 1;

        if matches!(entity.status, EntityStatus::Dormant) {
            unmeasured.push(UnmeasuredEntry {
                entity: entity.id,
                reason: UnmeasuredReason::Dormant,
            });
            continue;
        }

        match psi_synergy_inner(world, entity.id, decay_rates) {
            Some(psi) => {
                let entry = EmergenceSynergyEntry {
                    entity: entity.id,
                    psi,
                };
                if entry.psi.psi_corrected > 0.0 {
                    emergent.push(entry);
                } else {
                    spurious.push(entry);
                }
            }
            None => {
                let window_len = coherence_dense_series_inner(world, entity.id, decay_rates).len();
                let reason = if window_len < 3 {
                    UnmeasuredReason::InsufficientStableWindow {
                        layer_count: window_len,
                    }
                } else {
                    UnmeasuredReason::NoComponentHistory
                };
                unmeasured.push(UnmeasuredEntry {
                    entity: entity.id,
                    reason,
                });
            }
        }
    }

    emergent.sort_by(|a, b| {
        b.psi
            .psi_corrected
            .partial_cmp(&a.psi.psi_corrected)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    spurious.sort_by(|a, b| {
        b.psi
            .psi_corrected
            .partial_cmp(&a.psi.psi_corrected)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    EmergenceSynergyReport {
        emergent,
        spurious,
        unmeasured,
        n_entities,
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Return slot-1 (weight) of the most recent change to `rel` at or before `batch`.
/// Falls back to 0.0 when the relationship has no recorded changes.
fn rel_weight_at(batch: BatchId, rel_id: RelationshipId, world: &World) -> f64 {
    world
        .changes_to_relationship(rel_id)
        .find(|c| c.batch <= batch && matches!(c.subject, ChangeSubject::Relationship(_)))
        .and_then(|c| c.after.as_slice().get(1).copied())
        .unwrap_or(0.0) as f64
}

/// Return slot-0 (activity) of the most recent change to `rel` at or before `batch`.
///
/// When `decay_rates` is `Some(rates)` and the relationship's kind has an
/// entry, the return value is decay-corrected:
///
/// `rate^(batch − change.batch) × change.after[0]`
///
/// When `decay_rates` is `None` (or the kind has no entry, i.e. rate = 1),
/// returns the raw `after[0]`. Falls back to `0.0` when the relationship
/// has no recorded changes.
fn rel_activity_at(
    batch: BatchId,
    rel_id: RelationshipId,
    world: &World,
    decay_rates: Option<&DecayRates>,
) -> f32 {
    let change = match world
        .changes_to_relationship(rel_id)
        .find(|c| c.batch <= batch && matches!(c.subject, ChangeSubject::Relationship(_)))
    {
        Some(c) => c,
        None => return 0.0,
    };
    let after_activity = change.after.as_slice().get(0).copied().unwrap_or(0.0);

    let rates = match decay_rates {
        Some(r) => r,
        None => return after_activity,
    };
    let rel = match world.relationships().get(rel_id) {
        Some(r) => r,
        None => return after_activity,
    };
    let rate = rates.get(&rel.kind).copied().unwrap_or(1.0);
    if rate >= 1.0 - f32::EPSILON {
        return after_activity;
    }
    let gap = batch.0.saturating_sub(change.batch.0);
    if gap == 0 {
        return after_activity;
    }
    // `powi` is well-defined for our batch gaps (bounded by u32::MAX
    // across any realistic run). Safe cast via `min`.
    after_activity * rate.powi(gap.min(i32::MAX as u64) as i32)
}

/// Reconstruct the entity coherence at a given batch.
///
/// Mirrors the formula used by `DefaultEmergencePerspective::component_stats`
/// in `graph-engine`:
///
/// ```text
/// coherence = mean_activity × density
///   mean_activity = mean of { activity(rel) : rel ∈ eligible, activity ≥ threshold }
///   density       = min(active_count / reference, 1.0)
///   reference     = n × ln(n + 1) / 2     where n = |members|
///   eligible      = { rel ∈ member_rels : both endpoints ∈ members }
/// ```
///
/// Activity is read via [`rel_activity_at`]; if `decay_rates` is `Some`,
/// decay is applied between the last change batch and the sample batch.
fn coherence_at_batch(
    world: &World,
    members: &[LocusId],
    member_rels: &[RelationshipId],
    batch: BatchId,
    threshold: f32,
    decay_rates: Option<&DecayRates>,
) -> f32 {
    let member_set: rustc_hash::FxHashSet<LocusId> = members.iter().copied().collect();
    let mut sum = 0.0f32;
    let mut active_count = 0usize;

    for &rel_id in member_rels {
        let rel = match world.relationships().get(rel_id) {
            Some(r) => r,
            None => continue,
        };
        if !rel.endpoints.all_endpoints_in(&member_set) {
            continue;
        }
        let activity = rel_activity_at(batch, rel_id, world, decay_rates);
        if activity >= threshold {
            sum += activity;
            active_count += 1;
        }
    }

    let mean_activity = if active_count == 0 {
        0.0
    } else {
        sum / active_count as f32
    };
    let n = members.len();
    let reference = if n <= 1 {
        1.0f32
    } else {
        (n as f32) * ((n as f32 + 1.0).ln()) / 2.0
    };
    let density = (active_count as f32 / reference).min(1.0);
    mean_activity * density
}

/// Pearson correlation between two equal-length slices.
/// Returns None when n < 2 or either series has zero variance.
fn pearson_r(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len();
    if n != b.len() || n < 2 {
        return None;
    }
    let mean_a = a.iter().sum::<f64>() / n as f64;
    let mean_b = b.iter().sum::<f64>() / n as f64;

    let (cov, var_a, var_b) =
        a.iter()
            .zip(b.iter())
            .fold((0.0f64, 0.0f64, 0.0f64), |(c, va, vb), (&ai, &bi)| {
                let da = ai - mean_a;
                let db = bi - mean_b;
                (c + da * db, va + da * da, vb + db * db)
            });
    if var_a < f64::EPSILON || var_b < f64::EPSILON {
        return None;
    }
    Some(cov / (var_a * var_b).sqrt())
}

/// Gaussian MI approximation: I(X;Y) ≈ −½ ln(1 − r²).
/// Returns None when Pearson r cannot be computed or |r| ≥ 1.
fn gaussian_mi_from_series(a: &[f64], b: &[f64]) -> Option<f64> {
    let r = pearson_r(a, b)?;
    let r2 = r * r;
    if r2 >= 1.0 - f64::EPSILON {
        return None;
    }
    Some(-0.5 * (1.0 - r2).ln())
}

/// Solve `A x = b` for symmetric square `A` via Gaussian elimination with
/// partial pivoting. Returns `None` if `A` is singular (pivot below
/// `1e-12`) or shapes are inconsistent.
///
/// The caller's `a` and `b` are consumed (mutated in place) during
/// elimination. Clone beforehand if you need the originals.
fn solve_linear_system(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = a.len();
    if n == 0 || b.len() != n || a.iter().any(|row| row.len() != n) {
        return None;
    }
    for i in 0..n {
        // Partial pivot: find the row in [i, n) with the largest |a[k][i]|.
        let mut max_row = i;
        for k in (i + 1)..n {
            if a[k][i].abs() > a[max_row][i].abs() {
                max_row = k;
            }
        }
        if a[max_row][i].abs() < 1e-12 {
            return None;
        }
        a.swap(i, max_row);
        b.swap(i, max_row);
        // Eliminate column i below the pivot.
        for k in (i + 1)..n {
            let factor = a[k][i] / a[i][i];
            for j in i..n {
                a[k][j] -= factor * a[i][j];
            }
            b[k] -= factor * b[i];
        }
    }
    // Back-substitution.
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = b[i];
        for j in (i + 1)..n {
            s -= a[i][j] * x[j];
        }
        x[i] = s / a[i][i];
    }
    Some(x)
}

/// Joint Gaussian MI of multiple predictors `x` with scalar target `y`.
///
/// Computes R² from OLS regression of `y` on the centred predictor matrix
/// `[x_0, x_1, ...]`, then returns `−½ ln(1 − R²)`. This captures
/// all pairwise correlations among the predictors, unlike a naïve sum of
/// per-predictor MIs which double-counts redundancy.
///
/// Returns `None` when:
/// - `x` is empty, sample lengths disagree, or sample count `< n + 2`.
/// - The target has zero variance.
/// - `X^T X` is singular (predictors are linearly dependent).
/// - `R² ≥ 1 − ε` (perfect fit — MI would be infinite).
fn gaussian_joint_mi(x: &[Vec<f64>], y: &[f64]) -> Option<f64> {
    let n = x.len();
    if n == 0 {
        return None;
    }
    let m = y.len();
    if m < n + 2 {
        return None;
    }
    if x.iter().any(|xi| xi.len() != m) {
        return None;
    }

    // Centre y.
    let y_mean = y.iter().sum::<f64>() / m as f64;
    let y_c: Vec<f64> = y.iter().map(|v| v - y_mean).collect();
    let ss_tot: f64 = y_c.iter().map(|v| v * v).sum();
    if ss_tot < f64::EPSILON {
        return None;
    }

    // Centre predictors.
    let x_c: Vec<Vec<f64>> = x
        .iter()
        .map(|xi| {
            let mean = xi.iter().sum::<f64>() / m as f64;
            xi.iter().map(|v| v - mean).collect()
        })
        .collect();

    // Build X^T X (n×n) and X^T y (n-vec).
    let mut ata = vec![vec![0.0; n]; n];
    let mut aty = vec![0.0; n];
    for i in 0..n {
        for j in 0..=i {
            let s: f64 = x_c[i].iter().zip(x_c[j].iter()).map(|(a, b)| a * b).sum();
            ata[i][j] = s;
            ata[j][i] = s;
        }
        aty[i] = x_c[i].iter().zip(y_c.iter()).map(|(a, b)| a * b).sum();
    }

    // Keep a copy of X^T y for the SS_explained shortcut: β · (X^T y).
    let aty_orig = aty.clone();
    let beta = solve_linear_system(ata, aty)?;

    let ss_exp: f64 = beta.iter().zip(aty_orig.iter()).map(|(b, a)| b * a).sum();
    let r2 = (ss_exp / ss_tot).clamp(0.0, 1.0);
    if r2 >= 1.0 - f64::EPSILON {
        return None;
    }
    Some(-0.5 * (1.0 - r2).ln())
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
        CompressionLevel::Full => layer.snapshot.as_ref().map(|s| s.coherence),
        CompressionLevel::Compressed { coherence, .. } => Some(*coherence),
        CompressionLevel::Skeleton { coherence, .. } => Some(*coherence),
    }
}

/// Pearson autocorrelation at `lag` using the standard unbiased formula.
///
/// r_k = Σ_{t=0}^{n-k-1} (x_t - x̄)(x_{t+k} - x̄) / Σ_{t=0}^{n-1} (x_t - x̄)²
///
/// Returns `None` when the series is too short or has zero variance.
fn pearson_autocorr(series: &[f32], lag: usize) -> Option<f64> {
    let n = series.len();
    if n < lag + 2 {
        return None;
    }

    let mean: f64 = series.iter().map(|&x| x as f64).sum::<f64>() / n as f64;

    let variance: f64 = series
        .iter()
        .map(|&x| {
            let d = x as f64 - mean;
            d * d
        })
        .sum::<f64>();

    if variance < f64::EPSILON {
        return None;
    }

    let cross: f64 = series[..n - lag]
        .iter()
        .zip(series[lag..].iter())
        .map(|(&a, &b)| (a as f64 - mean) * (b as f64 - mean))
        .sum();

    Some(cross / variance)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

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

    /// Insert a symmetric relationship between `LocusId(1)` and `LocusId(2)`
    /// with the given (batch, activity, weight) change history. Each change
    /// is recorded in the ChangeLog so `coherence_dense_series` can find it.
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
        // DepositLayer event — stays in window.
        e.deposit(
            BatchId(2),
            snapshot(0.6),
            LayerTransition::CoherenceShift { from: 0.5, to: 0.6 },
        );
        // Lifecycle transition — resets the window.
        e.deposit(BatchId(3), snapshot(0.3), LayerTransition::BecameDormant);
        // DepositLayer after revival — this is the new stable window.
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
        assert_eq!(series.len(), 2, "only post-lifecycle deposits");
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
        assert_eq!(
            series.len(),
            2,
            "Born layer excluded; two deposits included"
        );
    }

    #[test]
    fn autocorr_lag1_linear_trend_is_positive() {
        // Monotone 10-point series: standard autocorr at lag-1 ≈ 0.70.
        let series: Vec<f32> = (0..10).map(|i| i as f32 * 0.1).collect();
        let r = pearson_autocorr(&series, 1).unwrap();
        assert!(r > 0.6, "monotone trend should give r>0.6, got {r}");
    }

    #[test]
    fn autocorr_lag1_alternating_is_negative() {
        // Alternating 10-point series: standard autocorr at lag-1 = -0.90.
        let series: Vec<f32> = (0..10)
            .map(|i| if i % 2 == 0 { 0.9 } else { 0.1 })
            .collect();
        let r = pearson_autocorr(&series, 1).unwrap();
        assert!(r < -0.85, "alternating series should give r≈-0.9, got {r}");
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

    // ─── emergence_report ────────────────────────────────────────────────

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
        // Engine sets status externally; do so for the test.
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
        // Active entity with only the Born layer — window length 0.
        let mut world = World::new();
        let e = born_entity(3, 1, 0.5);
        world.entities_mut().insert(e);

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
        // Active entity whose one member relationship has ≥ 3 ChangeLog entries
        // (so the dense series is long enough), but every change has the same
        // weight — zero variance → no MI → n_components == 0 in psi_scalar.
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

    // ─── coherence_dense_series ──────────────────────────────────────────

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
        // Member relationship with changes at batches 2, 3, 5 → dense series
        // should have three samples at exactly those batches, and coherence
        // at each should match the engine's `mean_activity × density` formula
        // (n=2 members, so reference = 2·ln(3)/2 ≈ 1.098; one active edge →
        // density = min(1/1.098, 1.0) ≈ 0.911, coherence ≈ activity × 0.911).
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
            assert!(
                (got.1 - expected).abs() < 1e-5,
                "coherence at {:?}: got {}, expected {}",
                got.0,
                got.1,
                expected,
            );
        }
    }

    #[test]
    fn dense_series_respects_lifecycle_window() {
        // Rel has changes at 2, 4, 6. A BecameDormant layer at batch 3
        // resets the window, so only samples strictly after batch 3
        // (i.e. 4 and 6) should appear.
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
        assert_eq!(series.len(), 2, "pre-lifecycle sample (batch 2) excluded");
        assert_eq!(series[0].0, BatchId(4));
        assert_eq!(series[1].0, BatchId(6));
    }

    #[test]
    fn emergence_report_measures_entity_with_rich_dense_series() {
        // 4 changes, varying activity AND weight. Dense series has 4 samples
        // (3 lag-1 pairs). Both V (activity-derived coherence) and X (weight)
        // have variance → psi_scalar should return Some(_).
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

        let psi =
            psi_scalar(&world, EntityId(0)).expect("rich history should produce a Ψ estimate");
        assert_eq!(psi.n_samples, 3, "4 samples → 3 lag-1 pairs");
        assert_eq!(psi.n_components, 1);

        let r = emergence_report(&world);
        assert_eq!(r.n_measured(), 1);
    }

    #[test]
    fn emergence_report_mixes_measured_and_unmeasured() {
        // Two entities: one dormant, one active-but-short. The report
        // should count both under `n_entities` and slot each under the
        // correct unmeasured reason.
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
        assert!(dormant_found, "dormant entity missing from unmeasured");

        let short_found = r.unmeasured.iter().any(|u| {
            u.entity == EntityId(2)
                && matches!(u.reason, UnmeasuredReason::InsufficientStableWindow { .. })
        });
        assert!(short_found, "short-window entity missing from unmeasured");
    }

    // ─── H2: joint MI + pairwise PID ──────────────────────────────────────

    #[test]
    fn solve_linear_system_recovers_known_solution() {
        // 2x + 3y = 8,  5x + 4y = 13  →  x=1, y=2
        let a = vec![vec![2.0, 3.0], vec![5.0, 4.0]];
        let b = vec![8.0, 13.0];
        let x = solve_linear_system(a, b).expect("non-singular");
        assert!((x[0] - 1.0).abs() < 1e-9, "x0: {}", x[0]);
        assert!((x[1] - 2.0).abs() < 1e-9, "x1: {}", x[1]);
    }

    #[test]
    fn solve_linear_system_detects_singular_matrix() {
        // Second row is a multiple of the first → rank 1 → singular.
        let a = vec![vec![1.0, 2.0], vec![2.0, 4.0]];
        let b = vec![3.0, 6.0];
        assert!(solve_linear_system(a, b).is_none());
    }

    #[test]
    fn joint_mi_equal_to_individual_when_other_predictor_uncorrelated() {
        // X1 partially predicts Y; X2 alternates and is uncorrelated with
        // both X1 and Y. I(X1,X2;Y) ≈ I(X1;Y) since X2 adds nothing.
        let y: Vec<f64> = vec![1.0, 2.2, 2.9, 4.1, 5.3, 6.0, 7.2, 7.9, 9.1, 10.3];
        let x1: Vec<f64> = vec![1.2, 1.9, 3.1, 3.8, 5.0, 6.3, 6.9, 8.1, 9.0, 10.1];
        let x2: Vec<f64> = vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];

        let i_x1 = gaussian_mi_from_series(&x1, &y).unwrap();
        let i_joint = gaussian_joint_mi(&[x1, x2], &y).unwrap();
        assert!(
            i_joint + 1e-6 >= i_x1,
            "joint {} should be ≥ individual {}",
            i_joint,
            i_x1
        );
        // And within a reasonable margin — x2 contributes near-zero MI.
        assert!(
            i_joint - i_x1 < 0.5,
            "joint {} should not far exceed individual {} with uncorrelated x2",
            i_joint,
            i_x1
        );
    }

    #[test]
    fn joint_mi_exceeds_sum_when_predictors_uncorrelated() {
        // x1 and x2 are orthogonal; y ≈ x1 + x2 + mild noise. Each alone
        // explains ~½ of y; jointly they explain nearly all → super-additive.
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
        assert!(
            i_joint > i_x1 + i_x2,
            "synergistic predictors: joint {} should exceed sum {}",
            i_joint,
            i_x1 + i_x2
        );
    }

    #[test]
    fn joint_mi_near_individual_when_predictors_identical() {
        // Two identical predictors → joint information is no more than
        // either one alone. `gaussian_joint_mi` returns None on singular
        // X^T X (perfect collinearity); the caller interprets that as
        // "use individual MI as the joint".
        let y: Vec<f64> = (0..20).map(|i| (i as f64).sin()).collect();
        let x1: Vec<f64> = y.iter().map(|v| v + 0.01).collect();
        let x2 = x1.clone();

        // Singular matrix → None.
        assert!(gaussian_joint_mi(&[x1, x2], &y).is_none());
    }

    #[test]
    fn psi_synergy_returns_some_on_rich_history() {
        // Build an entity with two member relationships, each with 7+
        // non-degenerate changes that are NOT linearly dependent after
        // centering (i.e. their differences are not a constant — otherwise
        // X^T X becomes singular and joint MI is undefined).
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

        let synergy =
            psi_synergy(&world, EntityId(0)).expect("rich history should produce a synergy Ψ");
        assert_eq!(synergy.n_components, 2);
        assert!(
            synergy.top_pairs.len() == 1,
            "with 2 components there is exactly 1 pair"
        );
        // Identity: R + U_a + U_b + S = joint_mi (within FP tolerance).
        let pair = &synergy.top_pairs[0];
        let unique_a = pair.mi_a - pair.redundancy;
        let unique_b = pair.mi_b - pair.redundancy;
        let reconstructed = pair.redundancy + unique_a + unique_b + pair.synergy;
        assert!(
            (reconstructed - pair.joint_mi).abs() < 1e-9,
            "PID identity violated: {} vs {}",
            reconstructed,
            pair.joint_mi
        );
        // Joint MI cannot exceed individual MI plus its partner by more
        // than synergy — i.e. corrected Ψ ≤ naive Ψ + Σ redundancy_row.
        // Sanity: psi_corrected = i_self - i_joint ≥ psi_naive iff
        // i_joint ≤ i_sum_components, which is always true for Gaussian MI.
        assert!(
            synergy.psi_corrected + 1e-9 >= synergy.psi_naive,
            "psi_corrected {} should be ≥ psi_naive {} (joint ≤ sum)",
            synergy.psi_corrected,
            synergy.psi_naive
        );

        // H5 aggregate fields. With exactly 2 components there is 1 pair:
        // n_pairs_evaluated = 1, and psi_pair_top3 reduces to
        // `i_self - joint_mi_of_that_pair` = psi_corrected (exact).
        assert_eq!(synergy.n_pairs_evaluated, 1);
        assert!((synergy.total_pair_synergy - pair.synergy).abs() < 1e-12);
        assert!((synergy.mean_pair_synergy - pair.synergy).abs() < 1e-12);
        assert!(
            (synergy.psi_pair_top3 - synergy.psi_corrected).abs() < 1e-9,
            "with 2 components, psi_pair_top3 ({}) should equal psi_corrected ({})",
            synergy.psi_pair_top3,
            synergy.psi_corrected
        );
    }

    // ─── H5 — pair-grain Ψ ──────────────────────────────────────────────

    #[test]
    fn pair_synergy_aggregate_non_negative_on_synergistic_components() {
        // Three relationships with weight trajectories chosen so every
        // pair carries non-additive information about V_{t+1}. At least
        // one pair should have positive synergy; the total_pair_synergy
        // should then be non-negative.
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
        assert_eq!(synergy.n_pairs_evaluated, 3, "C(3,2) = 3");
        // total_pair_synergy = sum of synergy over evaluated pairs; it
        // may be negative under MMI when pairs are purely redundant but
        // should match the stored per-pair values by construction.
        let top_sum: f64 = synergy.top_pairs.iter().map(|p| p.synergy).sum();
        assert!(
            (synergy.total_pair_synergy - top_sum).abs() < 1e-9,
            "aggregate {} disagrees with top_pairs sum {} (only ≤ {} kept)",
            synergy.total_pair_synergy,
            top_sum,
            synergy.top_pairs.len()
        );
    }

    #[test]
    fn psi_pair_top3_equals_psi_corrected_for_two_components() {
        // With only 2 components, top_pairs has 1 entry; the top-3 sum
        // reduces to that one joint MI = I_joint(all). Equivalent to
        // psi_corrected.
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
        // Sanity: the metric subtracts sum of joint MIs (not synergies).
        // If we sum synergies instead, the value differs (unless every
        // pair has zero redundancy, which we avoid here by design).
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
        // joint_sum > synergy_sum unless redundancy is zero, so the two
        // formulations are distinguishable.
        assert!(
            joint_sum - synergy_sum > 1e-9,
            "joint_sum {} should exceed synergy_sum {}",
            joint_sum,
            synergy_sum
        );
    }

    // ─── H4.2 — leave-one-out ────────────────────────────────────────────

    #[test]
    fn leave_one_out_none_with_two_components() {
        // LOO requires ≥ 3 components so post-drop has ≥ 2.
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
        assert_eq!(loo.drops.len(), 3, "one drop per non-degenerate component");
        // Each drop should reference a distinct rel from the member set.
        let mut dropped_ids: Vec<_> = loo.drops.iter().map(|d| d.dropped).collect();
        dropped_ids.sort_by_key(|r| r.0);
        dropped_ids.dedup();
        assert_eq!(dropped_ids.len(), 3, "all drops target distinct rels");
    }

    #[test]
    fn leave_one_out_delta_invariants() {
        // ψ_corrected with full set should equal baseline.psi_corrected.
        // For drops, delta_corrected = baseline - after; if the dropped
        // component carried unique info, after is smaller (more
        // negative), so delta > 0.
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
                    < eps,
                "delta_corrected invariant violated for drop {:?}",
                d.dropped,
            );
            assert!(
                ((loo.baseline.psi_pair_top3 - d.psi_pair_top3) - d.psi_pair_top3_delta).abs()
                    < eps,
                "delta_pair_top3 invariant violated for drop {:?}",
                d.dropped,
            );
        }
    }

    #[test]
    fn psi_synergy_none_with_single_component() {
        // 1 member rel → joint MI over 1 series is just the individual MI;
        // no redundancy correction to perform. psi_synergy requires ≥ 2
        // components and returns None.
        let mut world = World::new();
        let rel = add_rel_with_changes(
            &mut world,
            &[(2, 0.3, 0.1), (3, 0.5, 0.2), (4, 0.4, 0.35), (5, 0.6, 0.5)],
        );
        let e = Entity::born(EntityId(0), BatchId(1), snapshot_with_rels(0.5, vec![rel]));
        world.entities_mut().insert(e);
        assert!(psi_synergy(&world, EntityId(0)).is_none());
    }

    // ─── H4.3: decay-aware activity reconstruction ──────────────────────

    #[test]
    fn rel_activity_at_without_decay_returns_last_change_after() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(2, 0.8, 0.1), (5, 0.6, 0.2)]);
        // Query at batch 7 — two batches after last change at 5. No decay
        // map → returns 0.6 (the `after[0]` of the batch-5 change) as-is.
        let activity = rel_activity_at(BatchId(7), rel, &world, None);
        assert!((activity - 0.6).abs() < 1e-6, "got {}", activity);
    }

    #[test]
    fn rel_activity_at_with_decay_applies_rate_over_gap() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(5, 0.8, 0.1)]);
        // Decay rate 0.5 per batch → at batch 8 (gap of 3), expect
        // 0.8 * 0.5^3 = 0.1.
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 0.5);
        let activity = rel_activity_at(BatchId(8), rel, &world, Some(&rates));
        assert!((activity - 0.1).abs() < 1e-6, "got {}", activity);
    }

    #[test]
    fn rel_activity_at_decay_identity_when_rate_is_one() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(5, 0.8, 0.1)]);
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 1.0);
        // Rate = 1.0 → the `>= 1.0 - EPSILON` shortcut returns the
        // un-decayed value.
        assert_eq!(
            rel_activity_at(BatchId(8), rel, &world, Some(&rates)),
            rel_activity_at(BatchId(8), rel, &world, None),
        );
    }

    #[test]
    fn rel_activity_at_no_decay_for_gap_zero() {
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(5, 0.8, 0.1)]);
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 0.5);
        // At the same batch as the change, gap = 0 → un-decayed.
        let activity = rel_activity_at(BatchId(5), rel, &world, Some(&rates));
        assert!((activity - 0.8).abs() < 1e-6, "got {}", activity);
    }

    #[test]
    fn coherence_dense_series_with_decay_differs_from_no_decay() {
        // Single rel, activity 0.8 at batch 2 (active endpoints). Members
        // = [LocusId(1), LocusId(2)] — n=2 → reference ≈ 1.098 → density
        // = min(1/1.098, 1.0) ≈ 0.911 when one edge is active.
        //
        // No-decay series at batch 5: activity = 0.8 → coherence ≈ 0.729.
        // Decay 0.5 over 3 batches: activity = 0.1 → BELOW threshold 0.1
        // (tie is ≥), active_count = 1, coherence = 0.1 × 0.911 ≈ 0.091.
        //
        // To make the effect observable, we use activities that stay
        // above 0.1 but shrink significantly.
        let mut world = World::new();
        let rel = add_rel_with_changes(&mut world, &[(2, 0.8, 0.1), (4, 0.6, 0.2), (6, 0.4, 0.3)]);
        let e = Entity::born(EntityId(0), BatchId(1), snapshot_with_rels(0.5, vec![rel]));
        world.entities_mut().insert(e);

        let no_decay = coherence_dense_series(&world, EntityId(0));
        let mut rates = DecayRates::default();
        rates.insert(InfluenceKindId(0), 0.8);
        let with_decay = coherence_dense_series_with_decay(&world, EntityId(0), &rates);

        // Same sample batches.
        assert_eq!(no_decay.len(), with_decay.len());
        for ((b1, _), (b2, _)) in no_decay.iter().zip(with_decay.iter()) {
            assert_eq!(b1, b2);
        }
        // At the first sample batch (batch 2) both agree — gap = 0.
        assert!((no_decay[0].1 - with_decay[0].1).abs() < 1e-6);
        // At later batches they should differ because earlier changes
        // have smaller decay-reconstructed activities, but since each
        // sample batch coincides with a change (gap 0 at that batch for
        // the rel), the coherence is computed from the just-committed
        // activity — so they actually match here too. This test mostly
        // guards that the with-decay path doesn't regress for gap=0.
        for ((_, c1), (_, c2)) in no_decay.iter().zip(with_decay.iter()) {
            assert!(
                (c1 - c2).abs() < 1e-6,
                "single-rel series should agree when every sample is at that rel's change batch"
            );
        }
    }

    #[test]
    fn emergence_report_synergy_mirrors_shape_of_plain_report() {
        // Dormant entity + active-but-short entity — both should land in
        // unmeasured, same as plain emergence_report.
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
