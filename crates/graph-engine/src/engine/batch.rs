//! Batch loop internals: pending-change bookkeeping, per-locus dispatch
//! staging, and the two relationship-graph mutations that fire inside tick.

use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus,
    LocusId, LocusKindId, Properties, ProposedChange, Relationship, RelationshipId,
    RelationshipLineage, RelationshipSlotDef, StateVector, StructuralProposal,
};
use graph_world::World;

use crate::registry::{InfluenceKindConfig, InfluenceKindRegistry};

/// Maximum predecessor-DAG depth to search for feedback loops in STDP.
/// Covers A→B→C→A (2-hop) plus one extra for robustness. Bounded to
/// prevent O(log-size) traversal on deep DAGs.
const STDP_MAX_FEEDBACK_HOPS: u32 = 3;

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
    let Some(start) = log.get(start_id) else { return false };
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
    pub(crate) rel_id:      graph_core::RelationshipId,
    pub(crate) kind:        graph_core::InfluenceKindId,
    pub(crate) pre:         f32,
    pub(crate) post:        f32,
    pub(crate) timing:      TimingOrder,
    /// The postsynaptic locus — needed by the BCM rule to read/update θ_M.
    pub(crate) post_locus:  graph_core::LocusId,
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
        max_activity: Option<f32>,
    },
    /// Blocked by `min_emerge_activity` — no relationship change needed.
    Blocked,
}

/// A cross-locus predecessor extracted in the compute phase.
/// Carried into the apply phase so relationship mutations can run
/// there with full write access to the relationship store.
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

