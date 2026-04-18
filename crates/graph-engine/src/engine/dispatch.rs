use rayon::prelude::*;

use super::*;

pub(super) fn dispatch_affected_loci(
    engine: &Engine,
    world: &mut World,
    loci_registry: &LocusKindRegistry,
    slot_defs: &crate::registry::SlotDefsMap,
    batch: BatchId,
    state: &mut TickState,
    telemetry: &mut TickTelemetry,
) {
    let td = telemetry.start();
    engine.resolve_relationship_notifications(world, state);
    let prepared = engine.collect_dispatch_inputs(world, loci_registry, batch, state);
    let executed = engine.run_dispatches(world, slot_defs, batch, prepared);
    engine.collect_dispatch_outputs(loci_registry, batch, executed, state);
    TickTelemetry::record(&mut telemetry.dispatch, td);
}

pub(super) fn resolve_relationship_notifications(
    _engine: &Engine,
    world: &World,
    state: &mut TickState,
) {
    for (rel_id, change_id, kind, from, to) in state.acc.pending_rel_notifications.drain(..) {
        let subscribers = world
            .subscriptions()
            .collect_subscribers(rel_id, kind, from, to);
        for subscriber in subscribers {
            state
                .acc
                .committed_ids_by_locus
                .entry(subscriber)
                .or_default()
                .push(change_id);
            if state.acc.affected_loci_set.insert(subscriber) {
                state.acc.affected_loci.push(subscriber);
            }
        }
    }
}

pub(super) fn collect_dispatch_inputs<'a>(
    _engine: &Engine,
    world: &'a World,
    loci_registry: &'a LocusKindRegistry,
    batch: BatchId,
    state: &TickState,
) -> DispatchPrepared<'a> {
    let batch_num = batch.0;
    let inputs = state
        .acc
        .affected_loci
        .iter()
        .filter_map(|locus_id| {
            let locus = world.locus(*locus_id)?;
            let cfg = loci_registry.get_config(locus.kind)?;
            if cfg.refractory_batches > 0
                && let Some(&fired_at) = state.last_fired.get(locus_id)
                && batch_num.saturating_sub(fired_at) < cfg.refractory_batches as u64
            {
                return None;
            }
            let program = cfg.program.as_ref();
            let inbox: Vec<&Change> = state
                .acc
                .committed_ids_by_locus
                .get(locus_id)
                .map(|ids| ids.iter().filter_map(|id| world.log().get(*id)).collect())
                .unwrap_or_default();
            let derived: Vec<ChangeId> = inbox.iter().map(|c| c.id).collect();
            Some(DispatchInput {
                locus,
                program,
                inbox,
                derived,
            })
        })
        .collect();
    DispatchPrepared { inputs }
}

pub(super) fn run_dispatches<'a>(
    _engine: &Engine,
    world: &World,
    slot_defs: &crate::registry::SlotDefsMap,
    batch: BatchId,
    prepared: DispatchPrepared<'a>,
) -> DispatchExecuted<'a> {
    let batch_ctx = graph_world::BatchContext::new(
        graph_world::BatchStores {
            loci: world.loci(),
            relationships: world.relationships(),
            log: world.log(),
            entities: world.entities(),
            coheres: world.coheres(),
            properties: world.properties(),
        },
        batch,
        slot_defs,
    );

    let results = prepared
        .inputs
        .par_iter()
        .map(|inp| {
            let state = inp.program.process(inp.locus, &inp.inbox, &batch_ctx);
            let structural = inp
                .program
                .structural_proposals(inp.locus, &inp.inbox, &batch_ctx);
            (state, structural, inp.derived.clone())
        })
        .collect();
    DispatchExecuted {
        inputs: prepared.inputs,
        results,
    }
}

pub(super) fn collect_dispatch_outputs(
    _engine: &Engine,
    loci_registry: &LocusKindRegistry,
    batch: BatchId,
    executed: DispatchExecuted<'_>,
    state: &mut TickState,
) {
    let batch_num = batch.0;
    let DispatchExecuted { inputs, results } = executed;
    for (idx, (mut state_proposals, structural, derived)) in results.into_iter().enumerate() {
        if let Some(cfg) = loci_registry.get_config(inputs[idx].locus.kind)
            && let Some(max) = cfg.max_proposals_per_dispatch
        {
            state_proposals.truncate(max);
        }
        if !state_proposals.is_empty() {
            state.last_fired.insert(inputs[idx].locus.id, batch_num);
        }
        state
            .pending
            .extend(state_proposals.into_iter().map(|p| PendingChange {
                proposed: p,
                derived_predecessors: derived.clone(),
            }));
        state.acc.structural_proposals.extend(structural);
    }
}
