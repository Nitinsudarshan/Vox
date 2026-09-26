//! Where provider API keys live: the OS credential store, not `settings.json`.
//!
//! Keys used to be written into `settings.json` in plain text, beside every
//! other preference, and handed to the webview whole by `get_settings`. Both
//! are fixed here:
//!
//! * **At rest** — [`AppSettings::save`](crate::settings::AppSettings::save)
//!   writes the keys to one credential-store entry (Windows Credential
//!   Manager, macOS Keychain, the Secret Service on Linux) and blanks them in
//!   the JSON; [`AppSettings::load`](crate::settings::AppSettings::load) puts
//!   them back in memory. A key found in an older `settings.json` is moved on
//!   the next save.
//! * **In the webview** — everything sent to the frontend goes through
//!   [`ProviderConfig::redacted`], which replaces each stored key with
//!   [`STORED_KEY_PLACEHOLDER`]. The settings page shows it as a filled
//!   password field; sending it back unchanged means "keep the stored key",
//!   which [`ProviderConfig::resolve_placeholders`] honours on save.
//!
//! When the credential store is unavailable — a Linux box with no Secret
//! Service running, say — the keys stay in `settings.json` as they always did,
//! with a warning in the log. Refusing to save a key the user just pasted
//! would be worse than storing it where it was.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::ProviderConfig;

/// What the webview sees in place of a stored key.
///
/// Deliberately not a plausible key for any provider, so it can never be
/// mistaken for one if it leaked into a request.
pub const STORED_KEY_PLACEHOLDER: &str = "__vox_stored_key__";

/// The credential-store service name, beside the OAuth token namespaces.
const SERVICE: &str = "com.vox.app.providers";
/// The single entry holding every provider key, as a JSON object.
const ACCOUNT: &str = "provider_keys";
/// The map key the legacy single `cloud_api_key` is stored under.
const LEGACY_KEY: &str = "cloud_api_key";

/// A place to keep one secret string.
pub trait SecretStore: Send + Sync {
    fn read(&self) -> Result<Option<String>, String>;
    fn write(&self, value: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

/// The OS credential store, via the `keyring` crate.
pub struct OsKeyring;

impl OsKeyring {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
    }
}

impl SecretStore for OsKeyring {
    fn read(&self) -> Result<Option<String>, String> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, value: &str) -> Result<(), String> {
        Self::entry()?.set_password(value).map_err(|e| e.to_string())
    }

