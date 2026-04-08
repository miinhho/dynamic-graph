use graph_core::{EmissionLaw, LawId};
use rustc_hash::FxHashMap;

pub trait LawCatalog: Sync {
    fn get(&self, law_id: LawId) -> Option<&dyn EmissionLaw>;
}

#[derive(Default)]
pub struct LawRegistry {
    entries: FxHashMap<LawId, Box<dyn EmissionLaw>>,
}

impl LawRegistry {
    pub fn insert(&mut self, law_id: LawId, law: Box<dyn EmissionLaw>) {
        self.entries.insert(law_id, law);
    }

    pub fn get(&self, law_id: LawId) -> Option<&dyn EmissionLaw> {
        self.entries.get(&law_id).map(Box::as_ref)
    }
}

impl LawCatalog for LawRegistry {
    fn get(&self, law_id: LawId) -> Option<&dyn EmissionLaw> {
        self.get(law_id)
    }
}
