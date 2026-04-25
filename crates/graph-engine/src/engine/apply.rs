use super::*;

pub(super) fn apply_built_changes(
    engine: &Engine,
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    batch: BatchId,
    built_batch: BuiltBatch,
    state: &mut TickState,
    telemetry: &mut TickTelemetry,
) -> AppliedBatch {
    engine.preapply_locus_state(world, &built_batch.built, telemetry);
    state.acc.batch_changes.reserve(built_batch.built.len());
    let t_ae0 = telemetry.start();
    for change in built_batch.built {
        engine.apply_built_change(world, influence_registry, batch, change, state);
        state.result.changes_committed += 1;
    }
    TickTelemetry::record(&mut telemetry.apply_emerge, t_ae0);

    let t_acl0 = telemetry.start();
    world.extend_batch_changes(std::mem::take(&mut state.acc.batch_changes));
    TickTelemetry::record(&mut telemetry.apply_changelog, t_acl0);

    let t_ab30 = telemetry.start();
    engine.append_emergence_changes(world, batch, state);
    TickTelemetry::record(&mut telemetry.apply_b3, t_ab30);
    AppliedBatch { batch }
}

pub(super) fn settle_batch(
    engine: &Engine,
    context: &mut SettleContext<'_>,
    applied: AppliedBatch,
) -> SettledBatch {
    engine.dispatch_affected_loci(
        context.world,
        context.loci_registry,
        context.slot_defs,
        applied.batch,
        context.state,
        context.telemetry,
    );
    engine.apply_structural_and_hebbian(
        context.world,
        context.influence_registry,
        applied.batch,
        context.state,
        context.telemetry,
    );
    SettledBatch {
        batch: applied.batch,
    }
}

pub(super) fn settle_empty_batch(
    engine: &Engine,
    context: &mut SettleContext<'_>,
    batch: BatchId,
) -> SettledBatch {
    engine.dispatch_affected_loci(
        context.world,
        context.loci_registry,
        context.slot_defs,
        batch,
        context.state,
        context.telemetry,
    );
    engine.apply_structural_and_hebbian(
        context.world,
        context.influence_registry,
        batch,
        context.state,
        context.telemetry,
    );
    SettledBatch { batch }
}

pub(super) fn advance_batch(
    _engine: &Engine,
    world: &mut World,
    settled: SettledBatch,
    state: &mut TickState,
    telemetry: &mut TickTelemetry,
) {
    let to2 = telemetry.start();
    state.result.events.append(&mut state.acc.events);
    debug_assert_eq!(settled.batch, world.current_batch());
    world.advance_batch();
    TickTelemetry::record(&mut telemetry.other, to2);
    state.result.batches_committed += 1;
}

pub(super) fn preapply_locus_state(
    _engine: &Engine,
    world: &mut World,
    built: &[BuiltChange],
    telemetry: &mut TickTelemetry,
) {
    let t_al0 = telemetry.start();
    let mut n_potential_new_rels: usize = 0;
    for change in built {
        if let BuiltChange::Locus(c) = change {
            if let Some(locus) = world.locus_mut(c.locus_id) {
                locus.state = c.after.clone();
            }
            n_potential_new_rels += c.cross_locus_preds.len();
        }
    }
    if n_potential_new_rels > 0 {
        world.relationships_mut().reserve(n_potential_new_rels);
    }
    TickTelemetry::record(&mut telemetry.apply_locus, t_al0);
}

pub(super) fn apply_built_change(
    engine: &Engine,
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    batch: BatchId,
    built: BuiltChange,
    state: &mut TickState,
) {
    match built {
        BuiltChange::Locus(c) => {
            engine.apply_locus_change(world, influence_registry, batch, c, state)
        }
        BuiltChange::Relationship(c) => engine.apply_relationship_change(c, world, state),
    }
}