    fn clear(&self) -> Result<(), String> {
        match Self::entry()?.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A store that is never available, so keys stay in the file.
///
/// What unit tests get: they must not write into the developer's (or a CI
/// runner's) real credential store, and every test that saves settings keeps
/// the behaviour it was written against.
pub struct Unavailable;

impl SecretStore for Unavailable {
    fn read(&self) -> Result<Option<String>, String> {
        Err("no credential store in tests".to_string())
    }

    fn write(&self, _value: &str) -> Result<(), String> {
        Err("no credential store in tests".to_string())
    }

    fn clear(&self) -> Result<(), String> {
        Err("no credential store in tests".to_string())
    }
}

/// Remembers what the wrapped store holds, so a save that did not touch the
/// keys — most of them: a pill position, a toggle — does not write the
/// credential store again.
pub struct CachedStore<S> {
    inner: S,
    /// `None` until first read or written; `Some(None)` means known empty.
    known: Mutex<Option<Option<String>>>,
}

impl<S> CachedStore<S> {
    pub const fn new(inner: S) -> Self {
        Self {
            inner,
            known: Mutex::new(None),
        }
    }

    fn known(&self) -> std::sync::MutexGuard<'_, Option<Option<String>>> {
        self.known.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<S: SecretStore> SecretStore for CachedStore<S> {
    fn read(&self) -> Result<Option<String>, String> {
        let value = self.inner.read()?;
        *self.known() = Some(value.clone());
        Ok(value)
    }

    fn write(&self, value: &str) -> Result<(), String> {
        if self.known().as_ref().is_some_and(|known| known.as_deref() == Some(value)) {
            return Ok(());
        }
        self.inner.write(value)?;
        *self.known() = Some(Some(value.to_string()));
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        if matches!(*self.known(), Some(None)) {
            return Ok(());
        }
        self.inner.clear()?;
        *self.known() = Some(None);
        Ok(())
    }
}

#[cfg(not(test))]
static OS_KEYRING: CachedStore<OsKeyring> = CachedStore::new(OsKeyring);

/// The store the app uses.
pub fn default_store() -> &'static dyn SecretStore {
    #[cfg(not(test))]
    {
        &OS_KEYRING
    }
    #[cfg(test)]
    {
        &Unavailable
    }
}

/// Every non-empty key in `config`, keyed as the store keeps them.
fn collect_keys(config: &ProviderConfig) -> BTreeMap<String, String> {
    let mut keys: BTreeMap<String, String> = config
        .provider_keys
        .iter()
        .filter(|(_, key)| !key.trim().is_empty() && key.as_str() != STORED_KEY_PLACEHOLDER)
        .map(|(slug, key)| (slug.clone(), key.clone()))
        .collect();
    if let Some(legacy) = config
        .cloud_api_key
        .as_ref()
        .filter(|key| !key.trim().is_empty() && key.as_str() != STORED_KEY_PLACEHOLDER)
    {
        keys.insert(LEGACY_KEY.to_string(), legacy.clone());
    }
    keys
}

/// Writes the keys to `store` and returns the copy of `config` to put on
/// disk: without them if the store took them, unchanged if it did not.
pub fn persist_keys(store: &dyn SecretStore, config: &ProviderConfig) -> ProviderConfig {
    let keys = collect_keys(config);
    let result = if keys.is_empty() {
        store.clear()
    } else {
        // A `BTreeMap` serialises in key order, so an unchanged map is an
        // identical string and a `CachedStore` skips the write.
        serde_json::to_string(&keys)
            .map_err(|e| e.to_string())
            .and_then(|json| store.write(&json))
    };
    if let Err(e) = result {
        if !keys.is_empty() {
            tracing::warn!("Credential store unavailable ({e}); provider API keys stay in settings.json");
        }
        return config.clone();
    }

    let mut on_disk = config.clone();
    on_disk.provider_keys.clear();
    on_disk.cloud_api_key = None;
    on_disk
}

/// Fills in any key `config` does not already carry from `store`.
///
/// A key already present — one an older version wrote into the file — wins
/// and is moved into the store by the next save.
pub fn hydrate_keys(store: &dyn SecretStore, config: &mut ProviderConfig) {
    let stored = match store.read() {
        Ok(Some(json)) => match serde_json::from_str::<BTreeMap<String, String>>(&json) {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!("Stored provider keys could not be read ({e}); ignoring them");
                return;
            }
        },
        Ok(None) => return,
        Err(e) => {
            tracing::debug!("Credential store unavailable ({e}); using keys from settings.json only");
            return;
        }
    };

    for (slug, key) in stored {
        if slug == LEGACY_KEY {
            if config.cloud_api_key.as_deref().is_none_or(|k| k.trim().is_empty()) {
                config.cloud_api_key = Some(key);
            }
        } else if config.provider_keys.get(&slug).is_none_or(|k| k.trim().is_empty()) {
            config.provider_keys.insert(slug, key);
        }
    }
}

impl ProviderConfig {
    /// A copy safe to hand to the webview: every stored key replaced with
    /// [`STORED_KEY_PLACEHOLDER`].
    pub fn redacted(&self) -> ProviderConfig {
        let mut copy = self.clone();
        for key in copy.provider_keys.values_mut() {
            if !key.trim().is_empty() {
                *key = STORED_KEY_PLACEHOLDER.to_string();
            }
        }
        if copy.cloud_api_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
            copy.cloud_api_key = Some(STORED_KEY_PLACEHOLDER.to_string());
        }
        copy
    }

