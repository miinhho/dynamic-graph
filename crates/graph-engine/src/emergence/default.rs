//! Default perspective: **locus-flow reconciliation**.
//!
//! The engine already has per-batch ground truth about where each locus is —
//! `EntityStore.by_member` maps it to the entity (or entities) whose
//! `current.members` included it at the last recognize. Instead of summarising
//! set relationships via Jaccard (which requires a tuning knob — see
//! `docs/complexity-audit.md` Finding 2b), we directly observe the flow:
//!
//! 1. Run weighted label propagation to find the current connected components.
//! 2. For each *active* entity, bucket its members by which component they
//!    landed in (plus an `unassigned` bucket for members absent from any
//!    component). The distribution of bucket sizes determines the entity's
//!    fate deterministically:
//!    - ≥ 2 significant buckets → **Split** (offspring = the components it
//!      fans out into).
//!    - 1 significant bucket, `unassigned > bucket` → **Dormant** (the entity
//!      has lost more members than it retains; the bucket's component gets a
//!      fresh `Born`).
//!    - 1 significant bucket, `unassigned ≤ bucket` → the entity *claims* that
//!      component (continuation).
//!    - 0 significant buckets → **Dormant**.
//! 3. For each component, count how many *active* entities claimed it. ≥ 2
//!    claimers collapse into a **Merge**; 1 claimer becomes a `DepositLayer`
//!    with the appropriate `MembershipDelta`/`CoherenceShift` transition;
//!    0 claimers fall back to **Revive** (if a dormant entity overlaps
//!    majority of the component) or **Born**.
//!
//! No `overlap_threshold` knob: the only constant is `MIN_SIGNIFICANT_BUCKET`
//! (size 2 — drift tolerance for 1 locus noise), which is hard-coded because
//! 1-locus drift is universally meaningless.

mod community;
mod proposals;

use std::sync::atomic::{AtomicUsize, Ordering};

/// Diagnostic-only: last component count observed in `recognize`.
/// Lets tests distinguish detection-layer fragmentation from matching failure
/// without threading returns through the trait surface.
static LAST_COMPONENT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[doc(hidden)]
pub fn debug_last_component_count() -> usize {
    LAST_COMPONENT_COUNT.load(Ordering::Relaxed)
}

/// Diagnostic-only: read the exclusivity-filter counters.
/// Returns `(unchanged, filtered, collapsed)`. Tests call
/// `reset_exclusivity_counters` before a run and read this after.
/// See `docs/hep-ph-finding.md §4` for the investigation these counters support.
#[doc(hidden)]
pub fn debug_exclusivity_counters() -> (usize, usize, usize) {
    proposals::exclusivity_counters()
}

#[doc(hidden)]
pub fn reset_exclusivity_counters() {
    proposals::reset_exclusivity_counters();
}

use graph_core::{BatchId, EmergenceProposal, EntityId, LocusId, RelationshipId};
use graph_world::{EntityStore, RelationshipStore};
use rustc_hash::{FxHashMap, FxHashSet};

use super::EmergencePerspective;

/// Default perspective: locus-flow reconciliation.
#[derive(Debug, Clone, Default)]
pub struct DefaultEmergencePerspective {
    /// Minimum relationship activity score to include an edge in the
    /// clustering graph during `recognize()`.
    ///
    /// **Default**: `None` — distribution-based auto. Adopted in Phase 2
    /// of the 2026-04-18 complexity sweep (`docs/complexity-audit.md`)
    /// after the old karate-tuned `0.1` constant collapsed every node
    /// into one community on Davis Southern Women (Finding 1, 91%
    /// density).
    ///
    /// **Override when**: the auto-computed gap heuristic picks the
    /// wrong cutoff for your activity distribution (e.g. a unimodal
    /// distribution where every edge is meaningful but the gap
    /// detector still fires). Leave `None` unless a benchmark
    /// demonstrates the override is necessary — the auto path has
    /// carried both karate and Davis without hand-tuning.
    ///
    /// **None semantics**: per `recognize()` call, scan nonzero absolute
    /// activities, find the largest relative gap in the lower half of
    /// the sorted list, and threshold just below it if the gap is
    /// `≥ 2×` (bimodal signal-vs-noise). Otherwise threshold is `0`
    /// (label-propagation's weighted voting does the filtering). See
    /// `auto_activity_threshold`. Setting `Some(x)` pins the threshold
    /// and bypasses the auto detector entirely.
    min_activity_threshold: Option<f32>,
}

