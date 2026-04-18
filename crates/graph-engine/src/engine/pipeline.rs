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

    let pending_batch: Vec<PendingChange> = std::mem::take(&mut state.pending);
    let t0 = telemetry.start();
    let computed = ComputedBatch {
        computed: pending_batch
            .into_par_iter()
            .map(|pc| compute_pending_change(pc, world, influence_registry))
            .collect(),
    };
    TickTelemetry::record(&mut telemetry.compute, t0);

    let ta = telemetry.start();
    if !computed.computed.is_empty() {
        let built = engine.build_changes(world, computed, batch, telemetry);
        if !built.built.is_empty() {
            let applied = engine.apply_built_changes(
                world,
                influence_registry,
                batch,
                built,
                state,
                telemetry,
            );
            let settled = engine.settle_batch(
                SettleContext {
                    world,
                    loci_registry,
                    influence_registry,
                    slot_defs,
                    state,
                    telemetry,
                },
                applied,
            );
            engine.advance_batch(world, settled, state, telemetry);
        } else {
            let settled = engine.settle_empty_batch(
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
            engine.advance_batch(world, settled, state, telemetry);
        }
    } else {
        let settled = engine.settle_empty_batch(
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
        engine.advance_batch(world, settled, state, telemetry);
    }
    TickTelemetry::record(&mut telemetry.apply, ta);
}

pub(super) fn build_changes(
    _engine: &Engine,
    world: &mut World,
    computed_batch: ComputedBatch,
    batch: BatchId,
    telemetry: &mut TickTelemetry,
) -> BuiltBatch {
    let non_elided: Vec<(usize, ComputedChange)> = {
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
    };

    let n = non_elided.len();
    if n == 0 {
        return BuiltBatch { built: Vec::new() };
    }

    let base_id = world.reserve_change_ids(n);
    world.log_mut().reserve(n);

    const PAR_BUILD_THRESHOLD: usize = 512;
    let tb = telemetry.start();
    let built = if n >= PAR_BUILD_THRESHOLD {
        non_elided
            .into_par_iter()
            .map(|(i, c)| build_computed_change(i, base_id, c, batch))
            .collect()
    } else {
        non_elided
            .into_iter()
            .map(|(i, c)| build_computed_change(i, base_id, c, batch))
            .collect()
    };
    TickTelemetry::record(&mut telemetry.build, tb);
    BuiltBatch { built }
}
