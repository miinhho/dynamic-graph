//! graph-testkit: fixtures, assertions, and deterministic generators.
//!
//! Three modules cover the standard test vocabulary:
//!
//! - **`programs`** — stateless `LocusProgram` implementations
//!   (`InertProgram`, `ForwardProgram`, `AccumulatorProgram`,
//!   `BroadcastProgram`) and the shared `TEST_KIND` constant.
//!
//! - **`fixtures`** — pre-wired `(World, LocusKindRegistry,
//!   InfluenceKindRegistry)` triples for canonical topologies:
//!   `chain_world`, `cyclic_pair_world`, `star_world`,
//!   `accumulator_world`. Also exports `stimulus()` for kicking off
//!   a tick with minimal boilerplate.
//!
//! - **`assertions`** — panic-on-failure invariant checks:
//!   `assert_bounded_activity`, `assert_changes_form_dag`,
//!   `assert_entity_active`, `assert_settling`,
//!   `assert_unique_change_ids`, `assert_relationship_count`.
//!
//! - **`generators`** — LCG-seeded random world builders:
//!   `random_chain_world`, `random_cyclic_pair_world`,
//!   `random_star_world`. `LcgRng` is also exported for callers that
//!   need their own deterministic sequences.

pub mod assertions;
pub mod fixtures;
pub mod generators;
pub mod programs;