pub(super) fn apply_locus_change(
    engine: &Engine,
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    batch: BatchId,
    c: batch::BuiltLocusChange,
    state: &mut TickState,
) {
    let batch::BuiltLocusChange {
        change,
        locus_id,
        property_patch,
        cross_locus_preds,
        kind,
        resolved_slots,
        plasticity_active,
        post_signal,
        ..
    } = c;
    let id = change.id;
    state.acc.batch_changes.push(change);
    engine.apply_locus_property_patch(world, locus_id, property_patch);
    let inputs = CrossLocusInputs {
        batch,
        locus_id,
        kind,
        resolved_slots: &resolved_slots,
        plasticity_active,
        post_signal,
        trigger_id: id,
    };
    let mut context = cross_locus_context(world, influence_registry, inputs, state);
    engine.apply_cross_locus_emergence(&mut context, cross_locus_preds);
    record_committed_locus_change(state, locus_id, id);
}

struct CrossLocusInputs<'a> {
    batch: BatchId,
    locus_id: LocusId,
    kind: InfluenceKindId,
    resolved_slots: &'a [graph_core::RelationshipSlotDef],
    plasticity_active: bool,
    post_signal: f32,
    trigger_id: ChangeId,
}

fn cross_locus_context<'a>(
    world: &'a mut World,
    influence_registry: &'a InfluenceKindRegistry,
    inputs: CrossLocusInputs<'a>,
    state: &'a mut TickState,
) -> CrossLocusContext<'a> {
    CrossLocusContext {
        world,
        influence_registry,
        batch: inputs.batch,
        locus_id: inputs.locus_id,
        kind: inputs.kind,
        resolved_slots: inputs.resolved_slots,
        plasticity_active: inputs.plasticity_active,
        post_signal: inputs.post_signal,
        trigger_id: inputs.trigger_id,
        state,
    }
}

fn record_committed_locus_change(state: &mut TickState, locus_id: LocusId, change_id: ChangeId) {
    state
        .acc
        .committed_ids_by_locus
        .entry(locus_id)
        .or_default()
        .push(change_id);
    if state.acc.affected_loci_set.insert(locus_id) {
        state.acc.affected_loci.push(locus_id);
    }
}

pub(super) fn apply_locus_property_patch(
    _engine: &Engine,
    world: &mut World,
    locus_id: LocusId,
    property_patch: Option<graph_core::Properties>,
) {
    let Some(patch) = property_patch else {
        return;
    };
    if let Some(props) = world.properties_mut().get_mut(locus_id) {
        props.extend(&patch);
    } else {
        world.properties_mut().insert(locus_id, patch);
    }
}

pub(super) struct AppliedCrossLocusEmergence {
    record: EmergenceRecord,
    new_emerged_state: Option<StateVector>,
    schema_violation: Option<(graph_core::LocusKindId, graph_core::LocusKindId)>,
    /// Phase 2b promotion path: when a `Pending` resolution accumulates
    /// past threshold and mints a relationship, this carries the full
    /// `contributing_changes` list so the resulting `Change.predecessors`
    /// reflects every observation that fed into the promotion. `None` for
    /// bypass-Create (single trigger, length-1 predecessors).
    promotion_predecessors: Option<smallvec::SmallVec<[ChangeId; 4]>>,
}

pub(super) fn apply_cross_locus_emergence(
    engine: &Engine,
    context: &mut CrossLocusContext<'_>,
    cross_locus_preds: Vec<batch::CrossLocusPred>,
) {
    let kind_cfg = context.influence_registry.get(context.kind);
    context.state.acc.events.reserve(cross_locus_preds.len());
    if context.plasticity_active {
        context
            .state
            .acc
            .plasticity_obs
            .reserve(cross_locus_preds.len());
    }
    context
        .state
        .acc
        .new_emerged_rels
        .reserve(cross_locus_preds.len());
    for pred in cross_locus_preds {
        let Some(applied) = apply_cross_locus_prediction(context, kind_cfg, pred) else {
            continue;
        };
        let record = &applied.record;
        engine.record_schema_violation(
            applied.schema_violation,
            context.kind,
            record.rel_id,
            context.state,
        );
        engine.record_plasticity_observation(record, context.state);
        engine.record_batch_kind_touch(
            record.from_locus,
            context.locus_id,
            context.kind,
            record.rel_id,
            kind_cfg,
            context.state,
        );
        engine.record_relationship_emergence(applied, context.state);
    }
}