/// Distribution-aware activity threshold.
///
/// Strategy: detect bimodal activity distributions (which arise from
/// signal vs. noise separation) via the largest **relative gap** between
/// sorted activities. If a clear gap exists (ratio ≥ 2×), place the
/// threshold just below it — noise below, signal above. Otherwise return
/// effectively 0 so label-propagation's weighted voting does the filtering
/// (sparse graphs with uniform weights shouldn't be thresholded).
///
/// This replaces the old `p25` heuristic, which worked for Davis's bimodal
/// co-attendance counts but wrongly excluded low-weight edges in karate
/// (a unimodal structural-weight distribution), isolating nodes and
/// breaking node-coverage assertions.
fn auto_activity_threshold(store: &RelationshipStore) -> f32 {
    let mut activities: Vec<f32> = store
        .iter()
        .map(|r| r.activity().abs())
        .filter(|&a| a > 0.0)
        .collect();
    if activities.len() < 4 {
        return 0.0;
    }
    activities.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Find the largest relative gap in the lower 75% of the distribution.
    // Noise is below signal; a gap in the upper tail is natural variance,
    // not a threshold boundary. The window was widened from 50% → 75%
    // after the SocioPatterns benchmark (2026-04-18) showed that heavy
    // noise floors (many low-activity ghost edges) push the true bimodal
    // cut above p50 — a 50% window was finding noise-internal gaps and
    // collapsing the partition. p75 captures the real signal/noise
    // boundary without reaching into the upper-tail natural variance.
    let search_end = activities.len() * 3 / 4 + 1;
    let mut best_gap_ratio = 1.0f32;
    let mut best_threshold = 0.0f32;
    for i in 0..search_end.saturating_sub(1) {
        let a = activities[i];
        let b = activities[i + 1];
        if a < 1e-6 {
            continue;
        }
        let ratio = b / a;
        if ratio > best_gap_ratio {
            best_gap_ratio = ratio;
            // Place threshold just above the "noise" side so `>=` in the
            // filter includes the signal side.
            best_threshold = a * 1.0001;
        }
    }
    // Require a meaningful gap (≥ 2×) before thresholding at all.
    if best_gap_ratio >= 2.0 {
        best_threshold
    } else {
        0.0
    }
}

/// Bucket size below which a membership bucket is treated as noise.
/// Hard-coded because "1 locus drift" is universally meaningless and any
/// larger value would require domain-specific tuning we actively want to
/// avoid. Matches the intuition: a locus pair is the smallest "meaningful
/// community" fragment.
const MIN_SIGNIFICANT_BUCKET: usize = 2;

/// Per-active-entity verdict from the flow analysis.
enum EntityDecision {
    /// Entity lost enough of its membership that it is no longer coherently
    /// present in any single component. Triggers `BecameDormant`.
    Dormant,
    /// Entity's members fanned out across multiple components. Each listed
    /// component becomes an offspring (new `Born`) of a `Split`.
    Split(Vec<usize>),
    /// Entity claims this component as its continuation. Resolution (whether
    /// this becomes a `DepositLayer`, or collapses with other claimers into
    /// a `Merge`) happens in pass 2.
    Claims(usize),
}

struct CommunityIndex {
    component_sets: Vec<FxHashSet<LocusId>>,
    locus_to_component: FxHashMap<LocusId, usize>,
}

pub(super) struct ProposalContext<'a> {
    existing: &'a EntityStore,
    relationships: &'a RelationshipStore,
    threshold: f32,
    proposals: &'a mut Vec<EmergenceProposal>,
    /// Loci already claimed by a Claims-decision active entity this tick.
    /// Used by `resolve_component_proposal` to enforce single-perspective
    /// membership exclusivity on new Born proposals (redesign §3.4).
    /// Without this gate, high-degree hub loci accumulate as members of
    /// every subfield community — HEP-PH Finding 5 (`docs/hep-ph-finding.md`).
    owned_loci: &'a FxHashSet<LocusId>,
}

struct RecognitionContext<'a> {
    batch: BatchId,
    existing: &'a EntityStore,
    relationships: &'a RelationshipStore,
    threshold: f32,
}

struct RecognitionState {
    components: Vec<Vec<LocusId>>,
    adj: AdjMap,
    community_index: CommunityIndex,
    flow: FlowAnalysis,
}

