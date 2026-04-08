//! Deterministic random graph generators for fuzz/property tests.
//!
//! Uses a tiny linear-congruential generator so we don't need to pull in
//! `rand`. Same seed → same world, every time.

use graph_core::{Channel, Entity};
use graph_world::World;

use crate::fixtures::{entity, pairwise_channel, world_from_components};

#[derive(Debug, Clone, Copy, Default)]
pub struct Seed(pub u64);

/// Tiny LCG (Numerical Recipes constants). Cheap, deterministic, sufficient
/// for fuzz tests where we only need a reproducible permutation.
#[derive(Debug, Clone, Copy)]
pub struct Lcg {
    state: u64,
}

impl Lcg {
    pub fn from_seed(seed: Seed) -> Self {
        // Avoid the all-zero state which gives a degenerate sequence.
        let state = if seed.0 == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed.0 };
        Self { state }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    pub fn next_unit(&mut self) -> f32 {
        // Take the top 24 bits to give a uniform value in [0, 1).
        ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32)
    }

    pub fn next_below(&mut self, exclusive_max: u64) -> u64 {
        if exclusive_max == 0 {
            return 0;
        }
        self.next_u64() % exclusive_max
    }
}

/// Build a random pairwise-only world. `n` entities are arranged on a line,
/// each pair `(i, j)` with `i != j` is connected with probability
/// `edge_density.clamp(0, 1)`. Useful for fuzz tests on the SCC scheduler and
/// the convergence classifier.
pub fn random_pairwise_world(seed: Seed, n: u64, edge_density: f32) -> World {
    let n = n.max(2);
    let density = edge_density.clamp(0.0, 1.0);
    let mut rng = Lcg::from_seed(seed);

    let mut entities: Vec<Entity> = Vec::with_capacity(n as usize);
    for id in 1..=n {
        entities.push(entity(id, 1, (id - 1) as f32));
    }

    let mut channels: Vec<Channel> = Vec::new();
    let mut channel_id: u64 = 1;
    for src in 1..=n {
        for dst in 1..=n {
            if src == dst {
                continue;
            }
            if rng.next_unit() < density {
                channels.push(pairwise_channel(channel_id, src, dst, 1));
                channel_id += 1;
            }
        }
    }

    world_from_components(entities, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_is_deterministic_per_seed() {
        let mut a = Lcg::from_seed(Seed(42));
        let mut b = Lcg::from_seed(Seed(42));
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn random_pairwise_world_is_reproducible() {
        let world_a = random_pairwise_world(Seed(7), 8, 0.4);
        let world_b = random_pairwise_world(Seed(7), 8, 0.4);
        assert_eq!(
            world_a.channels().count(),
            world_b.channels().count(),
            "same seed must yield same channel count"
        );
    }

    #[test]
    fn density_zero_yields_no_channels() {
        let world = random_pairwise_world(Seed(1), 6, 0.0);
        assert_eq!(world.channels().count(), 0);
    }

    #[test]
    fn density_one_yields_full_graph() {
        let n = 5;
        let world = random_pairwise_world(Seed(1), n, 1.0);
        // n * (n - 1) directed edges expected.
        assert_eq!(world.channels().count() as u64, n * (n - 1));
    }
}
