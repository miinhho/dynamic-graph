use rayon::prelude::*;

use super::*;

pub(super) fn process_batch(
    engine: &Engine,
    world: &mut World,
    loci_registry: &LocusKindRegistry,
    influence_registry: &InfluenceKindRegistry,
    slot_defs: &crate::registry::SlotDefsMap,
    state: &mut TickState,
    telemetry: &mut TickTelemetry,
) {
    let batch = world.current_batch();
    state.acc.clear();

    let computed = compute_batch(state, world, influence_registry, telemetry);

    let ta = telemetry.start();
    settle_and_advance(
        engine,
        compute_or_empty_batch(
            engine,
            world,
            influence_registry,
            batch,
            computed,
            state,
            telemetry,
        ),
        SettleContext {
            world,
            loci_registry,
            influence_registry,
            slot_defs,
            state,
            telemetry,
        },
        batch,
    );
    TickTelemetry::record(&mut telemetry.apply, ta);
}

fn compute_batch(
    state: &mut TickState,
    world: &World,
    influence_registry: &InfluenceKindRegistry,
    telemetry: &mut TickTelemetry,
) -> ComputedBatch {
    let pending_batch: Vec<PendingChange> = std::mem::take(&mut state.pending);
    let t0 = telemetry.start();
    let computed = ComputedBatch {
        computed: pending_batch
            .into_par_iter()
            .map(|pc| compute_pending_change(pc, world, influence_registry))
            .collect(),
    };
    TickTelemetry::record(&mut telemetry.compute, t0);
    computed
}

enum BatchOrEmpty {
    Applied(AppliedBatch),
    Empty,
}

fn compute_or_empty_batch(
    engine: &Engine,
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    batch: BatchId,
    computed: ComputedBatch,
    state: &mut TickState,
    telemetry: &mut TickTelemetry,
) -> BatchOrEmpty {
    if computed.computed.is_empty() {
        return BatchOrEmpty::Empty;
    }

    let built = engine.build_changes(world, computed, batch, telemetry);
    if built.built.is_empty() {
        return BatchOrEmpty::Empty;
    }

    BatchOrEmpty::Applied(engine.apply_built_changes(
        world,
        influence_registry,
        batch,
        built,
        state,
        telemetry,
    ))
}

fn settle_and_advance(
    engine: &Engine,
    batch_state: BatchOrEmpty,
    mut settle_context: SettleContext<'_>,
    batch: BatchId,
) {
    let settled = match batch_state {
        BatchOrEmpty::Applied(applied) => engine.settle_batch(&mut settle_context, applied),
        BatchOrEmpty::Empty => engine.settle_empty_batch(&mut settle_context, batch),
    };
    engine.advance_batch(
        settle_context.world,
        settled,
        settle_context.state,
        settle_context.telemetry,
    );
}

pub(super) fn build_changes(
    _engine: &Engine,
    world: &mut World,
    computed_batch: ComputedBatch,
    batch: BatchId,
    telemetry: &mut TickTelemetry,
) -> BuiltBatch {
    let non_elided = collect_non_elided_changes(computed_batch);
    let n = non_elided.len();
    if n == 0 {
        return BuiltBatch { built: Vec::new() };
    }

    let base_id = reserve_build_ids(world, n);
    let tb = telemetry.start();
    let built = build_change_set(non_elided, base_id, batch);
    TickTelemetry::record(&mut telemetry.build, tb);
    BuiltBatch { built }
}

fn collect_non_elided_changes(computed_batch: ComputedBatch) -> Vec<(usize, ComputedChange)> {
    let mut idx = 0usize;
    computed_batch
        .computed
        .into_iter()
        .filter_map(|c| {
            if matches!(c, ComputedChange::Elided) {
                None
            } else {
                let i = idx;
                idx += 1;
                Some((i, c))
            }
        })
        .collect()
}

fn reserve_build_ids(world: &mut World, n_changes: usize) -> ChangeId {
    let base_id = world.reserve_change_ids(n_changes);
    world.log_mut().reserve(n_changes);
    base_id
}

fn build_change_set(
    non_elided: Vec<(usize, ComputedChange)>,
    base_id: ChangeId,
    batch: BatchId,
) -> Vec<BuiltChange> {
    const PAR_BUILD_THRESHOLD: usize = 512;
    if non_elided.len() >= PAR_BUILD_THRESHOLD {
        non_elided
            .into_par_iter()
            .map(|(i, c)| build_computed_change(i, base_id, c, batch))
            .collect()
    } else {
        non_elided
            .into_iter()
            .map(|(i, c)| build_computed_change(i, base_id, c, batch))
            .collect()
    }
}
