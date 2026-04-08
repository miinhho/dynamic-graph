//! graph-world: in-memory stores and the `World` facade.
//!
//! Owns all five layer stores (Locus, Change, Relationship, Entity, Cohere)
//! and the `World` type that ties them together for the engine. See
//! `docs/identity.md` for the ontology.

pub mod change_log;
pub mod cohere_store;
pub mod entity_store;
pub mod locus_store;
pub mod relationship_store;
pub mod world;

pub use change_log::ChangeLog;
pub use cohere_store::CohereStore;
pub use entity_store::EntityStore;
pub use locus_store::LocusStore;
pub use relationship_store::RelationshipStore;
pub use world::World;
