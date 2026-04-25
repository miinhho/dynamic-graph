//! graph-world: in-memory stores and the `World` facade.
//!
//! Owns all five layer stores (Locus, Change, Relationship, Entity, Cohere)
//! and the `World` type that ties them together for the engine. See
//! `docs/identity.md` for the ontology.

pub mod context;
pub mod diff;
pub mod metrics;
pub mod store;
pub mod world;

pub use context::{BatchContext, BatchStores};
pub use diff::{RelationshipDelta, WorldDiff};
pub use graph_core::TrimSummary;
pub use metrics::{ACTIVITY_THRESHOLD, TOP_N, WorldMetrics};
pub use store::change_log::ChangeLog;
pub use store::cohere_store::{CohereSnapshot, CohereStore};
pub use store::entity_store::EntityStore;
pub use store::locus_store::LocusStore;
pub use store::name_index::NameIndex;
pub use store::pre_relationship_buffer::{PendingEvidence, PreRelationshipBuffer, RecordOutcome};
pub use store::property_store::PropertyStore;
pub use store::relationship_store::RelationshipStore;
pub use store::subscription_store::{SubscriptionEvent, SubscriptionScope, SubscriptionStore};
pub use world::{World, WorldMeta, WorldSnapshot};
