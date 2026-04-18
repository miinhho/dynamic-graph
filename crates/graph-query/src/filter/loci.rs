use super::metrics::{
    locus_degree_inner, locus_in_degree_inner, locus_out_degree_inner,
    most_connected_loci_with_degree_inner,
};
use graph_core::{Locus, LocusId, LocusKindId};
use graph_world::World;

pub fn locus_degree(world: &World, locus: LocusId) -> usize {
    locus_degree_inner(world, locus)
}

pub fn locus_in_degree(world: &World, locus: LocusId) -> usize {
    locus_in_degree_inner(world, locus)
}

pub fn locus_out_degree(world: &World, locus: LocusId) -> usize {
    locus_out_degree_inner(world, locus)
}

pub fn most_connected_loci(world: &World, n: usize) -> Vec<LocusId> {
    most_connected_loci_with_degree(world, n)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

pub fn most_connected_loci_with_degree(world: &World, n: usize) -> Vec<(LocusId, usize)> {
    most_connected_loci_with_degree_inner(world, n)
}

pub fn loci_top_n_by_state(world: &World, slot: usize, n: usize) -> Vec<&Locus> {
    if n == 0 {
        return Vec::new();
    }
    let mut loci: Vec<&Locus> = world
        .loci()
        .iter()
        .filter(|l| l.state.as_slice().len() > slot)
        .collect();
    loci.sort_unstable_by(|a, b| {
        let va = a.state.as_slice()[slot];
        let vb = b.state.as_slice()[slot];
        vb.total_cmp(&va)
    });
    loci.truncate(n);
    loci
}

pub fn loci_of_kind(world: &World, kind: LocusKindId) -> Vec<&Locus> {
    world.loci().iter().filter(|l| l.kind == kind).collect()
}

pub fn loci_with_state<F>(world: &World, slot: usize, pred: F) -> Vec<&Locus>
where
    F: Fn(f32) -> bool,
{
    world
        .loci()
        .iter()
        .filter(|l| l.state.as_slice().get(slot).is_some_and(|&v| pred(v)))
        .collect()
}

pub fn loci_with_str_property<'w, F>(world: &'w World, key: &str, pred: F) -> Vec<&'w Locus>
where
    F: Fn(&str) -> bool,
{
    world
        .loci()
        .iter()
        .filter(|l| {
            world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_str(key))
                .is_some_and(&pred)
        })
        .collect()
}

pub fn loci_with_f64_property<'w, F>(world: &'w World, key: &str, pred: F) -> Vec<&'w Locus>
where
    F: Fn(f64) -> bool,
{
    world
        .loci()
        .iter()
        .filter(|l| {
            world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_f64(key))
                .is_some_and(&pred)
        })
        .collect()
}

pub fn loci_matching<F>(world: &World, pred: F) -> Vec<&Locus>
where
    F: Fn(&Locus) -> bool,
{
    world.loci().iter().filter(|l| pred(l)).collect()
}
