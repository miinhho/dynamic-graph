use graph_core::{Emission, EntityId};
use graph_tx::DeltaProvenance;
use rustc_hash::FxHashSet;

use crate::TickDiagnostics;

pub fn from_emissions(
    entity_id: EntityId,
    emissions: &[Emission],
    applied_alpha: f32,
) -> DeltaProvenance {
    let mut provenance = DeltaProvenance {
        component: vec![entity_id],
        iteration_count: 1,
        applied_alpha,
        causes: Vec::new(),
        source_entities: Vec::new(),
        channel_ids: Vec::new(),
        law_ids: Vec::new(),
        interaction_kinds: Vec::new(),
    };

    for emission in emissions {
        provenance.causes.push(emission.cause);
        if let Some(origin) = &emission.origin {
            provenance.source_entities.push(origin.source);
            provenance.channel_ids.push(origin.channel);
            provenance.law_ids.push(origin.law);
            provenance.interaction_kinds.push(origin.kind);
        }
    }

    provenance.causes.sort_by_key(|cause| cause.0);
    provenance.causes.dedup_by_key(|cause| cause.0);
    provenance.source_entities.sort_by_key(|id| id.0);
    provenance.source_entities.dedup_by_key(|id| id.0);
    provenance.channel_ids.sort_by_key(|id| id.0);
    provenance.channel_ids.dedup_by_key(|id| id.0);
    provenance.law_ids.sort_by_key(|id| id.0);
    provenance.law_ids.dedup_by_key(|id| id.0);
    provenance.interaction_kinds.sort();
    provenance.interaction_kinds.dedup();
    provenance
}

pub fn dedup_diagnostics(diagnostics: &mut TickDiagnostics) {
    dedup_and_sort(&mut diagnostics.fanout_capped_entities, |id| id.0);
    dedup_and_sort(&mut diagnostics.emitted_channels, |id| id.0);
    dedup_and_sort(&mut diagnostics.law_ids, |id| id.0);
    dedup_and_sort(&mut diagnostics.promoted_to_field, |id| id.0);
    dedup_and_sort(&mut diagnostics.promoted_to_cohort, |id| id.0);
    diagnostics.interaction_kinds.sort_unstable();
    diagnostics.interaction_kinds.dedup();
}

fn dedup_and_sort<T, K>(values: &mut Vec<T>, key: impl Fn(&T) -> K)
where
    T: Copy + Eq + std::hash::Hash,
    K: Ord,
{
    let mut unique = FxHashSet::with_capacity_and_hasher(values.len(), Default::default());
    values.retain(|value| unique.insert(*value));
    values.sort_unstable_by_key(key);
}