    /// Replaces placeholders the webview sent back with the keys they stand
    /// for, taken from `stored`. A placeholder with nothing behind it becomes
    /// empty rather than being saved as if it were a key.
    pub fn resolve_placeholders(&mut self, stored: &ProviderConfig) {
        for (slug, key) in self.provider_keys.iter_mut() {
            if key == STORED_KEY_PLACEHOLDER {
                *key = stored.provider_keys.get(slug).cloned().unwrap_or_default();
            }
        }
        if self.cloud_api_key.as_deref() == Some(STORED_KEY_PLACEHOLDER) {
            self.cloud_api_key = stored.cloud_api_key.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory store, for exercising the logic the real one sits behind.
    #[derive(Default)]
    struct MemoryStore {
        value: Mutex<Option<String>>,
        writes: Mutex<usize>,
    }

    impl SecretStore for MemoryStore {
        fn read(&self) -> Result<Option<String>, String> {
            Ok(self.value.lock().unwrap().clone())
        }
        fn write(&self, value: &str) -> Result<(), String> {
            *self.writes.lock().unwrap() += 1;
            *self.value.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            *self.value.lock().unwrap() = None;
            Ok(())
        }
    }

    fn config_with_keys() -> ProviderConfig {
        let mut config = ProviderConfig::default();
        config.provider_keys.insert("groq".into(), "gsk-live".into());
        config.provider_keys.insert("cloud_openai".into(), "sk-live".into());
        config
    }

    #[test]
    fn saved_keys_leave_the_file_and_come_back_on_load() {
        let store = MemoryStore::default();

        let on_disk = persist_keys(&store, &config_with_keys());
        assert!(on_disk.provider_keys.is_empty(), "no key is written to the file");
        assert_eq!(on_disk.cloud_api_key, None);

        let mut loaded = on_disk.clone();
        hydrate_keys(&store, &mut loaded);
        assert_eq!(loaded.api_key_for(&super::super::ProviderType::Groq), Some("gsk-live"));
        assert_eq!(
            loaded.api_key_for(&super::super::ProviderType::CloudOpenAI),
            Some("sk-live")
        );
    }

    #[test]
    fn a_legacy_plaintext_key_is_moved_on_the_next_save() {
        let store = MemoryStore::default();

        let mut from_old_file = ProviderConfig {
            cloud_api_key: Some("sk-old".into()),
            ..Default::default()
        };
        hydrate_keys(&store, &mut from_old_file);
        assert_eq!(from_old_file.cloud_api_key.as_deref(), Some("sk-old"));

        let on_disk = persist_keys(&store, &from_old_file);
        assert_eq!(on_disk.cloud_api_key, None);
        assert!(store.read().unwrap().unwrap().contains("sk-old"));
    }

    #[test]
    fn an_unchanged_save_does_not_rewrite_the_store() {
        let store = CachedStore::new(MemoryStore::default());
        let config = config_with_keys();

        persist_keys(&store, &config);
        persist_keys(&store, &config);
        persist_keys(&store, &config);
        assert_eq!(*store.inner.writes.lock().unwrap(), 1);

        // Reading what is there counts as knowing it: saving it back is free.
        let fresh = CachedStore::new(MemoryStore::default());
        persist_keys(&fresh.inner, &config);
        let mut loaded = ProviderConfig::default();
        hydrate_keys(&fresh, &mut loaded);
        persist_keys(&fresh, &loaded);
        assert_eq!(*fresh.inner.writes.lock().unwrap(), 1);
    }

    #[test]
    fn an_unavailable_store_keeps_keys_in_the_file() {
        let config = config_with_keys();
        let on_disk = persist_keys(&Unavailable, &config);
        assert_eq!(on_disk.provider_keys, config.provider_keys);
    }

    #[test]
    fn removing_every_key_clears_the_store() {
        let store = MemoryStore::default();
        persist_keys(&store, &config_with_keys());
        persist_keys(&store, &ProviderConfig::default());
        assert_eq!(store.read().unwrap(), None);
    }

    #[test]
    fn the_webview_never_sees_a_key_and_a_placeholder_round_trips() {
        let stored = config_with_keys();
        let shown = stored.redacted();
        assert_eq!(shown.provider_keys["groq"], STORED_KEY_PLACEHOLDER);
        assert!(!serde_json::to_string(&shown).unwrap().contains("gsk-live"));

        // Sent back untouched: the stored key is kept.
        let mut returned = shown.clone();
        returned.resolve_placeholders(&stored);
        assert_eq!(returned.provider_keys["groq"], "gsk-live");

        // Replaced by the user: the new key wins.
        let mut edited = shown;
        edited.provider_keys.insert("groq".into(), "gsk-new".into());
        edited.resolve_placeholders(&stored);
        assert_eq!(edited.provider_keys["groq"], "gsk-new");
        assert_eq!(edited.provider_keys["cloud_openai"], "sk-live");
    }

    #[test]
    fn a_placeholder_with_nothing_behind_it_is_not_saved_as_a_key() {
        let mut returned = ProviderConfig::default();
        returned
            .provider_keys
            .insert("groq".into(), STORED_KEY_PLACEHOLDER.into());
        returned.resolve_placeholders(&ProviderConfig::default());
        assert_eq!(returned.provider_keys["groq"], "");
        assert_eq!(returned.api_key_for(&super::super::ProviderType::Groq), None);
    }
}
