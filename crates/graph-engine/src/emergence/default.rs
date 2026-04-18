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

use graph_core::{
    BatchId, EmergenceProposal, Entity, EntityId, EntityLayer, EntitySnapshot, EntityStatus,
    LayerTransition, LifecycleCause, LocusId, Relationship, RelationshipId,
};
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
    pub min_activity_threshold: Option<f32>,
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

impl EmergencePerspective for DefaultEmergencePerspective {
    fn recognize(
        &self,
        relationships: &RelationshipStore,
        existing: &EntityStore,
        batch: BatchId,
    ) -> Vec<EmergenceProposal> {
        let threshold = self
            .min_activity_threshold
            .unwrap_or_else(|| auto_activity_threshold(relationships));
        let community = find_communities(relationships, threshold);
        let components = community.components;
        let adj = &community.adj;

        let component_sets: Vec<FxHashSet<LocusId>> = components
            .iter()
            .map(|members| members.iter().copied().collect())
            .collect();

        // Reverse index: locus → component index. O(sum of component sizes).
        let mut locus_to_component: FxHashMap<LocusId, usize> = FxHashMap::default();
        for (idx, comp) in components.iter().enumerate() {
            for &locus in comp {
                locus_to_component.insert(locus, idx);
            }
        }

        // Pass 1: per-active-entity flow analysis.
        let mut decisions: FxHashMap<EntityId, EntityDecision> = FxHashMap::default();
        let mut claims_per_component: FxHashMap<usize, Vec<EntityId>> = FxHashMap::default();

        for entity in existing.active() {
            let mut buckets: FxHashMap<usize, usize> = FxHashMap::default();
            let mut unassigned = 0usize;
            for locus in &entity.current.members {
                match locus_to_component.get(locus) {
                    Some(&c_idx) => *buckets.entry(c_idx).or_default() += 1,
                    None => unassigned += 1,
                }
            }
            let mut significant: Vec<(usize, usize)> = buckets
                .into_iter()
                .filter(|&(_, n)| n >= MIN_SIGNIFICANT_BUCKET)
                .collect();
            // Deterministic order for tests.
            significant.sort_by_key(|&(i, _)| i);

            let decision = match significant.len() {
                0 => EntityDecision::Dormant,
                1 => {
                    let (c_idx, bucket_size) = significant[0];
                    if unassigned > bucket_size {
                        // Entity lost more than it kept — treat the
                        // surviving bucket as an independent Born, not
                        // the entity's continuation. This is the key
                        // guard against subset-attack degeneration.
                        EntityDecision::Dormant
                    } else {
                        claims_per_component
                            .entry(c_idx)
                            .or_default()
                            .push(entity.id);
                        EntityDecision::Claims(c_idx)
                    }
                }
                _ => {
                    let comp_idxs = significant.iter().map(|&(i, _)| i).collect();
                    EntityDecision::Split(comp_idxs)
                }
            };
            decisions.insert(entity.id, decision);
        }

        let mut proposals: Vec<EmergenceProposal> = Vec::new();
        // Components claimed as offspring of a Split — they are NOT independent
        // `Born` / `Merge` candidates in pass 2 (the Split proposal itself
        // creates their entities via `Entity::born`).
        let mut split_offspring_components: FxHashSet<usize> = FxHashSet::default();

        // Emit Dormant / Split proposals from pass 1.
        for (&entity_id, decision) in &decisions {
            match decision {
                EntityDecision::Dormant => {
                    let decayed: Vec<RelationshipId> = existing
                        .get(entity_id)
                        .map(|e| {
                            e.current
                                .member_relationships
                                .iter()
                                .copied()
                                .filter(|rid| {
                                    relationships
                                        .get(*rid)
                                        .map(|r| r.activity() < threshold)
                                        .unwrap_or(true)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    proposals.push(EmergenceProposal::Dormant {
                        entity: entity_id,
                        cause: LifecycleCause::RelationshipDecay {
                            decayed_relationships: decayed,
                        },
                    });
                }
                EntityDecision::Split(comp_idxs) => {
                    let mut offspring = Vec::with_capacity(comp_idxs.len());
                    for &i in comp_idxs {
                        let (coh, rels) = component_stats(&component_sets[i], adj, threshold);
                        offspring.push((components[i].clone(), rels, coh));
                        split_offspring_components.insert(i);
                    }
                    proposals.push(EmergenceProposal::Split {
                        source: entity_id,
                        offspring,
                        cause: LifecycleCause::ComponentSplit {
                            weak_bridges: Vec::new(),
                        },
                    });
                }
                EntityDecision::Claims(_) => {}
            }
        }

        // Pass 2: component resolution.
        for (c_idx, members) in components.iter().enumerate() {
            if split_offspring_components.contains(&c_idx) {
                continue;
            }
            let claimers = claims_per_component
                .get(&c_idx)
                .cloned()
                .unwrap_or_default();
            let (coherence, member_rels) = component_stats(&component_sets[c_idx], adj, threshold);

            match claimers.len() {
                0 => {
                    // No active entity claims this component. Try to match
                    // against a dormant entity with a majority overlap (Revive);
                    // otherwise a fresh Born.
                    let member_set = &component_sets[c_idx];
                    let dormant_match = existing
                        .iter()
                        .filter(|e| e.status == EntityStatus::Dormant)
                        .filter_map(|e| {
                            let overlap = e
                                .current
                                .members
                                .iter()
                                .filter(|l| member_set.contains(l))
                                .count();
                            if overlap >= MIN_SIGNIFICANT_BUCKET
                                && overlap * 2 >= e.current.members.len()
                            {
                                Some((e.id, overlap))
                            } else {
                                None
                            }
                        })
                        .max_by_key(|&(_, o)| o);
                    if let Some((dormant_id, _)) = dormant_match {
                        let snapshot = EntitySnapshot {
                            members: members.clone(),
                            member_relationships: member_rels.clone(),
                            coherence,
                        };
                        proposals.push(EmergenceProposal::Revive {
                            entity: dormant_id,
                            snapshot,
                            cause: LifecycleCause::RelationshipCluster {
                                key_relationships: member_rels,
                            },
                        });
                    } else {
                        proposals.push(EmergenceProposal::Born {
                            members: members.clone(),
                            member_relationships: member_rels.clone(),
                            coherence,
                            parents: Vec::new(),
                            cause: LifecycleCause::RelationshipCluster {
                                key_relationships: member_rels,
                            },
                        });
                    }
                }
                1 => {
                    let entity_id = claimers[0];
                    let entity = existing
                        .get(entity_id)
                        .expect("claimers contain only live active entity ids");
                    let snapshot = EntitySnapshot {
                        members: members.clone(),
                        member_relationships: member_rels,
                        coherence,
                    };
                    if snapshot_changed(entity, &snapshot) {
                        let transition = membership_delta(entity, &snapshot);
                        proposals.push(EmergenceProposal::DepositLayer {
                            entity: entity_id,
                            layer: EntityLayer::new(batch, snapshot, transition),
                        });
                    }
                }
                _ => {
                    // Multiple active entities converge on one component → Merge.
                    // Survivor: whichever had the most members pre-merge
                    // (largest identity absorbs smaller ones).
                    let into = *claimers
                        .iter()
                        .max_by_key(|id| {
                            existing
                                .get(**id)
                                .map(|e| e.current.members.len())
                                .unwrap_or(0)
                        })
                        .expect("claimers non-empty in ≥2 branch");
                    let absorbed: Vec<EntityId> =
                        claimers.iter().copied().filter(|id| *id != into).collect();
                    proposals.push(EmergenceProposal::Merge {
                        absorbed: absorbed.clone(),
                        into,
                        new_members: members.clone(),
                        member_relationships: member_rels,
                        coherence,
                        cause: LifecycleCause::MergedFrom { absorbed },
                    });
                }
            }
        }

        proposals
    }
}

// --- helpers ---------------------------------------------------------------

/// Adjacency entry: neighbor locus, relationship id, activity weight.
type AdjEntry = (LocusId, RelationshipId, f32);

/// Adjacency list built once per `find_communities` call, reused by
/// both label propagation and `component_stats`.
type AdjMap = rustc_hash::FxHashMap<LocusId, Vec<AdjEntry>>;

/// Result of `find_communities`: the communities plus the adjacency
/// list so `component_stats` can compute coherence + rel_ids without
/// re-scanning the RelationshipStore.
struct CommunityResult {
    components: Vec<Vec<LocusId>>,
    adj: AdjMap,
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
fn find_communities(store: &RelationshipStore, threshold: f32) -> CommunityResult {
    use rustc_hash::FxHashMap;

    // Phase 1: build LocusId adjacency map (returned in CommunityResult
    // for reuse by component_stats — do not change the output type).
    let mut adj: AdjMap = FxHashMap::default();
    for rel in store.iter() {
        if rel.activity().abs() < threshold {
            continue;
        }
        let (a, b) = endpoints_pair(rel);
        let w = rel.activity() + rel.weight();
        adj.entry(a).or_default().push((b, rel.id, w));
        adj.entry(b).or_default().push((a, rel.id, w));
    }

    if adj.is_empty() {
        return CommunityResult {
            components: Vec::new(),
            adj,
        };
    }

    // Phase 2: assign each LocusId a dense local index so label propagation
    // can use Vec<usize> instead of FxHashMap<LocusId, LocusId>.
    // all_loci is sorted so index 0 = smallest LocusId (tie-break matches old behavior).
    let mut all_loci: Vec<LocusId> = adj.keys().copied().collect();
    all_loci.sort();
    let n = all_loci.len();

    let mut locus_to_idx: FxHashMap<LocusId, usize> =
        FxHashMap::with_capacity_and_hasher(n, Default::default());
    for (i, &id) in all_loci.iter().enumerate() {
        locus_to_idx.insert(id, i);
    }

    // Convert adjacency to local-index form: (neighbor_idx, rel_id, weight).
    // Stored as flat Vec<Vec<(usize, RelationshipId, f32)>> indexed by local idx.
    let local_adj: Vec<Vec<(usize, RelationshipId, f32)>> = all_loci
        .iter()
        .map(|id| {
            adj[id]
                .iter()
                .map(|&(nb, rid, w)| (locus_to_idx[&nb], rid, w))
                .collect()
        })
        .collect();

    // Phase 3: label propagation with Vec scratch buffers.
    // labels[i] = local label index for node i.
    // label_weight[l] = accumulated vote weight for label l (zeroed between nodes).
    // seen[l] = true if label_weight[l] was written (avoids scanning full Vec to reset).
    let mut labels: Vec<usize> = (0..n).collect();
    let mut label_weight: Vec<f32> = vec![0.0; n];
    let mut seen: Vec<bool> = vec![false; n];
    let mut dirty_labels: Vec<usize> = Vec::new(); // labels touched this node

    let mut order: Vec<usize> = (0..n).collect();
    let mut lcg_state: u64 = 0x517cc1b727220a95;

    const MAX_ITER: usize = 15;
    for _ in 0..MAX_ITER {
        for i in (1..n).rev() {
            lcg_state = lcg_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (lcg_state >> 33) as usize % (i + 1);
            order.swap(i, j);
        }

        let mut changed = false;
        for &node in &order {
            let neighbors = &local_adj[node];
            dirty_labels.clear();

            for &(nb, _, w) in neighbors {
                let lbl = labels[nb];
                if !seen[lbl] {
                    seen[lbl] = true;
                    dirty_labels.push(lbl);
                }
                label_weight[lbl] += w;
            }

            // Tie-break: prefer smaller local index (= smaller LocusId, matching old behavior).
            let best_label = dirty_labels
                .iter()
                .copied()
                .max_by(|&a, &b| {
                    label_weight[a]
                        .partial_cmp(&label_weight[b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(b.cmp(&a))
                })
                .unwrap_or(labels[node]);

            // Reset scratch.
            for &lbl in &dirty_labels {
                label_weight[lbl] = 0.0;
                seen[lbl] = false;
            }

            if labels[node] != best_label {
                labels[node] = best_label;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Phase 4: group nodes by label → LocusId components.
    let mut groups: FxHashMap<usize, Vec<LocusId>> = FxHashMap::default();
    for (i, &lbl) in labels.iter().enumerate() {
        groups.entry(lbl).or_default().push(all_loci[i]);
    }
    let mut components: Vec<Vec<LocusId>> = groups.into_values().collect();
    for c in &mut components {
        c.sort();
    }
    components.sort_by(|a, b| a[0].0.cmp(&b[0].0));
    CommunityResult { components, adj }
}

fn endpoints_pair(rel: &Relationship) -> (LocusId, LocusId) {
    use graph_core::Endpoints;
    match &rel.endpoints {
        Endpoints::Directed { from, to } => (*from, *to),
        Endpoints::Symmetric { a, b } => (*a, *b),
    }
}

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
fn component_stats(
    member_set: &rustc_hash::FxHashSet<LocusId>,
    adj: &AdjMap,
    threshold: f32,
) -> (f32, Vec<RelationshipId>) {
    let mut sum = 0.0f32;
    let mut active_count = 0usize;
    let mut rel_ids = Vec::new();
    for &locus in member_set {
        if let Some(neighbors) = adj.get(&locus) {
            for &(nb, rel_id, activity) in neighbors {
                if nb > locus && member_set.contains(&nb) {
                    rel_ids.push(rel_id);
                    // Only excitatory relationships contribute to coherence.
                    // Inhibitory edges (negative activity) are part of the
                    // topology but do not add to internal binding strength.
                    if activity >= threshold {
                        sum += activity;
                        active_count += 1;
                    }
                }
            }
        }
    }
    let mean_activity = if active_count == 0 {
        0.0
    } else {
        sum / active_count as f32
    };
    // Reference edge count: n * ln(n+1) / 2.
    // Sub-quadratic so large sparse graphs score proportionally, not near 0.
    let n = member_set.len();
    let reference = if n <= 1 {
        1.0f32
    } else {
        (n as f32) * ((n as f32 + 1.0).ln()) / 2.0
    };
    let density = (active_count as f32 / reference).min(1.0);
    let coherence = mean_activity * density;
    (coherence, rel_ids)
}

// `overlap` / `all_matches` removed with locus-flow algorithm — see docstring.

fn snapshot_changed(entity: &Entity, new: &EntitySnapshot) -> bool {
    if entity.current.members != new.members {
        return true;
    }
    (entity.current.coherence - new.coherence).abs() > 0.05
}

fn membership_delta(entity: &Entity, new: &EntitySnapshot) -> LayerTransition {
    let old = &entity.current.members;
    let added: Vec<LocusId> = new
        .members
        .iter()
        .filter(|m| !old.contains(m))
        .copied()
        .collect();
    let removed: Vec<LocusId> = old
        .iter()
        .filter(|m| !new.members.contains(m))
        .copied()
        .collect();
    if added.is_empty() && removed.is_empty() {
        LayerTransition::CoherenceShift {
            from: entity.current.coherence,
            to: new.coherence,
        }
    } else {
        LayerTransition::MembershipDelta { added, removed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::Relationship;
    use graph_core::{
        Endpoints, InfluenceKindId, KindObservation, LocusId, RelationshipLineage, StateVector,
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

        let (coherence, rel_ids) = component_stats(&member_set, &adj, 0.1);

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

        let result = find_communities(&store, 0.1);

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

        let (coherence, _) = component_stats(&member_set, &adj, 0.1);

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
