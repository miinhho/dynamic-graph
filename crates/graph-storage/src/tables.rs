//! redb table definitions.
//!
//! All value columns store postcard-encoded bytes. Keys use native
//! integer types for efficient B-tree ordering; string keys (names,
//! aliases) use `&str`.

use redb::{MultimapTableDefinition, TableDefinition};

/// Locus records: `LocusId(u64)` → postcard `Locus`.
pub const LOCI: TableDefinition<u64, &[u8]> = TableDefinition::new("loci");

/// Relationship records: `RelationshipId(u64)` → postcard `Relationship`.
pub const RELATIONSHIPS: TableDefinition<u64, &[u8]> = TableDefinition::new("relationships");

/// Entity records: `EntityId(u64)` → postcard `Entity`.
pub const ENTITIES: TableDefinition<u64, &[u8]> = TableDefinition::new("entities");

/// Change records: `ChangeId(u64)` → postcard `Change`.
pub const CHANGES: TableDefinition<u64, &[u8]> = TableDefinition::new("changes");

/// Secondary index: batch → set of ChangeIds.
/// Enables `changes_for_batch(batch)` without scanning the full table.
pub const CHANGES_BY_BATCH: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("changes_by_batch");

/// Property records: `LocusId(u64)` → postcard `Properties`.
pub const PROPERTIES: TableDefinition<u64, &[u8]> = TableDefinition::new("properties");

/// Canonical name → `LocusId(u64)`.
pub const NAMES: TableDefinition<&str, u64> = TableDefinition::new("names");

/// Alias → `LocusId(u64)`.
pub const ALIASES: TableDefinition<&str, u64> = TableDefinition::new("aliases");

/// Subscription records: `LocusId(u64)` (subscriber) → multimap → `RelationshipId(u64)`.
pub const SUBSCRIPTIONS: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("subscriptions");

/// Secondary index for cold→hot promotion: `LocusId(u64)` → multimap → `RelationshipId(u64)`.
///
/// Allows `relationships_for_locus` to do an O(k) range scan instead of
/// an O(n) full RELATIONSHIPS table scan. Added in schema v3.
pub const REL_BY_LOCUS: MultimapTableDefinition<u64, u64> =
    MultimapTableDefinition::new("rel_by_locus");

/// BCM sliding threshold θ_M per locus: `LocusId(u64)` → postcard `f32`.
/// Only populated for simulations using BCM plasticity.
pub const BCM_THRESHOLDS: TableDefinition<u64, &[u8]> = TableDefinition::new("bcm_thresholds");

/// Metadata counters: string key → u64 value.
/// Keys: "current_batch", "next_change_id", "next_relationship_id", "next_entity_id", "schema_version".
pub const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

// ── meta key constants ──────────────────────────────────────────────

pub const META_CURRENT_BATCH: &str = "current_batch";
pub const META_NEXT_CHANGE_ID: &str = "next_change_id";
pub const META_NEXT_RELATIONSHIP_ID: &str = "next_relationship_id";
pub const META_NEXT_ENTITY_ID: &str = "next_entity_id";
pub const META_SCHEMA_VERSION: &str = "schema_version";
