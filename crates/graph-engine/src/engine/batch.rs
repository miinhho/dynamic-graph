//! Batch loop internals: pending-change bookkeeping, per-locus dispatch
//! staging, and the two relationship-graph mutations that fire inside tick.

mod compute;
mod evidence;
mod structural;

use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId, LocusId, LocusKindId,
    Properties, ProposedChange, RelationshipId, RelationshipSlotDef, StateVector,
    StructuralProposal,
};

pub(crate) use compute::{build_computed_change, compute_pending_change};
pub(crate) use evidence::{signed_activity_contribution, EmergenceEvidence};
pub(crate) use structural::apply_structural_proposals;

#[allow(dead_code)]
/// Maximum predecessor-DAG depth to search for feedback loops in STDP.
/// Covers A→B→C→A (2-hop) plus one extra for robustness. Bounded to
/// prevent O(log-size) traversal on deep DAGs.
const STDP_MAX_FEEDBACK_HOPS: u32 = 3;

#[allow(dead_code)]
/// Returns true when `target_locus` appears in the predecessor chain of
/// `start_id` within `max_hops` steps. Uses iterative DFS with a visited
/// set to avoid revisiting nodes in shared-predecessor DAGs.
///
/// Returns false immediately when `start_id` is not in the log (trimmed
/// or not yet committed) or when no path reaches `target_locus` within
/// the hop budget.
fn is_feedback_in_dag(
    log: &graph_world::ChangeLog,
    start_id: ChangeId,
    target_locus: LocusId,
    max_hops: u32,
) -> bool {
    let Some(start) = log.get(start_id) else {
        return false;
    };
    let mut stack: Vec<(ChangeId, u32)> = start
        .predecessors
        .iter()
        .map(|&id| (id, max_hops))
        .collect();
    let mut visited: rustc_hash::FxHashSet<ChangeId> = rustc_hash::FxHashSet::default();
    while let Some((id, hops)) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(change) = log.get(id) else { continue };
        if change.subject == ChangeSubject::Locus(target_locus) {
            return true;
        }
        if hops > 1 {
            stack.extend(change.predecessors.iter().map(|&pid| (pid, hops - 1)));
        }
    }
    false
}

/// A change queued for the next batch: the user/program-supplied proposal
/// plus any predecessor `ChangeId`s the engine derived from the previous
/// batch's commits.
pub(crate) struct PendingChange {
    pub(crate) proposed: ProposedChange,
    pub(crate) derived_predecessors: Vec<ChangeId>,
}

/// Per-locus dispatch input assembled after a batch commit. Holds
/// immutable references into the world valid for the duration of one
/// batch's program-dispatch phase.
pub(crate) struct DispatchInput<'a> {
    pub(crate) locus: &'a graph_core::Locus,
    pub(crate) program: &'a dyn graph_core::LocusProgram,
    pub(crate) inbox: Vec<&'a graph_core::Change>,
    pub(crate) derived: Vec<ChangeId>,
}

/// Output of one locus's program dispatch: proposed state changes,
/// structural topology proposals, and the derived predecessor ids to
/// attach to each follow-up change.
pub(crate) type DispatchResult = (Vec<ProposedChange>, Vec<StructuralProposal>, Vec<ChangeId>);

// ── Compute / Apply split ─────────────────────────────────────────────────
//
// The commit loop is split into two phases so the read-heavy computation
// can run in parallel (on the pre-batch world snapshot) while the write-heavy
// mutations remain sequential.
//
// COMPUTE phase: pure reads → `ComputedChange`
//   - Reads pre-batch locus/relationship state
//   - Computes stabilized state, cross-locus predecessor list, schema checks
//   - Does NOT mint IDs or touch the world
//
// APPLY phase: sequential mutations
//   - Mints ChangeId, appends to log, updates state
//   - Calls `auto_emerge_relationship` for cross-locus flow

/// Timing order of pre vs post synaptic activation, used for STDP.
///
/// In the batch loop, all changes within the same `BatchId` are considered
/// simultaneous.  A cross-locus predecessor chain gives ordering: if the
/// predecessor change fired in an earlier batch than the current change,
/// the predecessor is "pre" (causal).  Anti-causal flow cannot arise in the
/// auto-emerge path (predecessors always come from earlier or the same batch),
/// but the enum is complete so that callers invoking `apply_hebbian_updates`
/// directly can supply the full ordering.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TimingOrder {
    /// pre fired in an earlier batch than post (causal)
    PreFirst,
    /// post fired before pre (anti-causal)
    PostFirst,
    /// same batch (simultaneous)
    Simultaneous,
}

