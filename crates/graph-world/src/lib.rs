//! graph-world: in-memory locus store, change log, and the world that
//! ties them together.
//!
//! See `docs/redesign.md` for the framing. Higher-layer stores
//! (Relationship, Entity, Cohere) join `World` as their respective
//! layers land.

pub mod change_log;
pub mod locus_store;
pub mod relationship_store;
pub mod world;

pub use change_log::ChangeLog;
pub use locus_store::LocusStore;
pub use relationship_store::RelationshipStore;
pub use world::World;
