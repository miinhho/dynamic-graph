//! In-memory stores for each ontology layer.
//!
//! Each store manages a single layer's data with O(1) ID lookup
//! and reverse indexes for common query patterns.

pub mod change_log;
pub mod cohere_store;
pub mod entity_store;
pub mod locus_store;
pub mod name_index;
pub mod property_store;
pub mod relationship_store;
