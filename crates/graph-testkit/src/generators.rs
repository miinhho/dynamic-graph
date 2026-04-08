//! Deterministic pseudo-random world generators.
//!
//! All generators are seeded with a `u64` so tests are reproducible
//! without depending on external randomness. The underlying PRNG is a
//! plain 64-bit linear congruential generator — fast, zero-dependency,
//! and with enough period (2^64) for the world sizes used in tests.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use graph_testkit::generators::{LcgRng, random_chain_world};
//!
//! // Reproducible chain of 3–8 loci with a random gain in 0.3–0.7
//! let (world, loci_reg, inf_reg) = random_chain_world(42, 3, 8);
//! ```

use graph_engine::{InfluenceKindRegistry, LocusKindRegistry};
use graph_world::World;

use crate::fixtures::{chain_world, cyclic_pair_world, star_world};

/// A minimal 64-bit linear congruential generator.
///
/// Constants from Knuth (MMIX): `a = 6364136223846793005`, `c = 1442695040888963407`.
/// The full 64-bit period is 2^64.
pub struct LcgRng {
    state: u64,
}

impl LcgRng {
    const A: u64 = 6_364_136_223_846_793_005;
    const C: u64 = 1_442_695_040_888_963_407;

    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Advance and return the next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(Self::A).wrapping_add(Self::C);
        self.state
    }

    /// Uniform `u64` in `[lo, hi]` (inclusive). Panics if `lo > hi`.
    pub fn next_u64_range(&mut self, lo: u64, hi: u64) -> u64 {
        assert!(lo <= hi, "LcgRng::next_u64_range: lo > hi");
        if lo == hi {
            return lo;
        }
        lo + self.next_u64() % (hi - lo + 1)
    }

    /// Uniform `f32` in `[lo, hi)`.
    pub fn next_f32_range(&mut self, lo: f32, hi: f32) -> f32 {
        let t = (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32;
        lo + t * (hi - lo)
    }
}

/// Generate a random chain world.
///
/// - Chain length drawn uniformly from `[min_n, max_n]`.
/// - Gain drawn uniformly from `[0.3, 0.75]` — always below 1.0 so the
///   chain converges without hitting the batch cap.
pub fn random_chain_world(
    seed: u64,
    min_n: u64,
    max_n: u64,
) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut rng = LcgRng::new(seed);
    let n = rng.next_u64_range(min_n, max_n);
    let gain = rng.next_f32_range(0.3, 0.75);
    chain_world(n, gain)
}

/// Generate a random cyclic-pair world.
///
/// - Gain drawn uniformly from `[0.1, 0.5]` — well below 1.0 so the
///   pair converges.
pub fn random_cyclic_pair_world(seed: u64) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut rng = LcgRng::new(seed);
    let gain = rng.next_f32_range(0.1, 0.5);
    cyclic_pair_world(gain)
}

/// Generate a random star world.
///
/// - Arm count drawn uniformly from `[min_arms, max_arms]`.
/// - Gain drawn uniformly from `[0.3, 0.8]`.
pub fn random_star_world(
    seed: u64,
    min_arms: u64,
    max_arms: u64,
) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut rng = LcgRng::new(seed);
    let arms = rng.next_u64_range(min_arms, max_arms);
    let gain = rng.next_f32_range(0.3, 0.8);
    star_world(arms, gain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_is_deterministic() {
        let mut a = LcgRng::new(999);
        let mut b = LcgRng::new(999);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn lcg_range_stays_in_bounds() {
        let mut rng = LcgRng::new(1234);
        for _ in 0..1000 {
            let v = rng.next_u64_range(3, 7);
            assert!((3..=7).contains(&v));
        }
    }

    #[test]
    fn lcg_f32_range_stays_in_bounds() {
        let mut rng = LcgRng::new(5678);
        for _ in 0..1000 {
            let v = rng.next_f32_range(0.3, 0.75);
            assert!((0.3..0.75).contains(&v), "v={v}");
        }
    }

    #[test]
    fn random_chain_world_produces_expected_locus_count() {
        let (world, _, _) = random_chain_world(42, 4, 8);
        let count = world.loci().iter().count();
        assert!((4..=8).contains(&count), "count={count}");
    }
}