fn apply_cross_locus_prediction(
    context: &mut CrossLocusContext<'_>,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    pred: batch::CrossLocusPred,
) -> Option<AppliedCrossLocusEmergence> {
    let batch::CrossLocusPred {
        from_locus,
        pre_signal,
        pred_batch,
        is_feedback,
        schema_violation,
        emergence,
        ..
    } = pred;
    let outcome = emergence_apply::apply_emergence(
        context.world,
        emergence,
        context.trigger_id,
        context.batch,
        context.kind,
        pre_signal,
        kind_cfg,
        context.resolved_slots,
    )?;
    Some(AppliedCrossLocusEmergence {
        record: EmergenceRecord {
            batch: context.batch,
            from_locus,
            to_locus: context.locus_id,
            kind: context.kind,
            rel_id: outcome.rel_id,
            trigger_id: context.trigger_id,
            is_new: outcome.is_new,
            pre_signal,
            pred_batch,
            is_feedback,
            plasticity_active: context.plasticity_active,
            post_signal: context.post_signal,
            post_locus: context.locus_id,
        },
        new_emerged_state: outcome.initial_state,
        schema_violation,
        promotion_predecessors: outcome.promotion_predecessors,
    })
}

pub(super) fn record_schema_violation(
    _engine: &Engine,
    schema_violation: Option<(graph_core::LocusKindId, graph_core::LocusKindId)>,
    kind: InfluenceKindId,
    rel_id: RelationshipId,
    state: &mut TickState,
) {
    if let Some((fk, tk)) = schema_violation {
        state.acc.events.push(WorldEvent::SchemaViolation {
            relationship: rel_id,
            kind,
            from_locus_kind: fk,
            to_locus_kind: tk,
        });
    }
}

pub(super) fn record_relationship_emergence(
    _engine: &Engine,
    applied: AppliedCrossLocusEmergence,
    state: &mut TickState,
) {
    let record = applied.record;
    if !record.is_new {
        return;
    }
    state.acc.events.push(WorldEvent::RelationshipEmerged {
        relationship: record.rel_id,
        from: record.from_locus,
        to: record.to_locus,
        kind: record.kind,
        trigger_change_id: record.trigger_id,
    });
    let predecessors = applied
        .promotion_predecessors
        .unwrap_or_else(|| smallvec::smallvec![record.trigger_id]);
    state.acc.new_emerged_rels.push((
        record.rel_id,
        predecessors,
        record.kind,
        applied
            .new_emerged_state
            .expect("new relationship must have initial state"),
    ));
}

pub(super) fn record_plasticity_observation(
    _engine: &Engine,
    record: &EmergenceRecord,
    state: &mut TickState,
) {
    if !record.plasticity_active {
        return;
    }
    let timing = if record.pred_batch < record.batch {
        if record.is_feedback {
            TimingOrder::PostFirst
        } else {
            TimingOrder::PreFirst
        }
    } else {
        TimingOrder::Simultaneous
    };
    state.acc.plasticity_obs.push(PlasticityObs {
        rel_id: record.rel_id,
        kind: record.kind,
        pre: record.pre_signal,
        post: record.post_signal,
        timing,
        post_locus: record.post_locus,
    });
}

