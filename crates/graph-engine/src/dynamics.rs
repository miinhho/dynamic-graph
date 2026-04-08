use graph_core::{EntityKindId, EntityProgram};
use rustc_hash::FxHashMap;

pub trait ProgramCatalog: Sync {
    fn get(&self, kind: EntityKindId) -> Option<&dyn EntityProgram>;
}

#[derive(Default)]
pub struct ProgramRegistry {
    entries: FxHashMap<EntityKindId, Box<dyn EntityProgram>>,
}

impl ProgramRegistry {
    pub fn insert(&mut self, kind: EntityKindId, program: Box<dyn EntityProgram>) {
        self.entries.insert(kind, program);
    }

    pub fn get(&self, kind: EntityKindId) -> Option<&dyn EntityProgram> {
        self.entries.get(&kind).map(Box::as_ref)
    }
}

impl ProgramCatalog for ProgramRegistry {
    fn get(&self, kind: EntityKindId) -> Option<&dyn EntityProgram> {
        self.get(kind)
    }
}
