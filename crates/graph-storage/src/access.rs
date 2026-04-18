mod session;

use crate::{Storage, StorageCounts, StorageError};
use graph_core::{
    BatchId, Change, ChangeId, Entity, EntityId, Locus, LocusId, Properties, Relationship,
    RelationshipId,
};
use session::ReadSession;

impl Storage {
    pub fn get_locus(&self, id: LocusId) -> Result<Option<Locus>, StorageError> {
        ReadSession::new(self)?.read_locus(id)
    }

    pub fn all_relationships(&self) -> Result<Vec<Relationship>, StorageError> {
        ReadSession::new(self)?.read_all_relationships()
    }

    pub fn get_relationship(
        &self,
        id: RelationshipId,
    ) -> Result<Option<Relationship>, StorageError> {
        ReadSession::new(self)?.read_relationship(id)
    }

    pub fn get_entity(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        ReadSession::new(self)?.read_entity(id)
    }

    pub fn relationships_for_locus(
        &self,
        locus_id: LocusId,
    ) -> Result<Vec<Relationship>, StorageError> {
        ReadSession::new(self)?.read_relationships_for_locus(locus_id)
    }

    pub fn get_change(&self, id: ChangeId) -> Result<Option<Change>, StorageError> {
        ReadSession::new(self)?.read_change(id)
    }

    pub fn changes_for_batch(&self, batch: BatchId) -> Result<Vec<Change>, StorageError> {
        ReadSession::new(self)?.read_changes_for_batch(batch)
    }

    pub fn get_properties(&self, id: LocusId) -> Result<Option<Properties>, StorageError> {
        ReadSession::new(self)?.read_properties(id)
    }

    pub fn resolve_name(&self, name: &str) -> Result<Option<LocusId>, StorageError> {
        ReadSession::new(self)?.resolve_name(name)
    }

    pub fn table_counts(&self) -> Result<StorageCounts, StorageError> {
        ReadSession::new(self)?.table_counts()
    }
}