struct FlowAnalysis {
    decisions: FxHashMap<EntityId, EntityDecision>,
    claims_per_component: FxHashMap<usize, Vec<EntityId>>,
}

struct ComponentProposalPrep {
    component_idx: usize,
    coherence: f32,
    member_rels: Vec<RelationshipId>,
    claimers: Vec<EntityId>,
}

struct EntityBuckets {
    significant: Vec<(usize, usize)>,
    unassigned: usize,
}

fn build_community_index(components: &[Vec<LocusId>]) -> CommunityIndex {
    let component_sets: Vec<FxHashSet<LocusId>> = components
        .iter()
        .map(|members| members.iter().copied().collect())
        .collect();

    let mut locus_to_component: FxHashMap<LocusId, usize> = FxHashMap::default();
    for (idx, comp) in components.iter().enumerate() {
        for &locus in comp {
            locus_to_component.insert(locus, idx);
        }
    }

    CommunityIndex {
        component_sets,
        locus_to_component,
    }
}

fn analyze_entity_flows(
    existing: &EntityStore,
    locus_to_component: &FxHashMap<LocusId, usize>,
) -> FlowAnalysis {
    let decisions = collect_entity_decisions(existing, locus_to_component);
    let claims_per_component = collect_component_claims(existing, &decisions);

    FlowAnalysis {
        decisions,
        claims_per_component,
    }
}

fn collect_entity_decisions(
    existing: &EntityStore,
    locus_to_component: &FxHashMap<LocusId, usize>,
) -> FxHashMap<EntityId, EntityDecision> {
    existing
        .active()
        .map(|entity| {
            let buckets =
                bucket_entity_members(entity.current.members.as_slice(), locus_to_component);
            (entity.id, decide_entity_flow(&buckets))
        })
        .collect()
}

fn collect_component_claims(
    existing: &EntityStore,
    decisions: &FxHashMap<EntityId, EntityDecision>,
) -> FxHashMap<usize, Vec<EntityId>> {
    let mut claims_per_component: FxHashMap<usize, Vec<EntityId>> = FxHashMap::default();

    for entity in existing.active() {
        if let Some(EntityDecision::Claims(component_idx)) = decisions.get(&entity.id) {
            claims_per_component
                .entry(*component_idx)
                .or_default()
                .push(entity.id);
        }
    }

    claims_per_component
}

fn bucket_entity_members(
    members: &[LocusId],
    locus_to_component: &FxHashMap<LocusId, usize>,
) -> EntityBuckets {
    let mut buckets: FxHashMap<usize, usize> = FxHashMap::default();
    let mut unassigned = 0usize;
    for locus in members {
        match locus_to_component.get(locus) {
            Some(&c_idx) => *buckets.entry(c_idx).or_default() += 1,
            None => unassigned += 1,
        }
    }

    let mut significant: Vec<(usize, usize)> = buckets
        .into_iter()
        .filter(|&(_, n)| n >= MIN_SIGNIFICANT_BUCKET)
        .collect();
    significant.sort_by_key(|&(i, _)| i);

    EntityBuckets {
        significant,
        unassigned,
    }
}

fn decide_entity_flow(buckets: &EntityBuckets) -> EntityDecision {
    match buckets.significant.len() {
        0 => EntityDecision::Dormant,
        1 => {
            let (c_idx, bucket_size) = buckets.significant[0];
            if buckets.unassigned > bucket_size {
                EntityDecision::Dormant
            } else {
                EntityDecision::Claims(c_idx)
            }
        }
        _ => EntityDecision::Split(buckets.significant.iter().map(|&(i, _)| i).collect()),
    }
}

impl DefaultEmergencePerspective {
    /// Override the distribution-based auto-threshold. Only use when the
    /// auto heuristic picks the wrong cut for your specific activity
    /// distribution. Leave unset for standard workloads.
    pub fn with_min_activity_threshold(mut self, threshold: f32) -> Self {
        self.min_activity_threshold = Some(threshold);
        self
    }

    fn build_recognition_context<'a>(
        &self,
        relationships: &'a RelationshipStore,
        existing: &'a EntityStore,
        batch: BatchId,
    ) -> RecognitionContext<'a> {
        RecognitionContext {
            batch,
            existing,
            relationships,
            threshold: self.resolve_activity_threshold(relationships),
        }
    }

    fn resolve_activity_threshold(&self, relationships: &RelationshipStore) -> f32 {
        self.min_activity_threshold
            .unwrap_or_else(|| auto_activity_threshold(relationships))
    }
}