/// One Hebbian/STDP plasticity observation collected during a batch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlasticityObs {
    pub(crate) rel_id: graph_core::RelationshipId,
    pub(crate) kind: graph_core::InfluenceKindId,
    pub(crate) pre: f32,
    pub(crate) post: f32,
    #[allow(dead_code)]
    pub(crate) timing: TimingOrder,
    #[allow(dead_code)]
    /// The postsynaptic locus — needed by the BCM rule to read/update θ_M.
    pub(crate) post_locus: graph_core::LocusId,
}

/// Pre-resolved emergence decision for one cross-locus predecessor.
/// Computed during the parallel read phase; executed sequentially in the
/// apply phase.  Moving the HashMap lookup and decay arithmetic here means
/// the apply phase only performs writes — no redundant reads.
pub(crate) enum EmergenceResolution {
    /// The relationship exists.  `rel_id` is pre-resolved so the apply phase
    /// can call `get_mut(rel_id)` directly, skipping the 2-level
    /// endpoint-key lookup that `lookup(&key, kind)` would perform.
    /// Decay and activity arithmetic are still done in-place in the apply
    /// phase — no allocations are incurred here.
    Update { rel_id: RelationshipId },
    /// No relationship exists yet.  `initial_state` is pre-computed in the
    /// parallel compute phase to move the Vec allocation and arithmetic out
    /// of the sequential apply phase.
    /// Extra fields are retained so the apply phase can handle the rare
    /// case where another resolution in the same batch already created
    /// the same relationship (concurrent creation within one batch).
    Create {
        endpoints: Endpoints,
        kind: InfluenceKindId,
        initial_state: StateVector,
        /// Pre-synaptic signal — needed for the concurrent-creation fallback
        /// update in the apply phase.
        pre_signal: f32,
        activity_contribution: f32,
    },
    /// Phase 2b: the kind's `EmergenceThreshold` is active and no
    /// relationship currently exists for this `(endpoints, kind)`. The
    /// apply phase records the contribution into `PreRelationshipBuffer`,
    /// checks for window expiry and threshold crossing, and only mints
    /// a `Relationship` when the accumulated evidence promotes.
    ///
    /// Carries the minimum apply-side needs: `endpoints` + `kind` for
    /// buffer key, `contribution` (signed pre-computed magnitude added to
    /// the running sum), and `threshold` (Copy) for window/promotion
    /// arithmetic. The `resolved_slots` slice is plumbed separately
    /// through the apply call chain (it would force a Vec allocation per
    /// evidence item if duplicated here), and is what
    /// `apply_emergence_pending` reads at promotion time.
    Pending {
        endpoints: Endpoints,
        kind: InfluenceKindId,
        contribution: f32,
        threshold: crate::registry::EmergenceThreshold,
    },
}

/// Interpreted evidence: the result of the interpretation step that
/// follows detection. Carries the resolved `EmergenceResolution`, schema
/// check, and pre/post timing data required by the apply phase.
///
/// **Phase 0 todo**: rename to `InterpretedEvidence` to mirror
/// `EmergenceEvidence` on the detection side. Deferred from the initial
/// Phase 0 PR to keep the diff scoped to the new seam; rename is a
/// straightforward find-and-replace once the trigger-axis roadmap moves
/// past Phase 1.
pub(crate) struct CrossLocusPred {
    pub(crate) from_locus: LocusId,
    /// Activation value of `from_locus` at the time the predecessor fired.
    /// Used as the pre-synaptic signal for Hebbian plasticity.
    pub(crate) pre_signal: f32,
    /// The batch in which the predecessor change was committed.
    /// Used to derive STDP timing order relative to the current batch.
    pub(crate) pred_batch: BatchId,
    /// The ChangeId of the predecessor change.
    /// Used by engine-native STDP to walk the causal DAG and detect
    /// feedback loops (PostFirst classification).
    #[allow(dead_code)]
    pub(crate) pred_change_id: ChangeId,
    /// True when the causal DAG reveals this is a feedback edge:
    /// the post-locus appears within `STDP_MAX_FEEDBACK_HOPS` in the
    /// predecessor chain of the pre-change.
    /// Only set when STDP is active for the kind; always false otherwise.
    pub(crate) is_feedback: bool,
    /// If the (from_kind, to_kind) pair violates `applies_between`, carries
    /// the endpoint kinds so the apply phase can emit a `SchemaViolation`
    /// event.  `None` means no violation.
    pub(crate) schema_violation: Option<(LocusKindId, LocusKindId)>,
    /// Pre-resolved emergence decision from the compute phase.
    /// The apply phase executes the mutation described here without
    /// performing any additional relationship-store reads.
    pub(crate) emergence: EmergenceResolution,
}

