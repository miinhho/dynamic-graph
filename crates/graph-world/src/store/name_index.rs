//! `NameIndex` — bidirectional name ↔ `LocusId` lookup.
//!
//! Supports a canonical name per locus plus any number of aliases.
//! All lookups are case-sensitive; callers should normalise before
//! inserting if case-insensitive matching is desired.

use graph_core::LocusId;
use rustc_hash::FxHashMap;

/// Bidirectional index: canonical name ↔ `LocusId`, with alias support.
#[derive(Debug, Clone, Default)]
pub struct NameIndex {
    /// Canonical name → LocusId.
    name_to_id: FxHashMap<String, LocusId>,
    /// LocusId → canonical name.
    id_to_name: FxHashMap<LocusId, String>,
    /// Alias → LocusId. Aliases are additional lookup keys that resolve
    /// to the same locus as the canonical name.
    alias_to_id: FxHashMap<String, LocusId>,
}

impl NameIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a canonical name for a locus. Returns `Some(old_id)` if
    /// the name was already registered (the mapping is replaced).
    pub fn insert(&mut self, name: impl Into<String>, id: LocusId) -> Option<LocusId> {
        let name = name.into();
        self.register_canonical(name, id)
    }

    /// Add an alias that resolves to the same `LocusId` as an existing
    /// canonical name. No-op if `id` has no canonical entry.
    pub fn add_alias(&mut self, alias: impl Into<String>, id: LocusId) {
        if self.has_canonical_name(id) {
            self.alias_to_id.insert(alias.into(), id);
        }
    }

    /// Look up by name — checks canonical names first, then aliases.
    pub fn resolve(&self, name: &str) -> Option<LocusId> {
        self.name_to_id
            .get(name)
            .or_else(|| self.alias_to_id.get(name))
            .copied()
    }

    /// Get the canonical name for a locus.
    pub fn name_of(&self, id: LocusId) -> Option<&str> {
        self.id_to_name.get(&id).map(|s| s.as_str())
    }

    /// Remove a locus from the index (canonical + all aliases).
    pub fn remove(&mut self, id: LocusId) {
        self.remove_canonical(id);
        self.remove_aliases_for(id);
    }

    pub fn len(&self) -> usize {
        self.name_to_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.name_to_id.is_empty()
    }

    /// Iterate over all (canonical_name, LocusId) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, LocusId)> {
        self.name_to_id.iter().map(|(k, &v)| (k.as_str(), v))
    }

    /// Iterate over all (alias, LocusId) pairs.
    pub fn aliases(&self) -> impl Iterator<Item = (&str, LocusId)> {
        self.alias_to_id.iter().map(|(k, &v)| (k.as_str(), v))
    }

    /// Bulk-reconstruct from canonical names and aliases (used by snapshot restore).
    pub fn from_entries(
        names: impl IntoIterator<Item = (String, LocusId)>,
        aliases: impl IntoIterator<Item = (String, LocusId)>,
    ) -> Self {
        let mut idx = Self::new();
        idx.insert_entries(names);
        idx.insert_alias_entries(aliases);
        idx
    }

    fn has_canonical_name(&self, id: LocusId) -> bool {
        self.id_to_name.contains_key(&id)
    }

    fn register_canonical(&mut self, name: String, id: LocusId) -> Option<LocusId> {
        let previous = self.name_to_id.insert(name.clone(), id);
        self.id_to_name.insert(id, name);
        previous
    }

    fn remove_canonical(&mut self, id: LocusId) {
        if let Some(name) = self.id_to_name.remove(&id) {
            self.name_to_id.remove(&name);
        }
    }

    fn remove_aliases_for(&mut self, id: LocusId) {
        self.alias_to_id.retain(|_, alias_id| *alias_id != id);
    }

    fn insert_entries(&mut self, names: impl IntoIterator<Item = (String, LocusId)>) {
        for (name, id) in names {
            self.insert(name, id);
        }
    }

    fn insert_alias_entries(&mut self, aliases: impl IntoIterator<Item = (String, LocusId)>) {
        for (alias, id) in aliases {
            self.add_alias(alias, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_resolve() {
        let mut idx = NameIndex::new();
        idx.insert("Apple", LocusId(1));
        assert_eq!(idx.resolve("Apple"), Some(LocusId(1)));
        assert_eq!(idx.name_of(LocusId(1)), Some("Apple"));
    }

    #[test]
    fn alias_resolves_to_same_id() {
        let mut idx = NameIndex::new();
        idx.insert("Apple", LocusId(1));
        idx.add_alias("AAPL", LocusId(1));
        idx.add_alias("Apple Inc.", LocusId(1));
        assert_eq!(idx.resolve("AAPL"), Some(LocusId(1)));
        assert_eq!(idx.resolve("Apple Inc."), Some(LocusId(1)));
        assert_eq!(idx.len(), 1); // only 1 canonical entry
    }

    #[test]
    fn canonical_takes_precedence_over_alias() {
        let mut idx = NameIndex::new();
        idx.insert("Apple", LocusId(1));
        idx.insert("AAPL", LocusId(2));
        idx.add_alias("AAPL", LocusId(1)); // alias conflicts with canonical
        // Canonical wins.
        assert_eq!(idx.resolve("AAPL"), Some(LocusId(2)));
    }

    #[test]
    fn remove_cleans_all() {
        let mut idx = NameIndex::new();
        idx.insert("Apple", LocusId(1));
        idx.add_alias("AAPL", LocusId(1));
        idx.remove(LocusId(1));
        assert!(idx.resolve("Apple").is_none());
        assert!(idx.resolve("AAPL").is_none());
        assert!(idx.is_empty());
    }

    #[test]
    fn unknown_name_returns_none() {
        let idx = NameIndex::new();
        assert!(idx.resolve("nothing").is_none());
    }
}
