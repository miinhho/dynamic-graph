//! [`SchemaWorld`]: top-level container for the static declaration layer.

use rustc_hash::FxHashMap;

use crate::entity::{DeclaredEntity, DeclaredEntityId};
use crate::fact::{DeclaredFactId, DeclaredRelKind};
use crate::store::DeclarationStore;
use graph_core::LocusId;

/// Top-level container for the static declaration layer.
///
/// Owns a [`DeclarationStore`] (for pairwise facts) and a registry of
/// [`DeclaredEntity`] objects (for named groups of loci). Both share the same
/// `LocusId` space as the dynamic `World`, enabling boundary analysis without
/// any ID translation.
#[derive(Debug, Default, Clone)]
pub struct SchemaWorld {
    pub facts: DeclarationStore,
    entities: FxHashMap<DeclaredEntityId, DeclaredEntity>,
    next_entity_id: u64,
}

impl SchemaWorld {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Fact API (delegates to DeclarationStore) ──────────────────────────

    pub fn assert_fact(
        &mut self,
        subject: LocusId,
        predicate: DeclaredRelKind,
        object: LocusId,
    ) -> DeclaredFactId {
        self.facts.assert_fact(subject, predicate, object)
    }

    pub fn retract_fact(&mut self, id: DeclaredFactId) {
        self.facts.retract_fact(id);
    }

    // ── Entity API ────────────────────────────────────────────────────────

    /// Register a named entity. Returns its [`DeclaredEntityId`].
    pub fn declare_entity(
        &mut self,
        name: impl Into<String>,
        members: Vec<LocusId>,
    ) -> DeclaredEntityId {
        let id = DeclaredEntityId(self.next_entity_id);
        self.next_entity_id += 1;
        self.entities
            .insert(id, DeclaredEntity::new(id, name, members));
        id
    }

    /// Retrieve a declared entity by ID.
    pub fn entity(&self, id: DeclaredEntityId) -> Option<&DeclaredEntity> {
        self.entities.get(&id)
    }

    /// Retrieve a declared entity by name.
    pub fn entity_by_name(&self, name: &str) -> Option<&DeclaredEntity> {
        self.entities.values().find(|e| e.name == name)
    }

    /// Iterate all declared entities.
    pub fn entities(&self) -> impl Iterator<Item = &DeclaredEntity> {
        self.entities.values()
    }

    /// Declared entities that contain `locus` as a member.
    pub fn entities_containing(&self, locus: LocusId) -> impl Iterator<Item = &DeclaredEntity> {
        self.entities.values().filter(move |e| e.contains(locus))
    }

    /// Update the member list for an existing entity. Returns `false` if not found.
    pub fn update_entity_members(&mut self, id: DeclaredEntityId, members: Vec<LocusId>) -> bool {
        if let Some(e) = self.entities.get_mut(&id) {
            e.members = members;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact::DeclaredRelKind;

    #[test]
    fn assert_and_query_facts() {
        let mut w = SchemaWorld::new();
        w.assert_fact(LocusId(1), DeclaredRelKind::new("reports_to"), LocusId(5));
        assert_eq!(w.facts.active_facts().count(), 1);
    }

    #[test]
    fn declare_and_lookup_entity() {
        let mut w = SchemaWorld::new();
        let id = w.declare_entity("engineering", vec![LocusId(1), LocusId(2)]);
        let e = w.entity(id).unwrap();
        assert_eq!(e.name, "engineering");
        assert!(e.contains(LocusId(1)));
    }

    #[test]
    fn entity_by_name_lookup() {
        let mut w = SchemaWorld::new();
        w.declare_entity("sales", vec![LocusId(10)]);
        assert!(w.entity_by_name("sales").is_some());
        assert!(w.entity_by_name("ghost").is_none());
    }

    #[test]
    fn entities_containing_filters_by_member() {
        let mut w = SchemaWorld::new();
        w.declare_entity("a", vec![LocusId(1), LocusId(2)]);
        w.declare_entity("b", vec![LocusId(2), LocusId(3)]);
        let matches: Vec<_> = w.entities_containing(LocusId(2)).collect();
        assert_eq!(matches.len(), 2);
        let only: Vec<_> = w.entities_containing(LocusId(1)).collect();
        assert_eq!(only.len(), 1);
    }
}