fn prepare_recognition(context: &RecognitionContext<'_>) -> RecognitionState {
    let community = community::find_communities(context.relationships, context.threshold);
    let community_index = build_community_index(&community.components);
    let flow = analyze_entity_flows(context.existing, &community_index.locus_to_component);

    RecognitionState {
        components: community.components,
        adj: community.adj,
        community_index,
        flow,
    }
}

fn prepare_component_proposals(
    state: &RecognitionState,
    split_offspring_components: &FxHashSet<usize>,
    threshold: f32,
) -> Vec<ComponentProposalPrep> {
    state
        .components
        .iter()
        .enumerate()
        .filter(|(component_idx, _)| !split_offspring_components.contains(component_idx))
        .map(|(component_idx, _)| {
            let (coherence, member_rels) = community::component_stats(
                &state.community_index.component_sets[component_idx],
                &state.adj,
                threshold,
            );
            let claimers = state
                .flow
                .claims_per_component
                .get(&component_idx)
                .cloned()
                .unwrap_or_default();

            ComponentProposalPrep {
                component_idx,
                coherence,
                member_rels,
                claimers,
            }
        })
        .collect()
}

/// Collect loci belonging to active entities that will continue this tick
/// (Claims decision). These loci are excluded from new Born proposals to
/// enforce single-perspective membership exclusivity (redesign §3.4).
///
/// Split-source entities are intentionally excluded: their members are being
/// redistributed to offspring entities and should not block those offspring
/// from being Born. Dormant-decision entities are also excluded for the same
/// reason (they will no longer be active after this tick).
fn collect_claimed_loci(
    decisions: &FxHashMap<EntityId, EntityDecision>,
    existing: &EntityStore,
) -> FxHashSet<LocusId> {
    let mut owned: FxHashSet<LocusId> = FxHashSet::default();
    for (&entity_id, decision) in decisions {
        if !matches!(decision, EntityDecision::Claims(_)) {
            continue;
        }
        if let Some(entity) = existing.get(entity_id) {
            owned.extend(entity.current.members.iter().copied());
        }
    }
    owned
}

fn emit_component_proposals(
    state: &RecognitionState,
    prepared: &[ComponentProposalPrep],
    context: &mut ProposalContext<'_>,
    batch: BatchId,
) {
    for component in prepared {
        proposals::resolve_component_proposal(
            batch,
            &state.components[component.component_idx],
            &state.community_index.component_sets[component.component_idx],
            component.claimers.as_slice(),
            component.member_rels.clone(),
            component.coherence,
            context,
        );
    }
}

impl EmergencePerspective for DefaultEmergencePerspective {
    fn recognize(
        &self,
        relationships: &RelationshipStore,
        existing: &EntityStore,
        batch: BatchId,
    ) -> Vec<EmergenceProposal> {
        let recognition = self.build_recognition_context(relationships, existing, batch);
        let state = prepare_recognition(&recognition);
        LAST_COMPONENT_COUNT.store(state.components.len(), Ordering::Relaxed);

        let mut proposals: Vec<EmergenceProposal> = Vec::new();
        let owned_loci = collect_claimed_loci(&state.flow.decisions, recognition.existing);
        let mut context = ProposalContext {
            existing: recognition.existing,
            relationships: recognition.relationships,
            threshold: recognition.threshold,
            proposals: &mut proposals,
            owned_loci: &owned_loci,
        };
        let split_offspring_components = proposals::emit_entity_proposals(
            &state.flow.decisions,
            &state.components,
            &state.community_index.component_sets,
            &state.adj,
            &mut context,
        );
        let component_proposals =
            prepare_component_proposals(&state, &split_offspring_components, recognition.threshold);

        emit_component_proposals(
            &state,
            &component_proposals,
            &mut context,
            recognition.batch,
        );

        proposals
    }
}

// --- helpers ---------------------------------------------------------------

/// Adjacency entry: neighbor locus, relationship id, activity weight.
type AdjEntry = (LocusId, RelationshipId, f32);

/// Dense adjacency entry used by label propagation after `LocusId`s are
/// rewritten to stable local indices.
type LocalAdjEntry = (usize, RelationshipId, f32);