/// COMPUTE-PHASE helper — pure reads, safe to call in parallel.
///
/// Resolves whether a relationship of `kind` between `from` and `to`
/// already exists, computes the decayed + bumped state (for an existing
/// relationship) or the initial state (for a new one), and returns the
/// decision for the apply phase to execute without any further reads.
///
/// Mirrors the read-only half of `auto_emerge_relationship`; the apply
/// phase owns the matching write half.
pub(crate) fn resolve_emergence(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: InfluenceKindId,
    cfg: Option<&InfluenceKindConfig>,
    resolved_slots: &[RelationshipSlotDef],
    pre_signal: f32,
) -> EmergenceResolution {
    let activity_contribution = cfg.map(|c| c.activity_contribution).unwrap_or(1.0);
    let max_activity          = cfg.and_then(|c| c.max_activity);

    let endpoints = if cfg.map(|c| c.symmetric).unwrap_or(false) {
        Endpoints::Symmetric { a: from, b: to }
    } else {
        Endpoints::Directed { from, to }
    };
    let key = endpoints.key();
    let store = world.relationships();

    debug_assert!(cfg.is_some(), "resolve_emergence: InfluenceKindId {kind:?} is not registered — call influence_registry.register() before ticking");

    // Single lookup reused for both the min_emerge gate and the Update/Create branch.
    let existing = store.lookup(&key, kind);

    // min_emerge_activity gate — only blocks *creation* of new relationships.
    let min_emerge = cfg.map(|c| c.min_emerge_activity).unwrap_or(0.0);
    if min_emerge > 0.0 && existing.is_none() && pre_signal.abs() < min_emerge {
        return EmergenceResolution::Blocked;
    }

    if let Some(rel_id) = existing {
        // Decay and activity contribution are applied in-place during the
        // sequential apply phase (no allocation here).
        EmergenceResolution::Update { rel_id }
    } else {
        let initial_activity = {
            let a = activity_contribution * pre_signal.abs();
            match max_activity { Some(cap) => a.clamp(-cap, cap), None => a }
        };
        let mut values = vec![initial_activity, 0.0f32];
        for slot in resolved_slots {
            values.push(slot.default);
        }
        EmergenceResolution::Create {
            endpoints,
            kind,
            initial_state: StateVector::from_slice(&values),
            pre_signal,
            activity_contribution,
            max_activity,
        }
    }
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
    /// The change was elided (no effect, no cross-locus flow, no metadata).
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

/// BUILD PHASE — pure construction, safe to call in parallel.
///
/// Assigns the pre-reserved `ChangeId` at position `idx` within the
/// reserved block starting at `base_id`, then packages all APPLY-phase
/// inputs into a `BuiltChange`.  No world reads or writes occur here.
pub(crate) fn build_computed_change(
    idx: usize,
    base_id: ChangeId,
    computed: ComputedChange,
    batch: BatchId,
) -> BuiltChange {
    let id = ChangeId(base_id.0 + idx as u64);
    match computed {
        ComputedChange::Locus(c) => {
            let change = Change {
                id,
                subject: ChangeSubject::Locus(c.locus_id),
                kind: c.kind,
                predecessors: c.predecessors,
                before: c.before,
                after: c.after.clone(),
                batch,
                wall_time: c.wall_time,
                metadata: c.metadata,
            };
            BuiltChange::Locus(BuiltLocusChange {
                change,
                locus_id: c.locus_id,
                after: c.after,
                property_patch: c.property_patch,
                cross_locus_preds: c.cross_locus_preds,
                kind: c.kind,
                resolved_slots: c.resolved_slots,
                plasticity_active: c.plasticity_active,
                post_signal: c.post_signal,
            })
        }
        ComputedChange::Relationship(c) => {
            let change = Change {
                id,
                subject: ChangeSubject::Relationship(c.rel_id),
                kind: c.kind,
                predecessors: c.predecessors,
                before: c.before,
                after: c.after.clone(),
                batch,
                wall_time: c.wall_time,
                metadata: c.metadata,
            };
            BuiltChange::Relationship(BuiltRelChange {
                change,
                rel_id: c.rel_id,
                after: c.after,
                has_subscribers: c.has_subscribers,
                from: c.from,
                to: c.to,
                kind: c.kind,
            })
        }
        ComputedChange::Elided => unreachable!(
            "build_computed_change must not be called with Elided — filter before dispatch"
        ),
    }
}

/// COMPUTE PHASE — pure read, safe to call in parallel.
///
/// Dispatches to `compute_locus_change` or `compute_relationship_change`
/// based on `pending.proposed.subject`. No IDs are minted and no world
/// state is touched.
pub(crate) fn compute_pending_change(
    pending: PendingChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let PendingChange { proposed, derived_predecessors } = pending;
    let mut predecessors = derived_predecessors;
    predecessors.extend(proposed.extra_predecessors.iter().copied());
    let kind = proposed.kind;

    match proposed.subject {
        ChangeSubject::Locus(locus_id) =>
            compute_locus_change(locus_id, kind, predecessors, proposed, world, influence_registry),
        ChangeSubject::Relationship(rel_id) =>
            compute_relationship_change(rel_id, kind, predecessors, proposed, world, influence_registry),
    }
}

/// Compute phase for a locus-subject pending change.
fn compute_locus_change(
    locus_id: LocusId,
    kind: InfluenceKindId,
    predecessors: Vec<ChangeId>,
    proposed: ProposedChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let Some(locus) = world.locus(locus_id) else {
        return ComputedChange::Elided;
    };
    let before = locus.state.clone();
    let cross_locus_pairs: Vec<(LocusId, f32, BatchId, ChangeId)> = predecessors
        .iter()
        .filter_map(|pid| world.log().get(*pid))
        .filter_map(|pred| match pred.subject {
            ChangeSubject::Locus(pl)
                if pl != locus_id && world.locus(pl).is_some() =>
            {
                let pre = pred.after.as_slice().first().copied().unwrap_or(0.0);
                Some((pl, pre, pred.batch, pred.id))
            }
            _ => None,
        })
        .collect();
    let kind_cfg = influence_registry.get(kind);
    let resolved_slots = influence_registry.resolved_extra_slots(kind);
    let stabilized_after = match kind_cfg {
        Some(cfg) => cfg.stabilization.stabilize(&before, proposed.after),
        None => proposed.after,
    };
    // Elide no-op follow-ups: a change is silently dropped when ALL
    // four conditions hold:
    //   1. It has at least one predecessor (i.e. it is a derived
    //      change, not a user-supplied stimulus). Stimuli (empty
    //      predecessors) are never elided.
    //   2. It has no cross-locus predecessors that would trigger
    //      auto-emergence or Hebbian plasticity.
    //   3. The stabilized state equals the pre-batch state (no
    //      net numeric change after stabilization).
    //   4. It carries no metadata or property patch that must be
    //      committed regardless of state change.
    //
    // Callers that want "zero-magnitude but still recorded" changes
    // should attach metadata or set `extra_predecessors` to empty.
    if !predecessors.is_empty()
        && cross_locus_pairs.is_empty()
        && stabilized_after == before
        && proposed.metadata.is_none()
        && proposed.property_patch.is_none()
    {
        return ComputedChange::Elided;
    }
    let post_signal = stabilized_after.as_slice().first().copied().unwrap_or(0.0);
    let plasticity_active = kind_cfg
        .map(|cfg| cfg.plasticity.is_active())
        .unwrap_or(false);
    // Build cross-locus pred descriptors with schema violation checks and
    // pre-resolved emergence decisions.  Both are pure reads, so they are
    // safe here in the parallel compute phase.
    let cross_locus_preds: Vec<CrossLocusPred> = cross_locus_pairs
        .into_iter()
        .map(|(from_locus, pre_signal, pred_batch, pred_change_id)| {
            let schema_violation = if let Some(cfg) = kind_cfg
                && !cfg.applies_between.is_empty()
            {
                let from_kind = world.locus(from_locus).map(|l| l.kind);
                let to_kind = world.locus(locus_id).map(|l| l.kind);
                match (from_kind, to_kind) {
                    (Some(fk), Some(tk)) if !cfg.allows_endpoint_kinds(fk, tk) => {
                        Some((fk, tk))
                    }
                    _ => None,
                }
            } else {
                None
            };
            let emergence = resolve_emergence(
                world, from_locus, locus_id, kind,
                kind_cfg, &resolved_slots, pre_signal,
            );
            let is_feedback = kind_cfg
                .map(|k| k.plasticity.stdp && k.plasticity.is_active())
                .unwrap_or(false)
                && is_feedback_in_dag(world.log(), pred_change_id, locus_id, STDP_MAX_FEEDBACK_HOPS);
            CrossLocusPred { from_locus, pre_signal, pred_batch, pred_change_id, is_feedback, schema_violation, emergence }
        })
        .collect();
    ComputedChange::Locus(ComputedLocusChange {
        locus_id,
        kind,
        predecessors,
        before,
        after: stabilized_after,
        wall_time: proposed.wall_time,
        metadata: proposed.metadata,
        property_patch: proposed.property_patch,
        cross_locus_preds,
        resolved_slots,
        plasticity_active,
        post_signal,
    })
}

/// Compute phase for a relationship-subject pending change.
fn compute_relationship_change(
    rel_id: RelationshipId,
    kind: InfluenceKindId,
    predecessors: Vec<ChangeId>,
    proposed: ProposedChange,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
) -> ComputedChange {
    let (before, from, to) = match world.relationships().get(rel_id) {
        Some(r) => {
            let (f, t) = match r.endpoints {
                Endpoints::Directed { from, to } => (from, to),
                Endpoints::Symmetric { a, b } => (a, b),
            };
            (r.state.clone(), f, t)
        }
        None => return ComputedChange::Elided,
    };
    let raw_after = match proposed.slot_patches {
        Some(patches) => patches
            .into_iter()
            .fold(before.clone(), |s, (idx, delta)| s.with_slot_delta(idx, delta)),
        None => proposed.after,
    };
    let stabilized_after = match influence_registry.get(kind) {
        Some(cfg) => cfg.stabilization.stabilize(&before, raw_after),
        None => raw_after,
    };
    let has_subscribers =
        world.subscriptions().has_any_subscribers(rel_id, kind, from, to);
    ComputedChange::Relationship(ComputedRelChange {
        rel_id,
        kind,
        predecessors,
        before,
        after: stabilized_after,
        wall_time: proposed.wall_time,
        metadata: proposed.metadata,
        from,
        to,
        has_subscribers,
    })
}


/// Apply structural proposals collected during a batch's program-dispatch phase.
///
/// `CreateRelationship`: if the (endpoints, kind) pair already exists,
/// treat it as an activity touch. Otherwise mint and insert a new
/// relationship with `created_by: None` (no originating change). Extra
/// slots are initialised from the kind's `InfluenceKindConfig`.
///
/// `DeleteRelationship`: notify Specific subscribers (tombstone) **before**
/// removal, then remove from the store. The relationship's past changes in
/// the log remain intact.
///
/// `SubscribeToRelationship` / `UnsubscribeFromRelationship`: update the
/// world's subscription store so the subscriber locus receives inbox
/// entries when the relationship's state changes.
///
/// ## Return value
///
/// Returns a `Vec<PendingChange>` of **tombstone** notifications — one per
/// subscriber for each relationship deleted in this call. The caller must
/// extend the next batch's `pending` queue with this vec so subscribers
/// receive an inbox entry signalling the deletion.
///
/// A tombstone is a zero-delta `ChangeSubject::Locus` change with metadata:
/// ```text
/// { "tombstone": true, "rel_id": <id> }
/// ```
/// The subscriber's program can pattern-match on `change.metadata` to detect
/// it. Because `predecessors` is empty (root stimulus), the elision guard
/// does not apply, so the change is always committed even when locus state is
/// unchanged.
pub(crate) fn apply_structural_proposals(
    world: &mut World,
    proposals: Vec<StructuralProposal>,
    influence_registry: &crate::registry::InfluenceKindRegistry,
) -> Vec<PendingChange> {
    let current_batch = world.current_batch().0;
    let batch_id = BatchId(current_batch);
    let mut tombstones: Vec<PendingChange> = Vec::new();

    for proposal in proposals {
        match proposal {
            StructuralProposal::CreateRelationship { endpoints, kind, initial_activity, initial_state } => {
                let key = endpoints.key();
                let store = world.relationships_mut();
                if let Some(rel_id) = store.lookup(&key, kind) {
                    // Already exists: treat as activity touch regardless of initial_* fields.
                    let contribution = influence_registry
                        .get(kind)
                        .map(|c| c.activity_contribution)
                        .unwrap_or(1.0);
                    let rel = store.get_mut(rel_id).expect("indexed id must exist");
                    if let Some(a) = rel.state.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT) {
                        *a += contribution;
                    }
                    rel.lineage.change_count += 1;
                } else {
                    // New relationship: resolve initial state in priority order.
                    // 1. initial_state (full vector) takes precedence.
                    // 2. initial_activity overrides only slot 0.
                    // 3. Registry-resolved default (includes inherited slots).
                    let state = if let Some(s) = initial_state {
                        s
                    } else {
                        let mut s = influence_registry.initial_state_for(kind);
                        if let Some(act) = initial_activity {
                            if let Some(a) = s.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT) {
                                *a = act;
                            }
                        }
                        s
                    };
                    let new_id = store.mint_id();
                    store.insert(Relationship {
                        id: new_id,
                        kind,
                        endpoints,
                        state,
                        lineage: RelationshipLineage {
                            created_by: None,
                            last_touched_by: None,
                            change_count: 1,
                            kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
                        },
                        created_batch: BatchId(current_batch),
                        last_decayed_batch: current_batch,
                        metadata: None,
                    });
                }
            }
            StructuralProposal::DeleteRelationship { rel_id } => {
                // Capture (kind, subscribers) before removal so tombstones
                // carry the correct kind and subscriber list.
                let rel_kind = world.relationships().get(rel_id).map(|r| r.kind);
                let specific_subs = world.subscriptions_mut().remove_relationship(rel_id);
                world.relationships_mut().remove(rel_id);
                if let Some(kind) = rel_kind {
                    tombstones.extend(
                        make_tombstones(world, rel_id, kind, specific_subs),
                    );
                }
            }
            StructuralProposal::SubscribeToRelationship { subscriber, rel_id } => {
                world.subscriptions_mut().subscribe_at(subscriber, rel_id, Some(batch_id));
            }
            StructuralProposal::UnsubscribeFromRelationship { subscriber, rel_id } => {
                world.subscriptions_mut().unsubscribe_at(subscriber, rel_id, Some(batch_id));
            }
            StructuralProposal::SubscribeToKind { subscriber, kind } => {
                world.subscriptions_mut().subscribe_to_kind(subscriber, kind);
            }
            StructuralProposal::UnsubscribeFromKind { subscriber, kind } => {
                world.subscriptions_mut().unsubscribe_from_kind(subscriber, kind);
            }
            StructuralProposal::SubscribeToAnchorKind { subscriber, anchor, kind } => {
                world.subscriptions_mut().subscribe_to_anchor_kind(subscriber, anchor, kind);
            }
            StructuralProposal::UnsubscribeFromAnchorKind { subscriber, anchor, kind } => {
                world.subscriptions_mut().unsubscribe_from_anchor_kind(subscriber, anchor, kind);
            }
            StructuralProposal::DeleteLocus { locus_id } => {
                // Collect all relationship ids touching this locus first to avoid
                // holding an immutable borrow on the store during removal.
                let rel_ids: Vec<graph_core::RelationshipId> = world
                    .relationships()
                    .relationships_for_locus(locus_id)
                    .map(|r| r.id)
                    .collect();
                for rel_id in rel_ids {
                    let rel_kind = world.relationships().get(rel_id).map(|r| r.kind);
                    let specific_subs = world.subscriptions_mut().remove_relationship(rel_id);
                    world.relationships_mut().remove(rel_id);
                    if let Some(kind) = rel_kind {
                        // Only notify external subscribers — not the locus being deleted.
                        let external: Vec<_> = specific_subs
                            .into_iter()
                            .filter(|&s| s != locus_id)
                            .collect();
                        tombstones.extend(make_tombstones(world, rel_id, kind, external));
                    }
                }
                world.subscriptions_mut().remove_locus(locus_id);
                // Remove anchor-kind subscriptions for which this locus was the anchor.
                world.subscriptions_mut().remove_anchor_locus(locus_id);
                world.properties_mut().remove(locus_id);
                world.names_mut().remove(locus_id);
                world.bcm_thresholds_mut().remove(&locus_id);
                world.loci_mut().remove(locus_id);
            }
            StructuralProposal::CreateLocus { locus_id, kind, state, name, properties } => {
                // Resolve the target ID: explicit or auto-assigned.
                let id = locus_id.unwrap_or_else(|| world.loci().next_id());
                world.insert_locus(Locus::new(id, kind, state));
                if let Some(n) = name {
                    world.names_mut().insert(n, id);
                }
                if let Some(props) = properties {
                    world.properties_mut().insert(id, props);
                }
            }
        }
    }

    tombstones
}

