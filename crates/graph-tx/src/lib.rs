//! graph-tx: batch transactions and the change log.
//!
//! Cleared in preparation for the redesign described in `docs/redesign.md`.
//! The new tx layer records the per-batch Change stream and exposes the
//! causal links downstream layers (Relationship, Entity) lift; types land
//! in follow-up commits.
