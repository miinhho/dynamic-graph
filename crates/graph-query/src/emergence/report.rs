use graph_core::{EntityId, EntityStatus};
use graph_world::World;

use super::{
    DecayRates, EmergenceEntry, EmergenceReport, EmergenceSynergyEntry, EmergenceSynergyReport,
    UnmeasuredEntry, UnmeasuredReason, coherence_dense_series_inner, psi,
};

pub(super) fn emergence_report_inner(
    world: &World,
    decay_rates: Option<&DecayRates>,
) -> EmergenceReport {
    let mut emergent = Vec::new();
    let mut spurious = Vec::new();
    let mut unmeasured = Vec::new();
    let mut n_entities = 0;

    for entity in world.entities().iter() {
        n_entities += 1;

        if let Some(entry) = dormant_unmeasured_entry(entity.id, entity.status) {
            unmeasured.push(entry);
            continue;
        }

        match psi::psi_scalar_inner(world, entity.id, decay_rates) {
            Some(psi) => push_scalar_entry(entity.id, psi, &mut emergent, &mut spurious),
            None => unmeasured.push(component_history_entry(world, entity.id, decay_rates)),
        }
    }

    sort_emergence_entries(&mut emergent);
    sort_emergence_entries(&mut spurious);

    EmergenceReport {
        emergent,
        spurious,
        unmeasured,
        n_entities,
    }
}

pub(super) fn emergence_report_synergy_inner(
    world: &World,
    decay_rates: Option<&DecayRates>,
) -> EmergenceSynergyReport {
    let mut emergent = Vec::new();
    let mut spurious = Vec::new();
    let mut unmeasured = Vec::new();
    let mut n_entities = 0;

    for entity in world.entities().iter() {
        n_entities += 1;

        if let Some(entry) = dormant_unmeasured_entry(entity.id, entity.status) {
            unmeasured.push(entry);
            continue;
        }

        match psi::psi_synergy_inner(world, entity.id, decay_rates) {
            Some(psi) => push_synergy_entry(entity.id, psi, &mut emergent, &mut spurious),
            None => unmeasured.push(component_history_entry(world, entity.id, decay_rates)),
        }
    }

    sort_synergy_entries(&mut emergent);
    sort_synergy_entries(&mut spurious);

    EmergenceSynergyReport {
        emergent,
        spurious,
        unmeasured,
        n_entities,
    }
}

fn dormant_unmeasured_entry(entity_id: EntityId, status: EntityStatus) -> Option<UnmeasuredEntry> {
    matches!(status, EntityStatus::Dormant).then_some(UnmeasuredEntry {
        entity: entity_id,
        reason: UnmeasuredReason::Dormant,
    })
}

fn component_history_entry(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> UnmeasuredEntry {
    let window_len = coherence_dense_series_inner(world, entity_id, decay_rates).len();
    let reason = if window_len < 3 {
        UnmeasuredReason::InsufficientStableWindow {
            layer_count: window_len,
        }
    } else {
        UnmeasuredReason::NoComponentHistory
    };
    UnmeasuredEntry {
        entity: entity_id,
        reason,
    }
}

fn push_scalar_entry(
    entity_id: EntityId,
    psi: super::PsiResult,
    emergent: &mut Vec<EmergenceEntry>,
    spurious: &mut Vec<EmergenceEntry>,
) {
    let entry = EmergenceEntry {
        entity: entity_id,
        psi,
    };
    if entry.psi.psi > 0.0 {
        emergent.push(entry);
    } else {
        spurious.push(entry);
    }
}

fn push_synergy_entry(
    entity_id: EntityId,
    psi: super::PsiSynergyResult,
    emergent: &mut Vec<EmergenceSynergyEntry>,
    spurious: &mut Vec<EmergenceSynergyEntry>,
) {
    let entry = EmergenceSynergyEntry {
        entity: entity_id,
        psi,
    };
    if entry.psi.psi_corrected > 0.0 {
        emergent.push(entry);
    } else {
        spurious.push(entry);
    }
}

fn sort_emergence_entries(entries: &mut [EmergenceEntry]) {
    entries.sort_by(|a, b| {
        b.psi
            .psi
            .partial_cmp(&a.psi.psi)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn sort_synergy_entries(entries: &mut [EmergenceSynergyEntry]) {
    entries.sort_by(|a, b| {
        b.psi
            .psi_corrected
            .partial_cmp(&a.psi.psi_corrected)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}
