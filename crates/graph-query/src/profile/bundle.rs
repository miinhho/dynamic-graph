use graph_core::{LocusId, Relationship};
use graph_world::World;

/// All relationships between a specific pair of loci, across every kind.
///
/// Obtained via [`relationship_profile`]. The bundle is valid for the lifetime
/// of the `&World` borrow it was created from.
///
/// Directionality is preserved in the stored `Relationship` references; the
/// bundle itself is **undirected** — it collects edges in either direction.
pub struct RelationshipBundle<'w> {
    /// One of the two loci (the one passed as `a` to [`relationship_profile`]).
    pub a: LocusId,
    /// The other locus.
    pub b: LocusId,
    /// Every relationship connecting `a` and `b`, in any direction and of any kind.
    pub relationships: Vec<&'w Relationship>,
}

/// Collect all relationships between `a` and `b` into a [`RelationshipBundle`].
///
/// Both directions are included (A→B, B→A, and Symmetric A↔B). Loci that
/// have no mutual relationships produce an empty bundle.
///
/// ```ignore
/// let bundle = relationship_profile(&world, sender, receiver);
/// if !bundle.is_empty() {
///     println!("net coupling: {:.2}", bundle.net_activity());
/// }
/// ```
pub fn relationship_profile<'w>(
    world: &'w World,
    a: LocusId,
    b: LocusId,
) -> RelationshipBundle<'w> {
    let relationships = world.relationships_between(a, b).collect();
    RelationshipBundle {
        a,
        b,
        relationships,
    }
}
