//! [`DeclaredEntity`]: a named group of loci asserted to form a coherent unit.

use graph_core::LocusId;

/// Stable ID for a [`DeclaredEntity`] within a [`SchemaWorld`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclaredEntityId(pub u64);

/// A named group of loci declared to form a coherent unit.
///
/// Unlike emergent [`graph_core::Entity`] instances (which the engine derives
/// from behavioral clustering), a `DeclaredEntity` is explicitly stated by the
/// user. Examples: an org-chart team, a microservice boundary, a project squad.
///
/// Members may change over time. The history of membership changes is not tracked
/// here — replace the entity or use fact assertions for fine-grained temporal
/// membership tracking.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredEntity {
    pub id: DeclaredEntityId,
    pub name: String,
    pub members: Vec<LocusId>,
}

impl DeclaredEntity {
    pub fn new(id: DeclaredEntityId, name: impl Into<String>, members: Vec<LocusId>) -> Self {
        DeclaredEntity {
            id,
            name: name.into(),
            members,
        }
    }

    pub fn contains(&self, locus: LocusId) -> bool {
        self.members.contains(&locus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_checks_membership() {
        let e = DeclaredEntity::new(
            DeclaredEntityId(0),
            "team-alpha",
            vec![LocusId(1), LocusId(2), LocusId(3)],
        );
        assert!(e.contains(LocusId(2)));
        assert!(!e.contains(LocusId(99)));
    }
}
