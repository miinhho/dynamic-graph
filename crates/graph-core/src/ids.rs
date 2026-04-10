//! Opaque newtype IDs used across the substrate.
//!
//! Every ID in the new substrate is a `u64` newtype. This matches the
//! resolved decision in `docs/redesign.md` §8 (O6): runtime IDs only,
//! registered through user-supplied registries, opaque to the engine.
//!
//! IDs intentionally implement `Copy` and `Hash` so they can be used in
//! the change-graph data structures without ceremony.

macro_rules! id_newtype {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(pub u64);
    };
}

id_newtype!(LocusId, "Identity of a single locus (Layer 0).");
id_newtype!(
    LocusKindId,
    "User-defined kind tag for a locus. Determines which program runs."
);
id_newtype!(
    InfluenceKindId,
    "User-defined influence kind. Per-kind stabilization, decay, and \
     regime classification keys off this."
);

/// Relationship kind identifier.
///
/// Per O8 in `docs/redesign.md` §8, a relationship's kind is the same
/// dimension as the influence kind that created it — there is no separate
/// kind space. This is a type alias, not a distinct newtype, so
/// `RelationshipKindId` and `InfluenceKindId` are interchangeable at
/// every call site. If a future use case needs sub-kinds, promote this
/// to a full newtype.
pub type RelationshipKindId = InfluenceKindId;

id_newtype!(ChangeId, "Identity of a single change (Layer 1).");
id_newtype!(
    BatchId,
    "Logical batch index — the engine's notion of time. A batch is a \
     maximal antichain in the change DAG. Batches are totally ordered by \
     causal dependency, but two changes in the same batch are causally \
     independent."
);
