//! graph-schema: static declaration layer for the graph substrate.
//!
//! While `graph-engine` observes *emergent* structure from behavioral data,
//! this crate holds *declared* structure: facts that a user asserts about the
//! world regardless of observed activity.
//!
//! ## Core contrast
//!
//! | Layer | Source | Lifecycle |
//! |-------|--------|-----------|
//! | Dynamic (graph-world) | Observed cross-locus causal flow | Continuous; decays, strengthens via Hebbian plasticity |
//! | Static (graph-schema) | Explicit user declaration | Asserted and retracted by version; no decay |
//!
//! ## Key types
//!
//! - [`DeclaredRelKind`] — predicate category for a declared relationship
//!   (e.g. `"reports_to"`, `"is_member_of"`, `"contracted_with"`).
//! - [`DeclaredFact`] — a single time-versioned assertion between two loci.
//! - [`DeclaredEntity`] — a named group of loci declared to form a unit.
//! - [`DeclarationStore`] — append-only store; supports point-in-time queries.
//! - [`SchemaWorld`] — top-level container holding the store and entity registry.
//!
//! ## Time model
//!
//! `DeclarationStore` uses an internal **version counter** (not wall-clock or
//! batch ID) that increments on every mutation. `asserted_at` and `retracted_at`
//! are expressed in these versions, making point-in-time queries reproducible
//! without tying them to the dynamic world's clock.

pub mod entity;
pub mod fact;
pub mod store;
pub mod world;

pub use entity::{DeclaredEntity, DeclaredEntityId};
pub use fact::{DeclaredFact, DeclaredFactId, DeclaredRelKind};
pub use store::DeclarationStore;
pub use world::SchemaWorld;
