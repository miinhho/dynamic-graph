//! World snapshot, serialization, and ID-counter metadata.

use graph_core::{BatchId, Entity, Locus, LocusId, Properties, Relationship};

use super::World;

/// Opaque counter snapshot used by `graph-wal` for checkpoint and
/// recovery. Does not include program registries (those are re-supplied
/// by the caller at startup).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorldMeta {
    pub current_batch: BatchId,
    pub next_change_id: u64,
    pub next_relationship_id: u64,
    pub next_entity_id: u64,
}

/// Full in-memory snapshot of the world — used for checkpoint write and
/// recovery load. `CohereStore` is intentionally excluded (ephemeral).
///
/// The `log` field captures the retained change history at snapshot time.
/// After `trim_change_log`, only recent changes appear here — this is
/// intentional. Older history that was already trimmed is not recoverable
/// from a snapshot alone; the WAL provides deeper replay when needed.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorldSnapshot {
    pub loci: Vec<Locus>,
    pub relationships: Vec<Relationship>,
    pub entities: Vec<Entity>,
    pub log: Vec<graph_core::Change>,
    pub meta: WorldMeta,
    /// Domain-level properties per locus.
    #[cfg_attr(feature = "serde", serde(default))]
    pub properties: Vec<(LocusId, Properties)>,
    /// Canonical name → LocusId mappings.
    #[cfg_attr(feature = "serde", serde(default))]
    pub names: Vec<(String, LocusId)>,
    /// Alias → LocusId mappings.
    #[cfg_attr(feature = "serde", serde(default))]
    pub aliases: Vec<(String, LocusId)>,
    /// BCM sliding threshold θ_M per locus. Empty for non-BCM simulations.
    /// Serialised so θ_M survives save/load without needing to re-warm.
    #[cfg_attr(feature = "serde", serde(default))]
    pub bcm_thresholds: Vec<(LocusId, f32)>,
}

impl World {
    /// Metadata snapshot of the world's ID counters and batch clock.
    /// Used by `graph-wal` for checkpoint and recovery.
    pub fn world_meta(&self) -> WorldMeta {
        WorldMeta {
            current_batch: self.current_batch,
            next_change_id: self.next_change_id,
            next_relationship_id: self.relationships.next_id(),
            next_entity_id: self.entities.next_id(),
        }
    }

    /// Restore ID counters from a recovered `WorldMeta`. Called once
    /// after loading a checkpoint, before any engine activity.
    pub fn restore_meta(&mut self, meta: &WorldMeta) {
        self.current_batch = meta.current_batch;
        self.next_change_id = meta.next_change_id;
        self.relationships_mut().set_next_id(meta.next_relationship_id);
        self.entities_mut().set_next_id(meta.next_entity_id);
    }

    /// Capture the full mutable world state as a `WorldSnapshot`.
    ///
    /// The snapshot includes the retained change log (whatever survives
    /// `trim_change_log`), so a pure snapshot round-trip preserves recent
    /// causal history without WAL replay.
    ///
    /// `CohereStore` is excluded — it is ephemeral and is recomputed
    /// on demand via `extract_cohere`.
    pub fn to_snapshot(&self) -> WorldSnapshot {
        WorldSnapshot {
            loci: self.loci.iter().cloned().collect(),
            relationships: self.relationships.iter().cloned().collect(),
            entities: self.entities.iter().cloned().collect(),
            log: self.log.iter().cloned().collect(),
            meta: self.world_meta(),
            properties: self.properties.iter().map(|(id, p)| (id, p.clone())).collect(),
            names: self.names.iter().map(|(n, id)| (n.to_owned(), id)).collect(),
            aliases: self.names.aliases().map(|(a, id)| (a.to_owned(), id)).collect(),
            bcm_thresholds: self.bcm_thresholds().iter().map(|(&id, &v)| (id, v)).collect(),
        }
    }

    /// Restore a `World` from a `WorldSnapshot`.
    ///
    /// Returns a fresh, fully-populated world. The caller must
    /// re-register locus kind programs and influence kind configs
    /// (those live in the engine registries, not in the world).
    pub fn from_snapshot(snapshot: WorldSnapshot) -> Self {
        use crate::store::name_index::NameIndex;
        use crate::store::property_store::PropertyStore;

        let mut world = Self::default();
        for locus in snapshot.loci {
            world.insert_locus(locus);
        }
        for rel in snapshot.relationships {
            world.relationships_mut().insert(rel);
        }
        for entity in snapshot.entities {
            world.entities_mut().insert(entity);
        }
        for change in snapshot.log {
            world.log_mut().append(change);
        }
        world.properties = PropertyStore::from_entries(snapshot.properties);
        world.names = NameIndex::from_entries(snapshot.names, snapshot.aliases);
        for (id, theta) in snapshot.bcm_thresholds {
            world.bcm_thresholds_mut().insert(id, theta);
        }
        world.restore_meta(&snapshot.meta);
        world
    }