/// Build tombstone `PendingChange`s for a list of Specific-scope subscribers
/// when `rel_id` (of the given `kind`) is about to be deleted.
///
/// The tombstone is a zero-delta locus change whose metadata carries:
/// - `"tombstone"` = `true`
/// - `"rel_id"` = numeric id of the deleted relationship
///
/// Using the subscriber's current locus state as `after` means the engine's
/// stabilization step leaves the state unchanged; the only observable effect
/// is the inbox entry with tombstone metadata.
fn make_tombstones(
    world: &World,
    rel_id: graph_core::RelationshipId,
    kind: graph_core::InfluenceKindId,
    subscribers: Vec<graph_core::LocusId>,
) -> Vec<PendingChange> {
    subscribers
        .into_iter()
        .filter_map(|sub| {
            let after = world.locus(sub)?.state.clone();
            let mut meta = Properties::new();
            meta.set("tombstone", true);
            meta.set("rel_id", rel_id.0 as f64);
            Some(PendingChange {
                proposed: ProposedChange {
                    subject: ChangeSubject::Locus(sub),
                    kind,
                    after,
                    extra_predecessors: Vec::new(),
                    wall_time: None,
                    metadata: Some(meta),
                    property_patch: None,
                    slot_patches: None,
                },
                derived_predecessors: Vec::new(),
            })
        })
        .collect()
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
    pub new_emerged_rels: Vec<(RelationshipId, ChangeId, InfluenceKindId, StateVector)>,
    pub committed_ids_by_locus: FxHashMap<LocusId, Vec<ChangeId>>,
    pub affected_loci: Vec<LocusId>,
    pub affected_loci_set: FxHashSet<LocusId>,
    pub plasticity_obs: Vec<PlasticityObs>,
    pub structural_proposals: Vec<StructuralProposal>,
    /// (endpoint_key → (kinds_touched, rel_ids)) for interaction effects.
    pub batch_kind_touches: FxHashMap<EndpointKey, (FxHashSet<InfluenceKindId>, FxHashSet<RelationshipId>)>,
    /// (rel_id, change_id, kind, from, to) — resolved to subscribers after Apply.
    pub pending_rel_notifications: Vec<(RelationshipId, ChangeId, InfluenceKindId, LocusId, LocusId)>,
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
    use graph_core::{
        ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId,
        LocusKindId, ProposedChange, Relationship, RelationshipId, RelationshipLineage,
        StateVector,
    };
    use graph_world::World;
    use crate::registry::InfluenceKindRegistry;

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
        r.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));
        r
    }

    #[test]
    fn locus_change_captures_before_and_after() {
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1), LocusKindId(0), StateVector::from_slice(&[0.5]),
        ));

        let p = pending(ChangeSubject::Locus(LocusId(1)), StateVector::from_slice(&[1.0]));
        let result = compute_pending_change(p, &world, &reg());

        match result {
            ComputedChange::Locus(c) => {
                assert_eq!(c.locus_id, LocusId(1));
                assert!((c.before.as_slice()[0] - 0.5).abs() < 1e-6);
                assert!((c.after.as_slice()[0] - 1.0).abs() < 1e-6);
            }
            other => panic!("expected Locus variant, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn locus_change_elided_when_locus_missing() {
        let world = World::new(); // empty — no loci
        let p = pending(ChangeSubject::Locus(LocusId(99)), StateVector::from_slice(&[1.0]));
        let result = compute_pending_change(p, &world, &reg());
        assert!(matches!(result, ComputedChange::Elided));
    }

    #[test]
    fn relationship_change_captures_before_and_after() {
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(0), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(0), StateVector::zeros(1)));
        let rel_id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Symmetric { a: LocusId(1), b: LocusId(2) },
            state: StateVector::from_slice(&[2.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![
                    KindObservation::synthetic(InfluenceKindId(1))
                ],
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
            other => panic!("expected Relationship variant, got {:?}", std::mem::discriminant(&other)),
        }
    }
}