pub(super) fn record_batch_kind_touch(
    _engine: &Engine,
    from_locus: LocusId,
    to_locus: LocusId,
    kind: InfluenceKindId,
    rel_id: RelationshipId,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    state: &mut TickState,
) {
    let ep_key = if kind_cfg.map(|k| k.symmetric).unwrap_or(false) {
        graph_core::Endpoints::symmetric(from_locus, to_locus).key()
    } else {
        graph_core::Endpoints::directed(from_locus, to_locus).key()
    };
    let entry = state.acc.batch_kind_touches.entry(ep_key).or_default();
    entry.0.insert(kind);
    entry.1.insert(rel_id);
}

pub(super) fn apply_relationship_change(
    _engine: &Engine,
    c: batch::BuiltRelChange,
    world: &mut World,
    state: &mut TickState,
) {
    let id = c.change.id;
    if let Some(rel) = world.relationships_mut().get_mut(c.rel_id) {
        rel.state = c.after;
        rel.lineage.last_touched_by = Some(id);
        rel.lineage.change_count += 1;
    }
    state.acc.batch_changes.push(c.change);
    if c.has_subscribers {
        state
            .acc
            .pending_rel_notifications
            .push((c.rel_id, id, c.kind, c.from, c.to));
    }
}

pub(super) fn append_emergence_changes(
    _engine: &Engine,
    world: &mut World,
    batch: BatchId,
    state: &mut TickState,
) {
    if state.acc.new_emerged_rels.is_empty() {
        return;
    }
    let n_new = state.acc.new_emerged_rels.len();
    let emerge_base = world.reserve_change_ids(n_new);
    world.log_mut().reserve(n_new);
    let emerge_changes: Vec<Change> = state
        .acc
        .new_emerged_rels
        .iter()
        .enumerate()
        .map(|(i, (rel_id, predecessors, kind, initial_state))| {
            let before = StateVector::zeros(initial_state.dim());
            Change {
                id: ChangeId(emerge_base.0 + i as u64),
                subject: ChangeSubject::Relationship(*rel_id),
                kind: *kind,
                predecessors: predecessors.to_vec(),
                before,
                after: initial_state.clone(),
                batch,
                wall_time: None,
                metadata: None,
            }
        })
        .collect();
    world.extend_batch_changes(emerge_changes);
}

pub(super) fn apply_structural_and_hebbian(
    engine: &Engine,
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    batch: BatchId,
    state: &mut TickState,
    telemetry: &mut TickTelemetry,
) {
    let tombstones = batch::apply_structural_proposals(
        world,
        std::mem::take(&mut state.acc.structural_proposals),
        influence_registry,
    );
    state.pending.extend(tombstones);

    let th = telemetry.start();
    let hebbian_changes =
        world_ops::compute_hebbian_effects(world, &state.acc.plasticity_obs, influence_registry);
    world_ops::apply_hebbian_effects(world, &hebbian_changes);
    world_ops::apply_interaction_effects(world, &state.acc.batch_kind_touches, influence_registry);
    engine.record_hebbian_effects(world, batch, hebbian_changes);
    TickTelemetry::record(&mut telemetry.hebbian, th);
}

pub(super) fn record_hebbian_effects(
    _engine: &Engine,
    world: &mut World,
    batch: BatchId,
    hebbian_effects: Vec<world_ops::HebbianEffect>,
) {
    if hebbian_effects.is_empty() {
        return;
    }

    let n_hebb = hebbian_effects.len();
    let hebb_base = world.reserve_change_ids(n_hebb);
    world.log_mut().reserve(n_hebb);
    let hebb_log: Vec<Change> = hebbian_effects
        .into_iter()
        .enumerate()
        .map(|(i, effect)| Change {
            id: ChangeId(hebb_base.0 + i as u64),
            subject: ChangeSubject::Relationship(effect.rel_id),
            kind: effect.kind,
            predecessors: vec![],
            before: effect.before,
            after: effect.after,
            batch,
            wall_time: None,
            metadata: None,
        })
        .collect();
    world.extend_batch_changes(hebb_log);
}