/// Read-only result for a locus-subject pending change.
pub(crate) struct ComputedLocusChange {
    pub(crate) locus_id: LocusId,
    pub(crate) kind: InfluenceKindId,
    pub(crate) predecessors: Vec<ChangeId>,
    pub(crate) before: StateVector,
    pub(crate) after: StateVector,
    pub(crate) wall_time: Option<u64>,
    pub(crate) metadata: Option<Properties>,
    pub(crate) property_patch: Option<Properties>,
    pub(crate) cross_locus_preds: Vec<CrossLocusPred>,
    /// Resolved extra slot definitions (including inherited) — computed once
    /// in the parallel compute phase and carried to the apply phase so that
    /// lazy per-slot decay can be applied without a second registry call.
    pub(crate) resolved_slots: Vec<RelationshipSlotDef>,
    pub(crate) plasticity_active: bool,
    /// First slot of `after` — post-synaptic signal for Hebbian plasticity.
    pub(crate) post_signal: f32,
}

/// Read-only result for a relationship-subject pending change.
pub(crate) struct ComputedRelChange {
    pub(crate) rel_id: RelationshipId,
    pub(crate) kind: InfluenceKindId,
    pub(crate) predecessors: Vec<ChangeId>,
    pub(crate) before: StateVector,
    pub(crate) after: StateVector,
    pub(crate) wall_time: Option<u64>,
    pub(crate) metadata: Option<Properties>,
    pub(crate) from: LocusId,
    pub(crate) to: LocusId,
    /// Whether any subscriber is watching this relationship; cached here
    /// so the apply phase can skip the subscription store lookup.
    pub(crate) has_subscribers: bool,
}

/// Result of the compute phase for one `PendingChange`.
pub(crate) enum ComputedChange {
    Locus(ComputedLocusChange),
    Relationship(ComputedRelChange),
    Elided,
}

// ── Build phase types ─────────────────────────────────────────────────────
//
// After the COMPUTE phase, each `ComputedChange` is promoted to a
// `BuiltChange` that carries a pre-assigned `ChangeId` and a fully
// constructed `Change` record ready for log insertion.  The BUILD
// step can run in parallel (no world writes); the APPLY step that
// follows uses these pre-built records sequentially.

/// Pre-built locus change ready for sequential APPLY.
pub(crate) struct BuiltLocusChange {
    pub(crate) change: Change,
    pub(crate) locus_id: LocusId,
    /// Final locus state from this change — used by the dedup pass.
    pub(crate) after: StateVector,
    pub(crate) property_patch: Option<Properties>,
    pub(crate) cross_locus_preds: Vec<CrossLocusPred>,
    pub(crate) kind: InfluenceKindId,
    /// Resolved extra slot definitions — computed once in the parallel compute
    /// phase and forwarded here so the apply phase avoids a second registry call.
    pub(crate) resolved_slots: Vec<RelationshipSlotDef>,
    pub(crate) plasticity_active: bool,
    pub(crate) post_signal: f32,
}

/// Pre-built relationship change ready for sequential APPLY.
pub(crate) struct BuiltRelChange {
    pub(crate) change: Change,
    pub(crate) rel_id: RelationshipId,
    pub(crate) after: StateVector,
    pub(crate) has_subscribers: bool,
    pub(crate) from: LocusId,
    pub(crate) to: LocusId,
    pub(crate) kind: InfluenceKindId,
}

pub(crate) enum BuiltChange {
    Locus(BuiltLocusChange),
    Relationship(BuiltRelChange),
}

// ── per-partition apply accumulator ──────────────────────────────────────────

use graph_core::{EndpointKey, WorldEvent};
use rustc_hash::{FxHashMap, FxHashSet};

