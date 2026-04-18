use rustc_hash::FxHashMap;

use graph_core::{
    BatchId, ChangeId, InfluenceKindId, LocusId, ProposedChange, RelationshipId, StateVector,
};
use graph_world::World;

use super::batch::{
    BuiltChange, ComputedChange, DispatchInput, DispatchResult, PartitionAccumulator, PendingChange,
};
use super::{InfluenceKindRegistry, LocusKindRegistry, TickResult};

pub(super) struct TickState {
    pub(super) result: TickResult,
    pub(super) pending: Vec<PendingChange>,
    pub(super) last_fired: FxHashMap<LocusId, u64>,
    pub(super) acc: PartitionAccumulator,
}

pub(super) struct CrossLocusContext<'a> {
    pub(super) world: &'a mut World,
    pub(super) influence_registry: &'a InfluenceKindRegistry,
    pub(super) batch: BatchId,
    pub(super) locus_id: LocusId,
    pub(super) kind: InfluenceKindId,
    pub(super) resolved_slots: &'a [graph_core::RelationshipSlotDef],
    pub(super) plasticity_active: bool,
    pub(super) post_signal: f32,
    pub(super) trigger_id: ChangeId,
    pub(super) state: &'a mut TickState,
}

pub(super) struct EmergenceRecord {
    pub(super) batch: BatchId,
    pub(super) from_locus: LocusId,
    pub(super) to_locus: LocusId,
    pub(super) kind: InfluenceKindId,
    pub(super) rel_id: RelationshipId,
    pub(super) trigger_id: ChangeId,
    pub(super) is_new: bool,
    pub(super) emerged_state: Option<StateVector>,
    pub(super) pre_signal: f32,
    pub(super) pred_batch: BatchId,
    pub(super) is_feedback: bool,
    pub(super) plasticity_active: bool,
    pub(super) post_signal: f32,
    pub(super) post_locus: LocusId,
}

pub(super) struct ComputedBatch {
    pub(super) computed: Vec<ComputedChange>,
}

pub(super) struct BuiltBatch {
    pub(super) built: Vec<BuiltChange>,
}

pub(super) struct SettleContext<'a> {
    pub(super) world: &'a mut World,
    pub(super) loci_registry: &'a LocusKindRegistry,
    pub(super) influence_registry: &'a InfluenceKindRegistry,
    pub(super) slot_defs: &'a crate::registry::SlotDefsMap,
    pub(super) state: &'a mut TickState,
    pub(super) telemetry: &'a mut TickTelemetry,
}

pub(super) struct AppliedBatch {
    pub(super) batch: BatchId,
}

pub(super) struct SettledBatch {
    pub(super) batch: BatchId,
}

pub(super) struct DispatchPrepared<'a> {
    pub(super) inputs: Vec<DispatchInput<'a>>,
}

pub(super) struct DispatchExecuted<'a> {
    pub(super) inputs: Vec<DispatchInput<'a>>,
    pub(super) results: Vec<DispatchResult>,
}

impl TickState {
    pub(super) fn new(stimuli: Vec<ProposedChange>) -> Self {
        Self {
            result: TickResult::default(),
            pending: stimuli
                .into_iter()
                .map(|proposed| PendingChange {
                    proposed,
                    derived_predecessors: Vec::new(),
                })
                .collect(),
            last_fired: FxHashMap::default(),
            acc: PartitionAccumulator::new(),
        }
    }
}

#[derive(Default)]
pub(super) struct TickTelemetry {
    pub(super) enabled: bool,
    pub(super) compute: std::time::Duration,
    pub(super) build: std::time::Duration,
    pub(super) apply: std::time::Duration,
    pub(super) apply_locus: std::time::Duration,
    pub(super) apply_emerge: std::time::Duration,
    pub(super) apply_changelog: std::time::Duration,
    pub(super) apply_b3: std::time::Duration,
    pub(super) dispatch: std::time::Duration,
    pub(super) hebbian: std::time::Duration,
    pub(super) other: std::time::Duration,
}

impl TickTelemetry {
    pub(super) fn new() -> Self {
        Self {
            enabled: std::env::var_os("GRAPH_ENGINE_PROFILE").is_some(),
            ..Self::default()
        }
    }

    pub(super) fn start(&self) -> Option<std::time::Instant> {
        self.enabled.then(std::time::Instant::now)
    }

    pub(super) fn record(target: &mut std::time::Duration, started: Option<std::time::Instant>) {
        if let Some(t0) = started {
            *target += t0.elapsed();
        }
    }

    pub(super) fn print(&self, batches: u32) {
        if !self.enabled {
            return;
        }
        eprintln!(
            "[engine profile] batches={} compute={:.1}ms build={:.1}ms apply={:.1}ms(locus={:.1} emerge={:.1} changelog={:.1} b3={:.1}) dispatch={:.1}ms hebbian={:.1}ms other={:.1}ms",
            batches,
            self.compute.as_secs_f64() * 1000.0,
            self.build.as_secs_f64() * 1000.0,
            self.apply.as_secs_f64() * 1000.0,
            self.apply_locus.as_secs_f64() * 1000.0,
            self.apply_emerge.as_secs_f64() * 1000.0,
            self.apply_changelog.as_secs_f64() * 1000.0,
            self.apply_b3.as_secs_f64() * 1000.0,
            self.dispatch.as_secs_f64() * 1000.0,
            self.hebbian.as_secs_f64() * 1000.0,
            self.other.as_secs_f64() * 1000.0,
        );
    }
}
