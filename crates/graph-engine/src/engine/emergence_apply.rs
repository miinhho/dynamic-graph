use graph_core::{
    BatchId, ChangeId, InfluenceKindId, KindObservation, Relationship, RelationshipId,
    RelationshipLineage, StateVector,
};
use graph_world::{RecordOutcome, RelationshipStore, World};
use smallvec::SmallVec;

use super::batch::{signed_activity_contribution, EmergenceResolution};

/// Result of one `apply_emergence` call. The wider shape (vs the old
/// 3-tuple) accommodates Phase 2b's promotion path, which carries both
/// the freshly-minted state *and* the multi-change predecessor list.
pub(super) struct AppliedEmergenceOutcome {
    pub rel_id: RelationshipId,
    /// `true` when this call materialised a new relationship — either via
    /// bypass-Create on the first cross-locus predecessor, or via Phase 2b
    /// promotion when accumulated evidence crossed `min_evidence`.
    pub is_new: bool,
    /// Initial state for the newly-minted relationship; `None` when the
    /// call updated an existing one.
    pub initial_state: Option<StateVector>,
    /// `Some(contributing_changes)` when Phase 2b promotion fired —
    /// every change that fed evidence into the eventual promotion. The
    /// caller plumbs this through to `Change.predecessors` so promoted
    /// relationships preserve the full causal lineage of their
    /// pre-promotion buffer state. `None` for bypass-Create (caller
    /// uses `smallvec![trigger_id]`) and for Update (no Change record
    /// is appended at the emergence-apply step).
    pub promotion_predecessors: Option<SmallVec<[ChangeId; 4]>>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_emergence(
    world: &mut World,
    resolution: EmergenceResolution,
    change_id: ChangeId,
    batch: BatchId,
    kind: InfluenceKindId,
    pre_signal: f32,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) -> Option<AppliedEmergenceOutcome> {
    let update_context = EmergenceUpdateContext {
        change_id,
        batch,
        kind,
        pre_signal,
        kind_cfg,
        resolved_slots,
    };

    match resolution {
        EmergenceResolution::Update { rel_id } => {
            apply_emergence_update(world.relationships_mut(), rel_id, update_context)
        }
        EmergenceResolution::Create {
            endpoints,
            kind: rel_kind,
            initial_state,
            pre_signal: create_pre_signal,
            activity_contribution,
        } => apply_emergence_create(
            world.relationships_mut(),
            EmergenceCreateContext {
                change_id,
                batch,
                endpoints,
                rel_kind,
                initial_state,
                create_pre_signal,
                activity_contribution,
            },
        ),
        EmergenceResolution::Pending {
            endpoints,
            kind: rel_kind,
            contribution,
            threshold,
        } => apply_emergence_pending(
            world,
            EmergencePendingContext {
                change_id,
                batch,
                endpoints,
                rel_kind,
                contribution,
                threshold,
                resolved_slots,
            },
        ),
    }
}

struct EmergencePendingContext<'a> {
    change_id: ChangeId,
    batch: BatchId,
    endpoints: graph_core::Endpoints,
    rel_kind: InfluenceKindId,
    contribution: f32,
    threshold: crate::registry::EmergenceThreshold,
    resolved_slots: &'a [graph_core::RelationshipSlotDef],
}

