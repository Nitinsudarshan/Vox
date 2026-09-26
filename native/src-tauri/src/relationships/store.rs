//! Persistence and query store for relationships between Relay objects.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::model::{RelationshipRecord, RelationshipType};

/// In-memory indexed store backed by lightweight JSON storage.
pub struct RelationshipStore {
    /// Behind a lock because a vault move repoints a live store — see
    /// [`Self::reopen`].
    storage_path: RwLock<PathBuf>,
    records: RwLock<Vec<RelationshipRecord>>,
}

impl RelationshipStore {
    /// Creates or opens a RelationshipStore at the given directory.
    pub fn new(vault_dir: &Path) -> Self {
        let storage_path = Self::index_path(vault_dir);
        let loaded = Self::load_from(&storage_path);
        Self {
            storage_path: RwLock::new(storage_path),
            records: RwLock::new(loaded),
        }
    }

    /// Where the index lives under a vault root, creating its directory.
    fn index_path(vault_dir: &Path) -> PathBuf {
        let dir = vault_dir.join("relationships");
        let _ = fs::create_dir_all(&dir);
        dir.join("index.json")
    }

    /// The records in an index file; empty when it is absent, and empty with
    /// a warning when it cannot be parsed.
    fn load_from(storage_path: &Path) -> Vec<RelationshipRecord> {
        let Ok(data) = fs::read_to_string(storage_path) else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<RelationshipRecord>>(&data).unwrap_or_else(|_| {
            tracing::warn!("{} was malformed; recovering from a clean state", storage_path.display());
            Vec::new()
        })
    }

