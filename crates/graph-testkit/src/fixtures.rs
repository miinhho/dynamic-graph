//! Pre-built world configurations for common test topologies.
//!
//! Each fixture returns a fully wired `(World, LocusKindRegistry,
//! InfluenceKindRegistry)` triple that is ready to hand to
//! `Engine::tick`. The registries are pre-populated with
//! `TEST_KIND` and the appropriate programs; callers only need to
//! inject a stimulus.
//!
//! ## Topology overview
//!
//! | Function              | Shape                                          |
//! |-----------------------|------------------------------------------------|
//! | `chain_world(n, g)`   | L0 → L1 → … → L(n-1), last node is inert      |
//! | `cyclic_pair_world(g)`| L0 ⇆ L1 feedback loop (quiesces when g < 1)   |
//! | `star_world(arms, g)` | L0 (hub) → L1 … L(arms), spokes are inert     |

use graph_core::{Locus, LocusId, LocusKindId, StateVector};
use graph_engine::{InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry};
use graph_world::World;

use crate::programs::{BroadcastProgram, ForwardProgram, InertProgram, TEST_KIND};

/// Per-batch decay factor used in all testkit influence configs.
/// 0.9 lets signal attenuate naturally over multiple batches.
const DECAY: f32 = 0.9;

/// Map a `LocusId` to a unique `LocusKindId` so that each locus can
/// carry its own program without collision.
fn locus_kind(id: LocusId) -> LocusKindId {
    LocusKindId(id.0 + 1000)
}

/// Shared helper: insert the standard `TEST_KIND` influence config.
fn base_influence_registry() -> InfluenceKindRegistry {
    let mut reg = InfluenceKindRegistry::new();
    reg.insert(
        TEST_KIND,
        InfluenceKindConfig::new("test").with_decay(DECAY),
    );
    reg
}

/// A linear chain of `n` loci.
///
/// Topology: `L(0) → L(1) → … → L(n-1)`.
/// Each forwarding locus scales the signal by `gain` before passing it
/// along; the terminal locus runs `InertProgram`.
///
/// Useful for testing: batch propagation depth, DAG predecessor structure,
/// and quiescence when `gain < 1`.
pub fn chain_world(n: u64, gain: f32) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    assert!(n >= 1, "chain must have at least 1 locus");
    let mut world = World::new();
    let mut loci_reg = LocusKindRegistry::new();
    let inf_reg = base_influence_registry();

    for i in 0..n {
        let locus_id = LocusId(i);
        let kind_id = locus_kind(locus_id);
        world.insert_locus(Locus::new(locus_id, kind_id, StateVector::zeros(1)));
        if i < n - 1 {
            loci_reg.insert(
                kind_id,
                Box::new(ForwardProgram {
                    downstream: LocusId(i + 1),
                    gain,
                }),
            );
        } else {
            loci_reg.insert(kind_id, Box::new(InertProgram));
        }
    }

    (world, loci_reg, inf_reg)
}

/// A feedback loop between two loci: `L0 → L1 → L0`.
///
/// With `gain < 1` and the noise floor in `ForwardProgram` (0.001),
/// the loop quiesces without hitting the batch cap.
/// With `gain >= 1` the loop diverges — useful for testing guard rails.
pub fn cyclic_pair_world(gain: f32) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let kind_0 = LocusKindId(2000);
    let kind_1 = LocusKindId(2001);

    let mut world = World::new();
    let mut loci_reg = LocusKindRegistry::new();
    let inf_reg = base_influence_registry();

    world.insert_locus(Locus::new(LocusId(0), kind_0, StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(1), kind_1, StateVector::zeros(1)));
    loci_reg.insert(
        kind_0,
        Box::new(ForwardProgram {
            downstream: LocusId(1),
            gain,
        }),
    );
    loci_reg.insert(
        kind_1,
        Box::new(ForwardProgram {
            downstream: LocusId(0),
            gain,
        }),
    );

    (world, loci_reg, inf_reg)
}

/// A star topology: hub `L0` fans out to `arms` spoke loci `L1…L(arms)`.
///
/// The hub runs `BroadcastProgram`; all spokes run `InertProgram`.
/// Used for testing: fan-out relationship emergence, bounded activity
/// across multiple downstream loci, and entity recognition over
/// highly-connected subgraphs.
pub fn star_world(arms: u64, gain: f32) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    assert!(arms >= 1, "star must have at least 1 arm");
    let hub_kind = LocusKindId(3000);
    let spoke_kind = LocusKindId(3001);

    let mut world = World::new();
    let mut loci_reg = LocusKindRegistry::new();
    let inf_reg = base_influence_registry();

    let downstreams: Vec<LocusId> = (1..=arms).map(LocusId).collect();
    world.insert_locus(Locus::new(LocusId(0), hub_kind, StateVector::zeros(1)));
    loci_reg.insert(
        hub_kind,
        Box::new(BroadcastProgram { downstreams, gain }),
    );

    for i in 1..=arms {
        world.insert_locus(Locus::new(LocusId(i), spoke_kind, StateVector::zeros(1)));
    }
    loci_reg.insert(spoke_kind, Box::new(InertProgram));

    (world, loci_reg, inf_reg)
}

/// A single accumulator locus: `L0` adds `gain * incoming` to its own state.
///
/// Useful for testing: stabilization guard rails, divergence detection,
/// and per-kind alpha scaling via `AdaptiveGuardRail`.
pub fn accumulator_world(gain: f32) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    use crate::programs::AccumulatorProgram;
    let kind = LocusKindId(4000);

    let mut world = World::new();
    let mut loci_reg = LocusKindRegistry::new();
    let inf_reg = base_influence_registry();

    world.insert_locus(Locus::new(LocusId(0), kind, StateVector::zeros(1)));
    loci_reg.insert(kind, Box::new(AccumulatorProgram { gain }));

    (world, loci_reg, inf_reg)
}

/// The locus id of the first (or only) locus in any testkit fixture.
pub const FIRST_LOCUS: LocusId = LocusId(0);

/// Convenience: a `ProposedChange` that injects `value` into `FIRST_LOCUS`.
///
/// Use this to kick off a tick without manually building the full
/// `ProposedChange` struct.
pub fn stimulus(value: f32) -> graph_core::ProposedChange {
    graph_core::ProposedChange::new(
        graph_core::ChangeSubject::Locus(FIRST_LOCUS),
        TEST_KIND,
        StateVector::from_slice(&[value]),
    )
}