/// Phase 2b: route a `Pending` resolution through the buffer.
///
/// Records the contribution, then either keeps it pending (returns `None`,
/// which the caller treats like "no apply-side action — no event, no
/// change record") or promotes the entry into a fresh `Relationship` and
/// returns the full promotion outcome including the contributing-change
/// list for the eventual `Change.predecessors`.
fn apply_emergence_pending(
    world: &mut World,
    context: EmergencePendingContext<'_>,
) -> Option<AppliedEmergenceOutcome> {
    let key = context.endpoints.key();

    // Single-transition-point invariant (Phase 2 design review #4): once
    // a `(key, kind)` is in `RelationshipStore`, it must never also be in
    // `PreRelationshipBuffer`. Within one batch, evidence items are
    // computed in parallel — they all see the pre-batch world and may
    // resolve to `Pending` even when an earlier sibling in the same
    // sequential apply pass has just promoted the same `(key, kind)`. The
    // compute phase cannot know about that sibling promotion, so we
    // re-check here and route to the existing-relationship path. Without
    // this guard, the third-and-later sibling in a high-fan-in convergent
    // flow leaks a stale buffer entry.
    if let Some(existing_id) = world.relationships().lookup(&key, context.rel_kind) {
        // The pre_signal we should pass for the activity-contribution
        // recovery is encoded as the contribution itself (signed
        // a × pre_signal product). The `apply_existing_emergent_relationship`
        // path expects (pre_signal, activity_contribution) separately so it
        // can re-multiply; passing (contribution, 1.0) yields the same
        // delta (`activity_contribution_magnitude(1.0, contribution)`),
        // preserving bit-equivalence with the bypass concurrent-create path.
        crate::engine::emergence_apply::apply_existing_emergent_relationship(
            world.relationships_mut(),
            existing_id,
            context.change_id,
            context.batch,
            context.rel_kind,
            context.contribution,
            1.0,
        );
        return Some(AppliedEmergenceOutcome {
            rel_id: existing_id,
            is_new: false,
            initial_state: None,
            promotion_predecessors: None,
        });
    }

    let outcome = world.pre_relationships_mut().record_evidence(
        key,
        context.rel_kind,
        context.contribution,
        context.change_id,
        context.batch,
        context.threshold.window_batches,
        context.threshold.min_evidence,
    );
    match outcome {
        RecordOutcome::StillPending => None,
        RecordOutcome::Promoted {
            accumulated,
            contributing_changes,
        } => {
            let initial_state = build_promotion_state(accumulated, context.resolved_slots);
            let new_id = insert_emergent_relationship(
                world.relationships_mut(),
                context.change_id,
                context.batch,
                context.endpoints,
                context.rel_kind,
                initial_state.clone(),
            );
            Some(AppliedEmergenceOutcome {
                rel_id: new_id,
                is_new: true,
                initial_state: Some(initial_state),
                promotion_predecessors: Some(contributing_changes),
            })
        }
    }
}

/// Build the `StateVector` for a freshly-promoted relationship. Activity
/// slot is seeded with the accumulated signed evidence (matching the
/// semantics of bypass-Create where activity = activity_contribution × signal,
/// summed over all contributors); weight starts at 0; user slot defaults
/// follow.
fn build_promotion_state(
    accumulated: f32,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) -> StateVector {
    let mut values = vec![accumulated, 0.0f32];
    values.extend(resolved_slots.iter().map(|slot| slot.default));
    StateVector::from_slice(&values)
}

#[derive(Clone, Copy)]
struct EmergenceUpdateContext<'a> {
    change_id: ChangeId,
    batch: BatchId,
    kind: InfluenceKindId,
    pre_signal: f32,
    kind_cfg: Option<&'a crate::registry::InfluenceKindConfig>,
    resolved_slots: &'a [graph_core::RelationshipSlotDef],
}

struct EmergenceCreateContext {
    change_id: ChangeId,
    batch: BatchId,
    endpoints: graph_core::Endpoints,
    rel_kind: InfluenceKindId,
    initial_state: StateVector,
    create_pre_signal: f32,
    activity_contribution: f32,
}

fn apply_emergence_update(
    store: &mut RelationshipStore,
    rel_id: RelationshipId,
    context: EmergenceUpdateContext<'_>,
) -> Option<AppliedEmergenceOutcome> {
    let decay = emergence_decay(context.kind_cfg, context.pre_signal);
    if let Some(rel) = store.get_mut(rel_id) {
        apply_emergence_update_slots(rel, context.batch, decay, context.resolved_slots);
        touch_emergent_relationship(rel, context.change_id, context.kind, context.batch);
    }
    Some(AppliedEmergenceOutcome {
        rel_id,
        is_new: false,
        initial_state: None,
        promotion_predecessors: None,
    })
}

fn apply_emergence_create(
    store: &mut RelationshipStore,
    context: EmergenceCreateContext,
) -> Option<AppliedEmergenceOutcome> {
    let key = context.endpoints.key();
    if let Some(existing_id) = store.lookup(&key, context.rel_kind) {
        apply_existing_emergent_relationship(
            store,
            existing_id,
            context.change_id,
            context.batch,
            context.rel_kind,
            context.create_pre_signal,
            context.activity_contribution,
        );
        Some(AppliedEmergenceOutcome {
            rel_id: existing_id,
            is_new: false,
            initial_state: None,
            promotion_predecessors: None,
        })
    } else {
        let new_id = insert_emergent_relationship(
            store,
            context.change_id,
            context.batch,
            context.endpoints,
            context.rel_kind,
            context.initial_state.clone(),
        );
        Some(AppliedEmergenceOutcome {
            rel_id: new_id,
            is_new: true,
            initial_state: Some(context.initial_state),
            promotion_predecessors: None,
        })
    }
}

