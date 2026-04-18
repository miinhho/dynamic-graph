use rustc_hash::FxHashMap;

use graph_core::{InfluenceKindId, ProposedChange, WorldEvent};

use super::{Simulation, StepObservation, runtime};
use crate::regime::RegimeClassifier;
use crate::registry::InfluenceKindRegistry;

pub(super) fn step(sim: &mut Simulation, stimuli: Vec<ProposedChange>) -> StepObservation {
    let stimuli = merge_pending_stimuli(sim, stimuli);
    let mut effective = build_effective_influences(sim);
    let kinds: Vec<InfluenceKindId> = effective.kinds().collect();
    let prev_batch = sim.prev_batch;
    let execution = sim.run_step_world_mutation(stimuli, &mut effective, &kinds, prev_batch);
    let scales = collect_guard_rail_scales(sim, &kinds);
    let plasticity_scales = collect_plasticity_scales(sim, &kinds);
    let events = collect_regime_shift_events(sim);

    if let Some(ref mut history) = sim.event_history {
        history.push(execution.summary.clone());
    }

    let obs = build_step_observation(sim, execution, scales, plasticity_scales, events);
    sim.fire_watches(&obs);
    obs
}

pub(super) fn merge_pending_stimuli(
    sim: &mut Simulation,
    stimuli: Vec<ProposedChange>,
) -> Vec<ProposedChange> {
    if sim.pending_stimuli.is_empty() {
        stimuli
    } else {
        let mut all = std::mem::take(&mut sim.pending_stimuli);
        all.extend(stimuli);
        all
    }
}

pub(super) fn build_effective_influences(sim: &Simulation) -> InfluenceKindRegistry {
    let mut effective = sim.base_influences.clone();
    let kinds: Vec<InfluenceKindId> = effective.kinds().collect();
    for kind in &kinds {
        if let Some(cfg) = effective.get_mut(*kind) {
            cfg.stabilization = sim
                .guard_rail
                .effective_stabilization_config(*kind, &cfg.stabilization);
            if let Some(ref learners) = sim.plasticity_learners {
                cfg.plasticity.learning_rate *= learners.current(*kind);
            }
        }
    }
    effective
}

pub(super) fn collect_guard_rail_scales(
    sim: &Simulation,
    kinds: &[InfluenceKindId],
) -> FxHashMap<InfluenceKindId, f32> {
    kinds
        .iter()
        .map(|&kind| (kind, sim.guard_rail.current_scale(kind)))
        .collect()
}

pub(super) fn collect_plasticity_scales(
    sim: &Simulation,
    kinds: &[InfluenceKindId],
) -> FxHashMap<InfluenceKindId, f32> {
    kinds
        .iter()
        .map(|&kind| (kind, sim.current_plasticity_scale(kind)))
        .collect()
}

pub(super) fn collect_regime_shift_events(sim: &mut Simulation) -> Vec<WorldEvent> {
    let regime = sim.classifier.classify(&sim.history);
    if regime == sim.prev_regime {
        return Vec::new();
    }
    let event = WorldEvent::RegimeShift {
        from: sim.prev_regime.to_tag(),
        to: regime.to_tag(),
    };
    sim.prev_regime = regime;
    vec![event]
}

pub(super) fn build_step_observation(
    sim: &Simulation,
    execution: runtime::StepExecution,
    scales: FxHashMap<InfluenceKindId, f32>,
    plasticity_scales: FxHashMap<InfluenceKindId, f32>,
    events: Vec<WorldEvent>,
) -> StepObservation {
    let runtime::StepExecution {
        tick,
        relationships,
        active_entities,
        summary,
        ..
    } = execution;
    StepObservation {
        tick,
        regime: sim.prev_regime,
        relationships,
        active_entities,
        scales,
        plasticity_scales,
        events,
        summary,
    }
}

pub(super) fn step_n(
    sim: &mut Simulation,
    n: usize,
    stimuli: Vec<ProposedChange>,
) -> Vec<StepObservation> {
    if n == 0 {
        return Vec::new();
    }
    step_until(sim, &mut |_, _| false, n, stimuli).0
}

pub(super) fn step_until(
    sim: &mut Simulation,
    pred: &mut impl FnMut(&StepObservation, &graph_world::World) -> bool,
    max_steps: usize,
    stimuli: Vec<ProposedChange>,
) -> (Vec<StepObservation>, bool) {
    let mut observations = Vec::new();
    let mut stimuli = Some(stimuli);
    for _ in 0..max_steps {
        let s = sim.step(stimuli.take().unwrap_or_default());
        let done = {
            let w = sim.world.read().unwrap();
            pred(&s, &w)
        };
        observations.push(s);
        if done {
            return (observations, true);
        }
    }
    (observations, false)
}