    /// Serialize this world to `path` using postcard + CRC32 framing.
    ///
    /// The format is identical to the WAL checkpoint so the file can be
    /// inspected with the same tooling. Requires the `serde` feature.
    #[cfg(feature = "serde")]
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let snapshot = self.to_snapshot();
        let payload = postcard::to_allocvec(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let crc = crc32fast::hash(&payload);
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&payload);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("bin.tmp");
        {
            let file = std::fs::File::create(&tmp)?;
            let mut writer = std::io::BufWriter::new(file);
            writer.write_all(&buf)?;
            writer.flush()?;
            writer.into_inner().map_err(|e| e.into_error())?.sync_data()?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Deserialize a world previously written with [`World::save`].
    ///
    /// Requires the `serde` feature.
    #[cfg(feature = "serde")]
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Read as _;
        let file = std::fs::File::open(path)?;
        let mut reader = std::io::BufReader::new(file);
        let mut header = [0u8; 8];
        reader.read_exact(&mut header)?;
        let payload_len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        let stored_crc = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let mut payload = vec![0u8; payload_len];
        reader.read_exact(&mut payload)?;
        let actual_crc = crc32fast::hash(&payload);
        if actual_crc != stored_crc {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("CRC mismatch: stored={stored_crc:#010x} actual={actual_crc:#010x}"),
            ));
        }
        let snapshot: WorldSnapshot = postcard::from_bytes(&payload)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self::from_snapshot(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{LocusKindId, StateVector, props};

    #[test]
    fn snapshot_round_trip_preserves_properties_and_names() {
        let mut world = World::new();
        let id0 = LocusId(0);
        let id1 = LocusId(1);
        world.insert_locus(Locus::new(id0, LocusKindId(1), StateVector::zeros(2)));
        world.insert_locus(Locus::new(id1, LocusKindId(1), StateVector::from_slice(&[0.5])));

        world.properties_mut().insert(id0, props! { "name" => "Apple", "type" => "ORG" });
        world.properties_mut().insert(id1, props! { "name" => "Google", "score" => 0.9_f64 });

        world.names_mut().insert("Apple", id0);
        world.names_mut().insert("Google", id1);
        world.names_mut().add_alias("AAPL", id0);
        world.names_mut().add_alias("GOOG", id1);

        let snapshot = world.to_snapshot();
        let restored = World::from_snapshot(snapshot);

        // Properties survived.
        assert_eq!(restored.properties().get(id0).unwrap().get_str("name"), Some("Apple"));
        assert_eq!(restored.properties().get(id0).unwrap().get_str("type"), Some("ORG"));
        assert_eq!(restored.properties().get(id1).unwrap().get_f64("score"), Some(0.9));

        // Names survived.
        assert_eq!(restored.names().resolve("Apple"), Some(id0));
        assert_eq!(restored.names().resolve("Google"), Some(id1));

        // Aliases survived.
        assert_eq!(restored.names().resolve("AAPL"), Some(id0));
        assert_eq!(restored.names().resolve("GOOG"), Some(id1));

        // Canonical name lookups work.
        assert_eq!(restored.names().name_of(id0), Some("Apple"));
        assert_eq!(restored.names().name_of(id1), Some("Google"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn save_load_preserves_properties_and_names() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("world.bin");

        let mut world = World::new();
        let id = LocusId(0);
        world.insert_locus(Locus::new(id, LocusKindId(1), StateVector::zeros(2)));
        world.properties_mut().insert(id, props! { "label" => "test", "weight" => 1.5_f64 });
        world.names_mut().insert("test_node", id);
        world.names_mut().add_alias("tn", id);

        world.save(&path).unwrap();
        let loaded = World::load(&path).unwrap();

        assert_eq!(loaded.properties().get(id).unwrap().get_str("label"), Some("test"));
        assert_eq!(loaded.properties().get(id).unwrap().get_f64("weight"), Some(1.5));
        assert_eq!(loaded.names().resolve("test_node"), Some(id));
        assert_eq!(loaded.names().resolve("tn"), Some(id));
    }
}
