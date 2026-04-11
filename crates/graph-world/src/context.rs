//! `BatchContext` — the concrete `LocusContext` implementation passed to
//! `LocusProgram::process` during a batch dispatch.
//!
//! Holds shared references to the locus and relationship stores that are
//! valid for the duration of the dispatch phase (i.e. after the current
//! batch's changes were committed, before any new mutations). Programs
//! receive a `&dyn LocusContext` pointing to one of these; the reference
//! is only live during the single `process()` call.

use rustc_hash::FxHashMap;

use graph_core::{BatchId, Change, Cohere, Entity, EntityId, Locus, LocusContext, LocusId, Relationship, RelationshipId, RelationshipKindId, RelationshipSlotDef};

use crate::store::change_log::ChangeLog;
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
        loci: &'a LocusStore,
        relationships: &'a RelationshipStore,
        log: &'a ChangeLog,
        entities: &'a EntityStore,
        coheres: &'a CohereStore,
        batch: BatchId,
        slot_defs: &'a FxHashMap<RelationshipKindId, Vec<RelationshipSlotDef>>,
    ) -> Self {
        // Build reverse index: for each active entity, map its members
        // to its id. If a locus appears in multiple entities, the one
        // with the highest coherence wins.
        let mut locus_to_entity = FxHashMap::default();
        let mut coherence_by_entity: FxHashMap<EntityId, f32> = FxHashMap::default();
        for entity in entities.active() {
            coherence_by_entity.insert(entity.id, entity.current.coherence);
            for &lid in &entity.current.members {
                let replace = match locus_to_entity.get(&lid) {
                    None => true,
                    Some(&existing_eid) => {
                        let existing_coh = coherence_by_entity.get(&existing_eid).copied().unwrap_or(0.0);
                        entity.current.coherence > existing_coh
                    }
                };
                if replace {
                    locus_to_entity.insert(lid, entity.id);
                }
            }
        }

        Self { loci, relationships, log, entities, coheres, batch, locus_to_entity, slot_defs }
    }
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

    fn extra_slots_for_kind(&self, kind: RelationshipKindId) -> &[RelationshipSlotDef] {
        self.slot_defs.get(&kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
}
