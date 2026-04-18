use graph_core::{Endpoints, InfluenceKindId, LocusId};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use super::directed_path_of_kind;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransitiveRule {
    Product,
    Min,
    Mean,
}

pub fn has_cycle(world: &World) -> bool {
    let mut color: FxHashMap<LocusId, u8> = FxHashMap::default();

    for locus in world.loci().iter() {
        if color.get(&locus.id).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut stack: Vec<(LocusId, bool)> = vec![(locus.id, false)];
        let mut seen: FxHashSet<LocusId> = FxHashSet::default();
        while let Some((node, returning)) = stack.pop() {
            if returning {
                color.insert(node, 2);
                continue;
            }
            let current_color = color.get(&node).copied().unwrap_or(0);
            if current_color == 2 {
                continue;
            }
            if current_color == 1 {
                return true;
            }
            color.insert(node, 1);
            stack.push((node, true));
            seen.clear();
            for rel in world.relationships_for_locus(node) {
                if let Endpoints::Directed { from, to } = rel.endpoints
                    && from == node
                    && seen.insert(to)
                {
                    let target_color = color.get(&to).copied().unwrap_or(0);
                    if target_color == 1 {
                        return true;
                    }
                    if target_color == 0 {
                        stack.push((to, false));
                    }
                }
            }
        }
    }
    false
}

pub fn infer_transitive(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: InfluenceKindId,
    rule: TransitiveRule,
) -> Option<f32> {
    if from == to {
        return None;
    }
    let path = directed_path_of_kind(world, from, to, kind)?;
    if path.len() < 2 {
        return None;
    }

    let activities: Vec<f32> = path
        .windows(2)
        .map(|window| {
            let (a, b) = (window[0], window[1]);
            world
                .relationships()
                .iter()
                .find(|relationship| {
                    relationship.kind == kind
                        && matches!(
                            relationship.endpoints,
                            Endpoints::Directed { from: fa, to: tb } if fa == a && tb == b
                        )
                })
                .map(|relationship| relationship.activity())
                .unwrap_or(0.0)
        })
        .collect();

    if activities.is_empty() {
        return None;
    }

    Some(match rule {
        TransitiveRule::Product => activities.iter().product(),
        TransitiveRule::Min => activities.iter().copied().fold(f32::INFINITY, f32::min),
        TransitiveRule::Mean => activities.iter().sum::<f32>() / activities.len() as f32,
    })
}