#[derive(Clone, Copy)]
struct EmergenceDecay {
    activity_decay: f32,
    weight_decay: f32,
    /// Pre-computed activity contribution (`activity_contribution × |pre_signal|`).
    /// Centralised through `signed_activity_contribution` so Phase 1
    /// of the trigger-axis roadmap has one place to revisit signed semantics.
    activity_input: f32,
}

fn emergence_decay(
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    pre_signal: f32,
) -> EmergenceDecay {
    let activity_contribution = kind_cfg.map_or(1.0, |cfg| cfg.activity_contribution);
    EmergenceDecay {
        activity_decay: kind_cfg.map_or(1.0, |cfg| cfg.decay_per_batch),
        weight_decay: kind_cfg.map_or(1.0, |cfg| cfg.plasticity.weight_decay),
        activity_input: signed_activity_contribution(activity_contribution, pre_signal),
    }
}

fn apply_emergence_update_slots(
    rel: &mut Relationship,
    batch: BatchId,
    decay: EmergenceDecay,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) {
    let delta = batch.0.saturating_sub(rel.last_decayed_batch);
    if delta > 0 {
        apply_decay_step(rel, delta, decay, resolved_slots);
        rel.last_decayed_batch = batch.0;
    } else if let Some(activity) = rel
        .state
        .as_mut_slice()
        .get_mut(Relationship::ACTIVITY_SLOT)
    {
        *activity += decay.activity_input;
    }
}

fn apply_decay_step(
    rel: &mut Relationship,
    delta: u64,
    decay: EmergenceDecay,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) {
    let activity_factor = pow_decay(decay.activity_decay, delta);
    let weight_factor = pow_decay(decay.weight_decay, delta);
    let slots = rel.state.as_mut_slice();
    if let Some(activity) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
        *activity = *activity * activity_factor + decay.activity_input;
    }
    if let Some(weight) = slots.get_mut(Relationship::WEIGHT_SLOT) {
        *weight *= weight_factor;
    }
    apply_resolved_slot_decay(slots, delta, resolved_slots);
}

fn apply_resolved_slot_decay(
    slots: &mut [f32],
    delta: u64,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) {
    for (index, slot_def) in resolved_slots.iter().enumerate() {
        if let Some(factor) = slot_def.decay
            && let Some(value) = slots.get_mut(2 + index)
        {
            *value *= pow_decay(factor, delta);
        }
    }
}

fn pow_decay(factor: f32, delta: u64) -> f32 {
    if delta == 1 {
        factor
    } else {
        factor.powi(delta as i32)
    }
}

fn touch_emergent_relationship(
    rel: &mut Relationship,
    change_id: ChangeId,
    kind: InfluenceKindId,
    batch: BatchId,
) {
    rel.lineage.last_touched_by = Some(change_id);
    rel.lineage.change_count += 1;
    rel.lineage.observe_kind(kind, batch);
}

fn apply_existing_emergent_relationship(
    store: &mut RelationshipStore,
    rel_id: RelationshipId,
    change_id: ChangeId,
    batch: BatchId,
    rel_kind: InfluenceKindId,
    create_pre_signal: f32,
    activity_contribution: f32,
) {
    let rel = store.get_mut(rel_id).expect("indexed id must exist");
    if let Some(activity) = rel
        .state
        .as_mut_slice()
        .get_mut(Relationship::ACTIVITY_SLOT)
    {
        *activity += signed_activity_contribution(activity_contribution, create_pre_signal);
    }
    touch_emergent_relationship(rel, change_id, rel_kind, batch);
}

fn insert_emergent_relationship(
    store: &mut RelationshipStore,
    change_id: ChangeId,
    batch: BatchId,
    endpoints: graph_core::Endpoints,
    rel_kind: InfluenceKindId,
    initial_state: StateVector,
) -> RelationshipId {
    let new_id = store.mint_id();
    store.insert(Relationship {
        id: new_id,
        kind: rel_kind,
        endpoints,
        state: initial_state,
        lineage: RelationshipLineage {
            created_by: Some(change_id),
            last_touched_by: Some(change_id),
            change_count: 1,
            kinds_observed: smallvec::smallvec![KindObservation::once(rel_kind, batch)],
        },
        created_batch: batch,
        last_decayed_batch: batch.0,
        metadata: None,
    });
    new_id
}
