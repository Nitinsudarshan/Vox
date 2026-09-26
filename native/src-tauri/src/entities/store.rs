//! Persistent store and index for canonical resolved entities.
//!
//! Stores entities and mentions in atomic JSON storage, supporting fast candidate
//! lookup during retrieval and context assembly.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::model::{EntityCategory, ResolvedEntity};

pub struct EntityStore {
    /// Behind a lock because a vault move repoints a live store — see
    /// [`Self::reopen`].
    storage_path: RwLock<PathBuf>,
    entities: RwLock<Vec<ResolvedEntity>>,
}

impl EntityStore {
    /// Creates or opens an EntityStore at the given vault directory.
    pub fn new(vault_dir: &Path) -> Self {
        let storage_path = Self::index_path(vault_dir);
        let loaded = Self::load_from(&storage_path);
        Self {
            storage_path: RwLock::new(storage_path),
            entities: RwLock::new(loaded),
        }
    }

    /// Where the index lives under a vault root, creating its directory.
    fn index_path(vault_dir: &Path) -> PathBuf {
        let dir = vault_dir.join("entities");
        let _ = fs::create_dir_all(&dir);
        dir.join("index.json")
    }

    /// The records in an index file; empty when it is absent, and empty with
    /// a warning when it cannot be parsed.
    fn load_from(storage_path: &Path) -> Vec<ResolvedEntity> {
        let Ok(data) = fs::read_to_string(storage_path) else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<ResolvedEntity>>(&data).unwrap_or_else(|_| {
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
        let mut records = self.entities.write().unwrap_or_else(|e| e.into_inner());
        *self.storage_path.write().unwrap_or_else(|e| e.into_inner()) = storage_path;
        *records = loaded;
    }

    /// Persists current state to disk atomically.
    fn persist(&self) -> Result<(), String> {
        let entities = self.entities.read().map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&*entities).map_err(|e| e.to_string())?;
        let storage_path = self.current_path();
        let tmp_path = storage_path.with_extension("tmp");
        fs::write(&tmp_path, json.as_bytes()).map_err(|e| e.to_string())?;
        fs::rename(&tmp_path, &storage_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Stores or merges a single resolved entity into the store.
    pub fn store_entity(&self, entity: ResolvedEntity) -> Result<(), String> {
        let mut list = self.entities.write().map_err(|e| e.to_string())?;

        // Check if an existing entity shares ID or canonical name + category
        let existing_idx = list.iter().position(|e| {
            e.id == entity.id
                || (e.category == entity.category
                    && e.canonical_name.to_lowercase() == entity.canonical_name.to_lowercase())
        });

        if let Some(idx) = existing_idx {
            let existing = &mut list[idx];
            for a in entity.aliases {
                if !existing.aliases.contains(&a) && existing.canonical_name != a {
                    existing.aliases.push(a);
                }
            }
            for sid in entity.source_identifiers {
                if !existing.source_identifiers.contains(&sid) {
                    existing.source_identifiers.push(sid);
                }
            }
            for u in entity.urls {
                if !existing.urls.contains(&u) {
                    existing.urls.push(u);
                }
            }
            for m in entity.mentions {
                if !existing.mentions.contains(&m) {
                    existing.mentions.push(m);
                }
            }
        } else {
            list.push(entity);
        }

        drop(list);
        self.persist()?;
        Ok(())
    }

    /// Stores a batch of resolved entities.
    pub fn store_entities(&self, entities: &[ResolvedEntity]) -> Result<(), String> {
        for ent in entities {
            self.store_entity(ent.clone())?;
        }
        Ok(())
    }

    /// Retrieves an entity by its ID.
    pub fn get_entity(&self, id: &str) -> Option<ResolvedEntity> {
        let list = self.entities.read().unwrap_or_else(|e| e.into_inner());
        list.iter().find(|e| e.id == id).cloned()
    }

    /// Finds an entity by canonical name or alias.
    pub fn find_by_name(&self, name: &str) -> Option<ResolvedEntity> {
        let lower = name.trim().to_lowercase();
        let list = self.entities.read().unwrap_or_else(|e| e.into_inner());
        list.iter()
            .find(|e| {
                e.canonical_name.to_lowercase() == lower
                    || e.aliases.iter().any(|a| a.to_lowercase() == lower)
            })
            .cloned()
    }

    /// Finds an entity by repo identifier or URL.
    pub fn find_by_identifier(&self, identifier: &str) -> Option<ResolvedEntity> {
        let lower = identifier.trim().to_lowercase();
        let list = self.entities.read().unwrap_or_else(|e| e.into_inner());
        list.iter()
            .find(|e| {
                e.source_identifiers.iter().any(|sid| sid.to_lowercase() == lower)
                    || e.urls.iter().any(|u| u.to_lowercase() == lower)
            })
            .cloned()
    }

    /// Lists all entities in the store.
    pub fn list_all(&self) -> Vec<ResolvedEntity> {
        let list = self.entities.read().unwrap_or_else(|e| e.into_inner());
        list.clone()
    }

    /// Lists entities matching a given category.
    pub fn list_by_category(&self, category: EntityCategory) -> Vec<ResolvedEntity> {
        let list = self.entities.read().unwrap_or_else(|e| e.into_inner());
        list.iter().filter(|e| e.category == category).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::model::EntityMention;

    fn entity(id: &str, name: &str) -> ResolvedEntity {
        ResolvedEntity {
            id: id.to_string(),
            canonical_name: name.to_string(),
            category: EntityCategory::Project,
            aliases: Vec::new(),
            source_identifiers: Vec::new(),
            urls: Vec::new(),
            confidence: 0.9,
            mentions: Vec::new(),
        }
    }

    #[test]
    fn reopening_at_another_vault_reads_and_writes_there() {
        let vault_a = std::env::temp_dir().join(format!("vox_test_ent_a_{}", uuid::Uuid::new_v4()));
        let vault_b = std::env::temp_dir().join(format!("vox_test_ent_b_{}", uuid::Uuid::new_v4()));
        let store = EntityStore::new(&vault_a);
        store.store_entity(entity("ent_a", "Alpha")).unwrap();

        store.reopen(&vault_b);
        assert!(store.list_all().is_empty(), "vault B starts empty");
        store.store_entity(entity("ent_b", "Beta")).unwrap();

        store.reopen(&vault_a);
        let in_a = store.list_all();
        assert_eq!(in_a.len(), 1, "vault A was not written to while B was open");
        assert_eq!(in_a[0].canonical_name, "Alpha");

        let _ = fs::remove_dir_all(vault_a);
        let _ = fs::remove_dir_all(vault_b);
    }

    #[test]
    fn test_entity_store_atomic_persistence_and_merge() {
        let temp_dir = std::env::temp_dir().join(format!("relay_ent_store_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);

        let store = EntityStore::new(&temp_dir);
        let mention1 = EntityMention {
            source_id: "src_1".to_string(),
            evidence: "Orca is an agent workflow platform.".to_string(),
            confidence: 0.95,
            timestamp: None,
        };

        let ent1 = ResolvedEntity {
            id: "ent_orca".to_string(),
            canonical_name: "Orca".to_string(),
            category: EntityCategory::Project,
            aliases: vec!["Orca Engine".to_string()],
            source_identifiers: vec!["stablyai/orca".to_string()],
            urls: vec!["https://github.com/stablyai/orca".to_string()],
            confidence: 0.95,
            mentions: vec![mention1],
        };

        store.store_entity(ent1).unwrap();

        // Verify retrieval by identifier
        let found = store.find_by_identifier("stablyai/orca");
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.canonical_name, "Orca");

        // Merge second mention
        let mention2 = EntityMention {
            source_id: "src_2".to_string(),
            evidence: "Using Orca for parallel workers.".to_string(),
            confidence: 0.90,
            timestamp: None,
        };
        let ent2 = ResolvedEntity {
            id: "ent_orca_2".to_string(),
            canonical_name: "Orca".to_string(),
            category: EntityCategory::Project,
            aliases: vec!["Orca Workflows".to_string()],
            source_identifiers: vec![],
            urls: vec![],
            confidence: 0.90,
            mentions: vec![mention2],
        };
        store.store_entity(ent2).unwrap();

        let updated = store.find_by_name("Orca").unwrap();
        assert_eq!(updated.mentions.len(), 2);
        assert!(updated.aliases.contains(&"Orca Workflows".to_string()));

        let _ = fs::remove_dir_all(temp_dir);
    }
}