    fn current_path(&self) -> PathBuf {
        self.storage_path.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Points this store at another vault root and loads what is there.
    ///
    /// The vault can be moved while Vox runs (Settings › Vault). Before this,
    /// the store kept the index it opened at launch, so after a move it went
    /// on reading the old vault and writing into it. Nothing at the old
    /// location is moved or deleted.
    pub fn reopen(&self, vault_dir: &Path) {
        let storage_path = Self::index_path(vault_dir);
        let loaded = Self::load_from(&storage_path);
        let mut records = self.records.write().unwrap_or_else(|e| e.into_inner());
        *self.storage_path.write().unwrap_or_else(|e| e.into_inner()) = storage_path;
        *records = loaded;
    }

    /// Persists current in-memory state to disk atomically.
    fn persist(&self) -> Result<(), String> {
        let records = self.records.read().map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&*records).map_err(|e| e.to_string())?;
        let storage_path = self.current_path();
        let tmp_path = storage_path.with_extension("tmp");
        fs::write(&tmp_path, json.as_bytes()).map_err(|e| e.to_string())?;
        fs::rename(&tmp_path, &storage_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Reloads records from disk.
    pub fn reload(&self) {
        let storage_path = self.current_path();
        if let Ok(data) = fs::read_to_string(&storage_path) {
            if let Ok(records) = serde_json::from_str::<Vec<RelationshipRecord>>(&data) {
                if let Ok(mut lock) = self.records.write() {
                    *lock = records;
                }
            }
        }
    }

    /// Adds or updates a relationship, validating endpoints and ensuring no supersedes cycle is introduced.
    pub fn add_relationship(&self, rel: RelationshipRecord) -> Result<(), String> {
        if rel.source_id.trim().is_empty() || rel.target_id.trim().is_empty() {
            return Err("Relationship endpoints cannot be empty".to_string());
        }
        if rel.source_id == rel.target_id {
            return Err(format!("Self-referential relationship is forbidden: {}", rel.source_id));
        }

        let mut records = self.records.write().map_err(|e| e.to_string())?;

        // Detect supersedes cycle if adding a supersedes link
        if rel.relationship_type == RelationshipType::Supersedes {
            let mut curr = rel.target_id.as_str();
            let mut visited = std::collections::HashSet::new();
            visited.insert(rel.source_id.as_str());

            while let Some(next_link) = records
                .iter()
                .find(|r| r.relationship_type == RelationshipType::Supersedes && r.source_id == curr)
            {
                if visited.contains(next_link.target_id.as_str()) {
                    return Err(format!(
                        "Cycle detected in supersedes chain: {} -> {}",
                        rel.source_id, rel.target_id
                    ));
                }
                visited.insert(curr);
                curr = next_link.target_id.as_str();
            }
        }

        // Deduplicate: replace existing with same ID or same (source, target, type)
        records.retain(|r| {
            !(r.id == rel.id
                || (r.source_id == rel.source_id
                    && r.target_id == rel.target_id
                    && r.relationship_type == rel.relationship_type))
        });

        records.push(rel);
        drop(records);
        self.persist()?;
        Ok(())
    }

    /// Retrieves all relationships where the object is the source.
    pub fn get_relationships_for_source(&self, source_id: &str) -> Vec<RelationshipRecord> {
        let records = self.records.read().unwrap_or_else(|e| e.into_inner());
        records
            .iter()
            .filter(|r| r.source_id == source_id)
            .cloned()
            .collect()
    }

    /// Retrieves all relationships where the object is the target.
    pub fn get_relationships_for_target(&self, target_id: &str) -> Vec<RelationshipRecord> {
        let records = self.records.read().unwrap_or_else(|e| e.into_inner());
        records
            .iter()
            .filter(|r| r.target_id == target_id)
            .cloned()
            .collect()
    }

    /// Finds relationships connecting two objects in either direction.
    pub fn find_relationships_between(&self, id_a: &str, id_b: &str) -> Vec<RelationshipRecord> {
        let records = self.records.read().unwrap_or_else(|e| e.into_inner());
        records
            .iter()
            .filter(|r| {
                (r.source_id == id_a && r.target_id == id_b)
                    || (r.source_id == id_b && r.target_id == id_a)
            })
            .cloned()
            .collect()
    }

    /// Deletes a relationship by ID.
    pub fn delete_relationship(&self, id: &str) -> Result<bool, String> {
        let mut records = self.records.write().map_err(|e| e.to_string())?;
        let prev_len = records.len();
        records.retain(|r| r.id != id);
        let removed = records.len() < prev_len;
        drop(records);
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Returns all stored relationships.
    pub fn list_all(&self) -> Vec<RelationshipRecord> {
        let records = self.records.read().unwrap_or_else(|e| e.into_inner());
        records.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_at_another_vault_reads_and_writes_there() {
        let vault_a = std::env::temp_dir().join(format!("vox_test_rel_a_{}", uuid::Uuid::new_v4()));
        let vault_b = std::env::temp_dir().join(format!("vox_test_rel_b_{}", uuid::Uuid::new_v4()));
        let store = RelationshipStore::new(&vault_a);
        store
            .add_relationship(RelationshipRecord::new("note_a", "source_a", RelationshipType::Summarizes).unwrap())
            .unwrap();

        store.reopen(&vault_b);
        assert!(store.list_all().is_empty(), "vault B starts empty");
        store
            .add_relationship(RelationshipRecord::new("note_b", "source_b", RelationshipType::Summarizes).unwrap())
            .unwrap();

        store.reopen(&vault_a);
        let in_a = store.list_all();
        assert_eq!(in_a.len(), 1, "vault A was not written to while B was open");
        assert_eq!(in_a[0].source_id, "note_a");

        let _ = std::fs::remove_dir_all(vault_a);
        let _ = std::fs::remove_dir_all(vault_b);
    }

    #[test]
    fn test_store_crud_and_query() {
        let temp_dir = std::env::temp_dir().join(format!("relay_test_rel_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let store = RelationshipStore::new(&temp_dir);

        let rel1 = RelationshipRecord::new("summary_1", "source_1", RelationshipType::Summarizes).unwrap();
        let rel2 = RelationshipRecord::new("source_1", "repo_1", RelationshipType::BelongsTo).unwrap();

        assert!(store.add_relationship(rel1.clone()).is_ok());
        assert!(store.add_relationship(rel2.clone()).is_ok());

        let from_source = store.get_relationships_for_source("summary_1");
        assert_eq!(from_source.len(), 1);
        assert_eq!(from_source[0].target_id, "source_1");

        let for_target = store.get_relationships_for_target("source_1");
        assert_eq!(for_target.len(), 1);
        assert_eq!(for_target[0].source_id, "summary_1");

        let between = store.find_relationships_between("repo_1", "source_1");
        assert_eq!(between.len(), 1);
        assert_eq!(between[0].relationship_type, RelationshipType::BelongsTo);

        assert!(store.delete_relationship(&rel1.id).unwrap());
        assert_eq!(store.get_relationships_for_source("summary_1").len(), 0);

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_detect_supersedes_cycle() {
        let temp_dir = std::env::temp_dir().join(format!("relay_test_rel_cyc_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let store = RelationshipStore::new(&temp_dir);

        let rel1 = RelationshipRecord::new("mem_b", "mem_a", RelationshipType::Supersedes).unwrap();
        let rel2 = RelationshipRecord::new("mem_c", "mem_b", RelationshipType::Supersedes).unwrap();
        let cycle = RelationshipRecord::new("mem_a", "mem_c", RelationshipType::Supersedes).unwrap();

        assert!(store.add_relationship(rel1).is_ok());
        assert!(store.add_relationship(rel2).is_ok());
        assert!(store.add_relationship(cycle).is_err());

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
