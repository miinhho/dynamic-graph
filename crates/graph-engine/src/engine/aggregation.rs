use graph_core::{CohortReducer, Emission, InteractionKind, LawId};
use graph_world::WorldSnapshot;
use rustc_hash::FxHashMap;

pub fn aggregate_cohort_emissions(
    world: WorldSnapshot<'_>,
    emissions: &[Emission],
) -> Vec<Emission> {
    let mut grouped: FxHashMap<(CohortReducer, LawId, InteractionKind), Vec<Emission>> =
        FxHashMap::default();

    for emission in emissions {
        let Some(origin) = &emission.origin else {
            continue;
        };
        let Some(channel) = world.channel(origin.channel) else {
            continue;
        };
        grouped
            .entry((channel.cohort_reducer, origin.law, origin.kind))
            .or_default()
            .push(emission.clone());
    }

    grouped
        .into_iter()
        .flat_map(|((reducer, _, _), entries)| reduce_group(entries, reducer).into_iter())
        .collect()
}

fn reduce_group(entries: Vec<Emission>, reducer: CohortReducer) -> Vec<Emission> {
    match reducer {
        CohortReducer::Sum => entries,
        CohortReducer::Mean => {
            let count = entries.len().max(1) as f32;
            entries
                .into_iter()
                .map(|mut emission| {
                    emission.signal = emission.signal.scaled(1.0 / count);
                    emission.magnitude = emission.signal.l2_norm();
                    emission
                })
                .collect()
        }
        CohortReducer::Max => entries
            .into_iter()
            .max_by(|lhs, rhs| lhs.magnitude.total_cmp(&rhs.magnitude))
            .into_iter()
            .collect(),
    }
}