/// Adjacency list built once per `find_communities` call, reused by
/// both label propagation and `component_stats`.
type AdjMap = rustc_hash::FxHashMap<LocusId, Vec<AdjEntry>>;

/// Result of `find_communities`: the communities plus the adjacency
/// list so `component_stats` can compute coherence + rel_ids without
/// re-scanning the RelationshipStore.
pub(super) struct CommunityResult {
    components: Vec<Vec<LocusId>>,
    adj: AdjMap,
}

pub(super) struct LocalCommunityGraph {
    all_loci: Vec<LocusId>,
    local_adj: Vec<Vec<LocalAdjEntry>>,
}

/// Weighted label propagation over the relationship graph.
///
/// Each node starts with its own label. In each iteration, a node
/// adopts the label with the highest total signed activity weight among
/// its neighbors.  Positive-activity edges act as **attraction** (they
/// pull nodes toward the same label), while negative-activity edges act
/// as **repulsion** (they push nodes toward different labels).
///
/// Repulsion is implemented by treating a negative-weight neighbor's
/// label as a negative vote: its contribution subtracts from the score
/// of that label rather than adding to it.  The propagation step still
/// picks the label with the highest net score — a node surrounded by
/// strong positive neighbors clusters with them; a node connected to
/// inhibitory edges tends to end up in a different community.
///
/// Converges when no labels change, or after `max_iter` rounds.
///
/// Implementation note: label propagation uses dense `Vec` storage
/// (local index → label index) rather than `HashMap` to eliminate
/// per-edge hash lookups in the hot inner loop.
/// Compute coherence and member relationship ids for a component using
/// the pre-built adjacency list.
///
/// Coherence = `mean_activity × density`, where density uses a log-scaled
/// reference that avoids the O(n²) penalty of the fully-connected baseline.
///
/// **Why not `n*(n-1)/2`?**  Real-world graphs (biological connectomes,
/// social networks) are sparse: edge count grows as O(n) or O(n log n),
/// not O(n²). Dividing by `n*(n-1)/2` makes any large sparse cluster
/// score near 0 simply because it exists — which destroys the signal.
///
/// **Reference formula**: `n * ln(n+1) / 2`.
/// - Grows sub-quadratically (≈ O(n log n)), matching empirical sparse
///   graph densities.
/// - For n=2 fully-connected (1 edge): `density ≈ 1/ln(3) ≈ 0.91` — close
///   to 1.0, preserving the "tight pair" signal.
/// - For n=84 with 300 active edges (biological connectome density):
///   reference ≈ 186 → `density ≈ min(300/186, 1.0) = 1.0`.
/// - For n=27 with 30 edges: reference ≈ 45 → `density ≈ 0.67`.
// `overlap` / `all_matches` removed with locus-flow algorithm — see docstring.

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, Entity, EntitySnapshot, InfluenceKindId, KindObservation, LocusId, Relationship,
        RelationshipLineage, StateVector,
    };
    use graph_world::{EntityStore, RelationshipStore};

    fn rel(store: &mut RelationshipStore, from: u64, to: u64, activity: f32) {
        let id = store.mint_id();
        store.insert(Relationship {
            id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[activity]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(InfluenceKindId(1))],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    #[test]
    fn finds_two_components_from_disconnected_pairs() {
        let mut store = RelationshipStore::new();
        rel(&mut store, 1, 2, 1.0);
        rel(&mut store, 3, 4, 1.0);

        let perspective = DefaultEmergencePerspective::default();
        let entities = EntityStore::new();
        let proposals = perspective.recognize(&store, &entities, BatchId(0));

        let born_count = proposals
            .iter()
            .filter(|p| matches!(p, EmergenceProposal::Born { .. }))
            .count();
        assert_eq!(born_count, 2, "{proposals:?}");
    }

    // Old test `low_activity_edge_excluded_from_clustering` removed with
    // Phase 2 (threshold auto-tune): "low" is now relative, not absolute,
    // and a world containing only 0.05-activity edges treats 0.05 as the
    // baseline. Behaviour now tested at test level with realistic
    // distributions (see lfr_dynamic.rs).

    #[test]
    fn continuation_produces_deposit_layer_not_new_born() {
        let mut rel_store = RelationshipStore::new();
        rel(&mut rel_store, 1, 2, 1.0);

        let mut entity_store = EntityStore::new();
        let eid = entity_store.mint_id();
        let snapshot = EntitySnapshot {
            members: vec![LocusId(1), LocusId(2)],
            member_relationships: vec![],
            coherence: 1.0,
        };
        entity_store.insert(Entity::born(eid, BatchId(0), snapshot));

        let perspective = DefaultEmergencePerspective::default();
        let proposals = perspective.recognize(&rel_store, &entity_store, BatchId(1));

        let born_count = proposals
            .iter()
            .filter(|p| matches!(p, EmergenceProposal::Born { .. }))
            .count();
        assert_eq!(born_count, 0, "{proposals:?}");
    }

    #[test]
    fn component_stats_triangle_graph() {
        // Triangle: nodes 1, 2, 3 with three bidirectional adj entries.
        // adj stores (neighbor, rel_id, signed_activity) per direction.
        let r0 = RelationshipId(0);
        let r1 = RelationshipId(1);
        let r2 = RelationshipId(2);
        let mut adj: AdjMap = rustc_hash::FxHashMap::default();
        adj.entry(LocusId(1))
            .or_default()
            .extend([(LocusId(2), r0, 0.8), (LocusId(3), r2, 0.7)]);
        adj.entry(LocusId(2))
            .or_default()
            .extend([(LocusId(1), r0, 0.8), (LocusId(3), r1, 0.6)]);
        adj.entry(LocusId(3))
            .or_default()
            .extend([(LocusId(2), r1, 0.6), (LocusId(1), r2, 0.7)]);

        let member_set: rustc_hash::FxHashSet<LocusId> = [LocusId(1), LocusId(2), LocusId(3)]
            .iter()
            .copied()
            .collect();

        let (coherence, rel_ids) = community::component_stats(&member_set, &adj, 0.1);

        // 3 edges above threshold (1-2, 1-3, 2-3); nb > locus dedup gives
        // visits for pairs (1,2), (1,3), (2,3) → active_count = 3.
        // mean_activity = (0.8 + 0.7 + 0.6) / 3 = 0.7
        // reference = 3 * ln(4) / 2 ≈ 2.079; density = 3/2.079 > 1.0 → 1.0
        // coherence = 0.7 * 1.0 = 0.7
        assert_eq!(rel_ids.len(), 3);
        assert!((coherence - 0.7).abs() < 1e-4, "coherence = {coherence}");
    }

    #[test]
    fn find_communities_two_disconnected_pairs() {
        let mut store = RelationshipStore::new();
        rel(&mut store, 1, 2, 1.0);
        rel(&mut store, 3, 4, 1.0);

        let result = community::find_communities(&store, 0.1);

        assert_eq!(result.components.len(), 2);
        for c in &result.components {
            assert_eq!(c.len(), 2);
        }
    }

    // `all_matches` removed with the locus-flow rewrite — Jaccard-based
    // overlap is no longer part of the perspective.

    #[test]
    fn component_stats_high_activity_not_capped() {
        // Activity > 1.0 should flow through without being capped.
        let r0 = RelationshipId(0);
        let mut adj: AdjMap = rustc_hash::FxHashMap::default();
        adj.entry(LocusId(1))
            .or_default()
            .push((LocusId(2), r0, 5.0));
        adj.entry(LocusId(2))
            .or_default()
            .push((LocusId(1), r0, 5.0));
        let member_set: rustc_hash::FxHashSet<LocusId> =
            [LocusId(1), LocusId(2)].iter().copied().collect();

        let (coherence, _) = community::component_stats(&member_set, &adj, 0.1);

        // mean_activity = 5.0; density ≤ 1.0 so coherence = 5.0 * density
        assert!(
            coherence > 1.0,
            "coherence should exceed 1.0 when activity > 1.0, got {coherence}"
        );
    }

    #[test]
    fn active_entity_with_no_component_becomes_dormant() {
        let rel_store = RelationshipStore::new();
        let mut entity_store = EntityStore::new();
        let eid = entity_store.mint_id();
        let snapshot = EntitySnapshot {
            members: vec![LocusId(1)],
            member_relationships: vec![],
            coherence: 1.0,
        };
        entity_store.insert(Entity::born(eid, BatchId(0), snapshot));

        let perspective = DefaultEmergencePerspective::default();
        let proposals = perspective.recognize(&rel_store, &entity_store, BatchId(1));

        let dormant = proposals
            .iter()
            .any(|p| matches!(p, EmergenceProposal::Dormant { entity, .. } if *entity == eid));
        assert!(dormant, "{proposals:?}");
    }
}
