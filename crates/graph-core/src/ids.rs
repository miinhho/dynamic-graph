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
     regime classification keys off this. Per O8 in redesign.md, this is \
     the same identifier as `RelationshipKindId`."
);
id_newtype!(ChangeId, "Identity of a single change (Layer 1).");
id_newtype!(
    BatchId,
    "Logical batch index — the engine's notion of time. A batch is a \
     maximal antichain in the change DAG. Batches are totally ordered by \
     causal dependency, but two changes in the same batch are causally \
     independent."
);
