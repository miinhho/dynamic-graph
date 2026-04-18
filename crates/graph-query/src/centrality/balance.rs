use graph_core::LocusId;
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum TriangleBalance {
    Balanced,
    Unstable,
}

pub fn triangle_balance(
    world: &World,
    a: LocusId,
    b: LocusId,
    c: LocusId,
    threshold: f32,
) -> Option<TriangleBalance> {
    let ab = edge_strength_between(world, a, b)?;
    let bc = edge_strength_between(world, b, c)?;
    let ac = edge_strength_between(world, a, c)?;
    let sign = |strength: f32| if strength > threshold { 1i32 } else { -1i32 };
    let product = sign(ab) * sign(bc) * sign(ac);
    if product > 0 {
        Some(TriangleBalance::Balanced)
    } else {
        Some(TriangleBalance::Unstable)
    }
}

pub fn all_triangles(world: &World) -> Vec<(LocusId, LocusId, LocusId)> {
    let adjacency = build_adj(world);
    let mut triangles = Vec::new();

    let mut loci: Vec<LocusId> = world.loci().iter().map(|locus| locus.id).collect();
    loci.sort();

    for &a in &loci {
        let Some(neighbors_a) = adjacency.get(&a) else {
            continue;
        };
        for &b in neighbors_a {
            if b <= a {
                continue;
            }
            let Some(neighbors_b) = adjacency.get(&b) else {
                continue;
            };
            for &c in neighbors_b {
                if c <= b {
                    continue;
                }
                if neighbors_a.contains(&c) {
                    triangles.push((a, b, c));
                }
            }
        }
    }

    triangles.sort();
    triangles
}

pub fn unstable_triangles(world: &World, threshold: f32) -> Vec<(LocusId, LocusId, LocusId)> {
    all_triangles(world)
        .into_iter()
        .filter(|&(a, b, c)| {
            triangle_balance(world, a, b, c, threshold) == Some(TriangleBalance::Unstable)
        })
        .collect()
}

pub fn balance_index(world: &World, threshold: f32) -> f32 {
    let triangles = all_triangles(world);
    if triangles.is_empty() {
        return 0.0;
    }
    let balanced = triangles
        .iter()
        .filter(|&&(a, b, c)| {
            triangle_balance(world, a, b, c, threshold) == Some(TriangleBalance::Balanced)
        })
        .count();
    balanced as f32 / triangles.len() as f32
}

fn edge_strength_between(world: &World, a: LocusId, b: LocusId) -> Option<f32> {
    let mut total = 0.0;
    let mut found = false;
    for relationship in world.relationships_between(a, b) {
        total += relationship.strength();
        found = true;
    }
    if found { Some(total) } else { None }
}

fn build_adj(world: &World) -> FxHashMap<LocusId, FxHashSet<LocusId>> {
    let mut adjacency: FxHashMap<LocusId, FxHashSet<LocusId>> = FxHashMap::default();
    for relationship in world.relationships().iter() {
        let (from, to) = match relationship.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        adjacency.entry(from).or_default().insert(to);
        adjacency.entry(to).or_default().insert(from);
    }
    adjacency
}