/// Collects all mutable state accumulated during a single partition's Apply
/// pass. In single-partition mode this is constructed once and used directly.
/// In parallel mode, one instance is created per partition; they are merged
/// sequentially (in ascending bucket ID order) after the rayon join.
pub(crate) struct PartitionAccumulator {
    pub batch_changes: Vec<Change>,
    /// (rel_id, trigger_change_id, kind, initial_state) — for Pass B3 log entries.
    /// Records produced by the apply phase for relationships that came
    /// into being during this batch — both bypass-Create (one trigger
    /// change, length-1 predecessors) and Phase 2b promotion-from-buffer
    /// (N contributing changes, length-N predecessors).
    /// Tuple: `(rel_id, predecessors, kind, initial_state)`.
    pub new_emerged_rels: Vec<(
        RelationshipId,
        smallvec::SmallVec<[ChangeId; 4]>,
        InfluenceKindId,
        StateVector,
    )>,
    pub committed_ids_by_locus: FxHashMap<LocusId, Vec<ChangeId>>,
    pub affected_loci: Vec<LocusId>,
    pub affected_loci_set: FxHashSet<LocusId>,
    pub plasticity_obs: Vec<PlasticityObs>,
    pub structural_proposals: Vec<StructuralProposal>,
    /// (endpoint_key → (kinds_touched, rel_ids)) for interaction effects.
    pub batch_kind_touches:
        FxHashMap<EndpointKey, (FxHashSet<InfluenceKindId>, FxHashSet<RelationshipId>)>,
    /// (rel_id, change_id, kind, from, to) — resolved to subscribers after Apply.
    pub pending_rel_notifications:
        Vec<(RelationshipId, ChangeId, InfluenceKindId, LocusId, LocusId)>,
    pub events: Vec<WorldEvent>,
}

impl PartitionAccumulator {
    pub fn new() -> Self {
        Self {
            batch_changes: Vec::new(),
            new_emerged_rels: Vec::new(),
            committed_ids_by_locus: FxHashMap::default(),
            affected_loci: Vec::new(),
            affected_loci_set: FxHashSet::default(),
            plasticity_obs: Vec::new(),
            structural_proposals: Vec::new(),
            batch_kind_touches: FxHashMap::default(),
            pending_rel_notifications: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.batch_changes.clear();
        self.new_emerged_rels.clear();
        self.committed_ids_by_locus.clear();
        self.affected_loci.clear();
        self.affected_loci_set.clear();
        self.plasticity_obs.clear();
        self.structural_proposals.clear();
        self.batch_kind_touches.clear();
        self.pending_rel_notifications.clear();
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::InfluenceKindRegistry;
    use graph_core::{
        ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId, LocusKindId,
        ProposedChange, Relationship, RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn pending(subject: ChangeSubject, after: StateVector) -> PendingChange {
        PendingChange {
            proposed: ProposedChange {
                subject,
                kind: InfluenceKindId(1),
                after,
                extra_predecessors: Vec::new(),
                wall_time: None,
                metadata: None,
                property_patch: None,
                slot_patches: None,
            },
            derived_predecessors: Vec::new(),
        }
    }

    fn reg() -> InfluenceKindRegistry {
        let mut r = InfluenceKindRegistry::new();
        r.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("t"),
        );
        r
    }

    #[test]
    fn locus_change_captures_before_and_after() {
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(0),
            StateVector::from_slice(&[0.5]),
        ));

        let p = pending(
            ChangeSubject::Locus(LocusId(1)),
            StateVector::from_slice(&[1.0]),
        );
        let result = compute_pending_change(p, &world, &reg());

        match result {
            ComputedChange::Locus(c) => {
                assert_eq!(c.locus_id, LocusId(1));
                assert!((c.before.as_slice()[0] - 0.5).abs() < 1e-6);
                assert!((c.after.as_slice()[0] - 1.0).abs() < 1e-6);
            }
            other => panic!(
                "expected Locus variant, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn locus_change_elided_when_locus_missing() {
        let world = World::new(); // empty — no loci
        let p = pending(
            ChangeSubject::Locus(LocusId(99)),
            StateVector::from_slice(&[1.0]),
        );
        let result = compute_pending_change(p, &world, &reg());
        assert!(matches!(result, ComputedChange::Elided));
    }

    #[test]
    fn relationship_change_captures_before_and_after() {
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        world.insert_locus(Locus::new(
            LocusId(2),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        let rel_id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Symmetric {
                a: LocusId(1),
                b: LocusId(2),
            },
            state: StateVector::from_slice(&[2.0, 0.0]),
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

        let p = pending(
            ChangeSubject::Relationship(rel_id),
            StateVector::from_slice(&[3.0, 0.0]),
        );
        let result = compute_pending_change(p, &world, &reg());

        match result {
            ComputedChange::Relationship(c) => {
                assert_eq!(c.rel_id, rel_id);
                assert!((c.before.as_slice()[0] - 2.0).abs() < 1e-6);
                assert!((c.after.as_slice()[0] - 3.0).abs() < 1e-6);
            }
            other => panic!(
                "expected Relationship variant, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }
}
