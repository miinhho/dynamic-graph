//! `BatchContext` — the concrete `LocusContext` implementation passed to
//! `LocusProgram::process` during a batch dispatch.
//!
//! Holds shared references to the locus and relationship stores that are
//! valid for the duration of the dispatch phase (i.e. after the current
//! batch's changes were committed, before any new mutations). Programs
//! receive a `&dyn LocusContext` pointing to one of these; the reference
//! is only live during the single `process()` call.

use rustc_hash::FxHashMap;

use graph_core::{
    BatchId, Change, Cohere, Entity, EntityId, Locus, LocusContext, LocusId, Properties,
    Relationship, RelationshipId, RelationshipKindId, RelationshipSlotDef,
};

use crate::store::change_log::ChangeLog;
use crate::store::property_store::PropertyStore;
use crate::{CohereStore, EntityStore, LocusStore, RelationshipStore};

/// Read-only view of the world's stores for one batch dispatch.
/// Constructed by the engine before calling `LocusProgram::process`
/// and dropped immediately after.
pub struct BatchContext<'a> {
    pub(crate) loci: &'a LocusStore,
    pub(crate) relationships: &'a RelationshipStore,
    pub(crate) log: &'a ChangeLog,
    pub(crate) entities: &'a EntityStore,
    pub(crate) coheres: &'a CohereStore,
    pub(crate) batch: BatchId,
    /// Domain-level properties per locus (set via `ingest()` or
    /// `world.properties_mut()`). Programs can read these to access
    /// human-readable labels, type tags, and other domain data.
    pub(crate) properties: &'a PropertyStore,
    /// Reverse index: locus → owning active entity. Built once at
    /// context creation to make `entity_of()` O(1). When a locus
    /// belongs to multiple active entities, the one with the highest
    /// coherence wins.
    locus_to_entity: FxHashMap<LocusId, EntityId>,
    /// Borrowed reference to the pre-built slot-definitions map from
    /// `InfluenceKindRegistry`. Avoids per-batch cloning.
    slot_defs: &'a FxHashMap<RelationshipKindId, Vec<RelationshipSlotDef>>,
}

impl<'a> BatchContext<'a> {
    pub fn new(
        stores: BatchStores<'a>,
        batch: BatchId,
        slot_defs: &'a FxHashMap<RelationshipKindId, Vec<RelationshipSlotDef>>,
    ) -> Self {
        let locus_to_entity = build_locus_entity_index(stores.entities);

        Self {
            loci: stores.loci,
            relationships: stores.relationships,
            log: stores.log,
            entities: stores.entities,
            coheres: stores.coheres,
            batch,
            properties: stores.properties,
            locus_to_entity,
            slot_defs,
        }
    }
}

pub struct BatchStores<'a> {
    pub loci: &'a LocusStore,
    pub relationships: &'a RelationshipStore,
    pub log: &'a ChangeLog,
    pub entities: &'a EntityStore,
    pub coheres: &'a CohereStore,
    pub properties: &'a PropertyStore,
}

fn build_locus_entity_index(entities: &EntityStore) -> FxHashMap<LocusId, EntityId> {
    let mut winner: FxHashMap<LocusId, (EntityId, f32)> = FxHashMap::default();
    for entity in entities.active() {
        let coherence = entity.current.coherence;
        for &locus_id in &entity.current.members {
            let replace = winner
                .get(&locus_id)
                .is_none_or(|&(_, prev_coherence)| coherence > prev_coherence);
            if replace {
                winner.insert(locus_id, (entity.id, coherence));
            }
        }
    }
    winner
        .into_iter()
        .map(|(locus_id, (entity_id, _))| (locus_id, entity_id))
        .collect()
}

impl<'a> LocusContext for BatchContext<'a> {
    fn locus(&self, id: LocusId) -> Option<&Locus> {
        self.loci.get(id)
    }

    fn relationships_for<'b>(
        &'b self,
        locus: LocusId,
    ) -> Box<dyn Iterator<Item = &'b Relationship> + 'b> {
        Box::new(self.relationships.relationships_for_locus(locus))
    }

    fn recent_changes<'b>(
        &'b self,
        locus: LocusId,
        since: BatchId,
    ) -> Box<dyn Iterator<Item = &'b Change> + 'b> {
        Box::new(
            self.log
                .changes_to_locus(locus)
                .take_while(move |c| c.batch.0 >= since.0),
        )
    }

    fn current_batch(&self) -> BatchId {
        self.batch
    }

    fn entity_of(&self, locus: LocusId) -> Option<&Entity> {
        self.locus_to_entity
            .get(&locus)
            .and_then(|eid| self.entities.get(*eid))
    }

    fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id)
    }

    fn coheres(&self, perspective: &str) -> Option<&[Cohere]> {
        self.coheres.get(perspective)
    }

    fn relationship(&self, id: RelationshipId) -> Option<&Relationship> {
        self.relationships.get(id)
    }

    fn properties(&self, id: LocusId) -> Option<&Properties> {
        self.properties.get(id)
    }

    fn recent_changes_to_relationship<'b>(
        &'b self,
        rel_id: RelationshipId,
        since: BatchId,
    ) -> Box<dyn Iterator<Item = &'b Change> + 'b> {
        Box::new(
            self.log
                .changes_to_relationship(rel_id)
                .take_while(move |c| c.batch.0 >= since.0),
        )
    }

    fn extra_slots_for_kind(&self, kind: RelationshipKindId) -> &[RelationshipSlotDef] {
        self.slot_defs
            .get(&kind)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
