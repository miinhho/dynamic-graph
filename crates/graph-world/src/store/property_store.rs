//! `PropertyStore` — domain-level property storage keyed by `LocusId`.
//!
//! Sits alongside the engine's `LocusStore` (which holds `StateVector`s)
//! and provides the human-readable, domain-specific data that the engine
//! itself never touches. Queries join `LocusId` across both stores.

use graph_core::{LocusId, Properties};
use rustc_hash::FxHashMap;

/// Maps `LocusId` → `Properties`. One entry per locus that was ingested
/// with domain data. Loci created directly via the low-level API may not
/// have a property entry.
#[derive(Debug, Clone, Default)]
pub struct PropertyStore {
    data: FxHashMap<LocusId, Properties>,
}

impl PropertyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: LocusId, properties: Properties) {
        self.data.insert(id, properties);
    }

    pub fn get(&self, id: LocusId) -> Option<&Properties> {
        self.data.get(&id)
    }

    pub fn get_mut(&mut self, id: LocusId) -> Option<&mut Properties> {
        self.data.get_mut(&id)
    }

    /// Update a single property on an existing entry. No-op if the locus
    /// has no property entry.
    pub fn set_property(
        &mut self,
        id: LocusId,
        key: impl Into<String>,
        value: impl Into<graph_core::PropertyValue>,
    ) {
        if let Some(props) = self.data.get_mut(&id) {
            props.set(key, value);
        }
    }

    pub fn remove(&mut self, id: LocusId) -> Option<Properties> {
        self.data.remove(&id)
    }

    pub fn contains(&self, id: LocusId) -> bool {
        self.data.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (LocusId, &Properties)> {
        self.data.iter().map(|(&id, props)| (id, props))
    }

    /// Bulk-reconstruct from (LocusId, Properties) pairs (used by snapshot restore).
    pub fn from_entries(entries: impl IntoIterator<Item = (LocusId, Properties)>) -> Self {
        let mut store = Self::new();
        for (id, props) in entries {
            store.insert(id, props);
        }
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::props;

    #[test]
    fn insert_and_get() {
        let mut store = PropertyStore::new();
        let id = LocusId(1);
        store.insert(id, props! { "name" => "Apple", "type" => "ORG" });
        let p = store.get(id).unwrap();
        assert_eq!(p.get_str("name"), Some("Apple"));
        assert_eq!(p.get_str("type"), Some("ORG"));
    }

    #[test]
    fn set_property_updates_existing() {
        let mut store = PropertyStore::new();
        let id = LocusId(1);
        store.insert(id, props! { "name" => "Apple" });
        store.set_property(id, "name", "Apple Inc.");
        assert_eq!(store.get(id).unwrap().get_str("name"), Some("Apple Inc."));
    }

    #[test]
    fn remove_returns_properties() {
        let mut store = PropertyStore::new();
        let id = LocusId(1);
        store.insert(id, props! { "x" => 1_i64 });
        assert!(store.remove(id).is_some());
        assert!(store.get(id).is_none());
    }
}
