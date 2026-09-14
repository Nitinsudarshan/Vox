//! Where connected accounts and their cached events live.
//!
//! Two separate things, on purpose:
//!
//! - **Tokens** go to the OS keyring, one entry per account address, through
//!   the existing [`crate::oauth::KeyringTokenStore`]. They never touch the
//!   vault and never appear in a settings file.
//! - **Accounts and events** are plain JSON in the vault. An event is a copy
//!   of something Google already has; losing the cache costs one sync.
//!
//! The cache exists so the agenda renders offline and instantly. A calendar
//! that shows nothing until a network round trip finishes is a calendar people
//! stop opening.

use std::fs;
use std::path::{Path, PathBuf};

use crate::oauth::{KeyringTokenStore, OAuthTokens};

use super::model::{CalendarAccount, CalendarEvent};

const ACCOUNTS_FILE: &str = "calendar-accounts.json";
const EVENTS_FILE: &str = "calendar-events.json";

/// Keyring service for calendar tokens. One entry per account address, so
/// disconnecting one account cannot take another's tokens with it.
const TOKEN_SERVICE: &str = "com.vox.app.calendar";

#[derive(Debug, thiserror::Error)]
pub enum CalendarStoreError {
    #[error("calendar storage: {0}")]
    Io(#[from] std::io::Error),

    #[error("calendar storage: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Tokens(String),
}

/// Reads and writes the calendar's own corner of the vault.
pub struct CalendarStore {
    dir: PathBuf,
    config_dir: PathBuf,
}

impl CalendarStore {
    pub fn new(dir: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            config_dir: config_dir.into(),
        }
    }

    fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    /// Every connected account, in the order they were added.
    ///
    /// A file that cannot be read means no accounts rather than an error: the
    /// rest of Vox works without a calendar, and failing to open Meetings
    /// because a cache file is corrupt would be the wrong trade.
    pub fn load_accounts(&self) -> Vec<CalendarAccount> {
        let path = self.path(ACCOUNTS_FILE);
        if !path.exists() {
            return Vec::new();
        }
        fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_accounts(&self, accounts: &[CalendarAccount]) -> Result<(), CalendarStoreError> {
        fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(ACCOUNTS_FILE), &serde_json::to_vec_pretty(accounts)?)?;
        Ok(())
    }

    /// Adds an account, or updates the one already signed in as that address.
    ///
    /// Matched on the address rather than appended, so signing in again to fix
    /// an expired grant repairs the account instead of listing it twice.
    pub fn upsert_account(&self, account: CalendarAccount) -> Result<Vec<CalendarAccount>, CalendarStoreError> {
        let mut accounts = self.load_accounts();
        match accounts
            .iter_mut()
            .find(|existing| existing.email.eq_ignore_ascii_case(&account.email))
        {
            Some(existing) => {
                // Keep what is the user's: which calendars they picked, and
                // whether they had this account switched off.
                let enabled = existing.enabled;
                let calendar_ids = existing.calendar_ids.clone();
                *existing = account;
                existing.enabled = enabled;
                if existing.calendar_ids.is_empty() {
                    existing.calendar_ids = calendar_ids;
                }
            }
            None => accounts.push(account),
        }
        self.save_accounts(&accounts)?;
        Ok(accounts)
    }

    /// Disconnects an account: its tokens, and its cached events with them.
    ///
    /// Removing the events is the point. Leaving a disconnected work
    /// calendar's meetings in the agenda is exactly the surprise a person
    /// disconnecting a work account is trying to avoid.
    pub fn remove_account(&self, email: &str) -> Result<Vec<CalendarAccount>, CalendarStoreError> {
        let mut accounts = self.load_accounts();
        accounts.retain(|account| !account.email.eq_ignore_ascii_case(email));
        self.save_accounts(&accounts)?;

        let mut events = self.load_events();
        events.retain(|event| !event.account_email.eq_ignore_ascii_case(email));
        self.save_events(&events)?;

        let _ = KeyringTokenStore::delete_explicit(
            &self.config_dir,
            TOKEN_SERVICE,
            &token_username(email),
            &token_fallback(email),
        );
        Ok(accounts)
    }

    pub fn load_tokens(&self, email: &str) -> Option<OAuthTokens> {
        KeyringTokenStore::load_explicit(
            &self.config_dir,
            TOKEN_SERVICE,
            &token_username(email),
            &token_fallback(email),
        )
    }

    pub fn save_tokens(&self, email: &str, tokens: &OAuthTokens) -> Result<(), CalendarStoreError> {
        KeyringTokenStore::save_explicit(
            &self.config_dir,
            TOKEN_SERVICE,
            &token_username(email),
            &token_fallback(email),
            tokens,
        )
        .map_err(CalendarStoreError::Tokens)
    }

    /// Every cached event, across every account.
    pub fn load_events(&self) -> Vec<CalendarEvent> {
        let path = self.path(EVENTS_FILE);
        if !path.exists() {
            return Vec::new();
        }
        fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_events(&self, events: &[CalendarEvent]) -> Result<(), CalendarStoreError> {
        fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(EVENTS_FILE), &serde_json::to_vec_pretty(events)?)?;
        Ok(())
    }

    /// Replaces one account's events, leaving every other account's alone.
    ///
    /// Per-account replacement rather than a global rewrite: accounts sync one
    /// at a time and one of them failing must not empty the agenda.
    pub fn replace_account_events(
        &self,
        email: &str,
        events: Vec<CalendarEvent>,
    ) -> Result<Vec<CalendarEvent>, CalendarStoreError> {
        let mut all = self.load_events();
        all.retain(|event| !event.account_email.eq_ignore_ascii_case(email));
        all.extend(events);
        self.save_events(&all)?;
        Ok(all)
    }
}

/// The keyring username for one account's tokens.
fn token_username(email: &str) -> String {
    format!("google_calendar_tokens::{}", email.to_lowercase())
}

/// The encrypted-file fallback name, for a machine with no usable keyring.
///
/// The address is reduced to characters that are safe in a filename on every
/// platform, which also keeps two addresses from colliding on one file.
fn token_fallback(email: &str) -> String {
    let slug: String = email
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("calendar_tokens_{slug}.bin")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::model::Attendance;

    fn store() -> (CalendarStore, tempdir::TempDir) {
        let dir = tempdir::TempDir::new();
        let store = CalendarStore::new(dir.path().join("calendar"), dir.path().join("config"));
        (store, dir)
    }

    fn account(email: &str) -> CalendarAccount {
        CalendarAccount {
            email: email.to_string(),
            display_name: Some("Nitin".into()),
            enabled: true,
            calendar_ids: Vec::new(),
            last_synced_at: None,
            last_error: None,
        }
    }

    fn event(id: &str, account: &str) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            account_email: account.to_string(),
            calendar_id: Some("primary".into()),
            title: "Sync".into(),
            start: "2026-09-14T10:00:00Z".into(),
            end: "2026-09-14T11:00:00Z".into(),
            all_day: false,
            location: None,
            conference_url: None,
            html_link: None,
            description: None,
            attendees: Vec::new(),
            attendance: Attendance::Unknown,
            meeting_id: None,
        }
    }

    #[test]
    fn a_vault_with_no_calendar_reads_as_no_accounts_rather_than_an_error() {
        let (store, _dir) = store();
        assert!(store.load_accounts().is_empty());
        assert!(store.load_events().is_empty());
    }

    #[test]
    fn accounts_round_trip() {
        let (store, _dir) = store();
        store
            .upsert_account(account("me@work.com"))
            .expect("save an account");
        store
            .upsert_account(account("me@gmail.com"))
            .expect("save an account");

        let accounts = store.load_accounts();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].email, "me@work.com");
    }

    #[test]
    fn signing_in_again_repairs_an_account_rather_than_listing_it_twice() {
        let (store, _dir) = store();
        store.upsert_account(account("me@work.com")).expect("save");

        let mut again = account("ME@WORK.COM");
        again.display_name = Some("Nitin S".into());
        let accounts = store.upsert_account(again).expect("save");

        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].display_name.as_deref(), Some("Nitin S"));
    }

    #[test]
    fn signing_in_again_keeps_the_choices_the_user_made() {
        let (store, _dir) = store();
        let mut existing = account("me@work.com");
        existing.enabled = false;
        existing.calendar_ids = vec!["team-cal".into()];
        store.upsert_account(existing).expect("save");

        let accounts = store.upsert_account(account("me@work.com")).expect("save");
        assert!(!accounts[0].enabled, "a switched-off account stays off");
        assert_eq!(accounts[0].calendar_ids, vec!["team-cal".to_string()]);
    }

    #[test]
    fn disconnecting_an_account_takes_its_events_with_it() {
        // Leaving a disconnected work calendar's meetings on screen is exactly
        // what someone disconnecting a work account is trying to prevent.
        let (store, _dir) = store();
        store.upsert_account(account("me@work.com")).expect("save");
        store.upsert_account(account("me@gmail.com")).expect("save");
        store
            .save_events(&[event("a", "me@work.com"), event("b", "me@gmail.com")])
            .expect("save events");

        let accounts = store.remove_account("me@work.com").expect("remove");
        assert_eq!(accounts.len(), 1);

        let events = store.load_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].account_email, "me@gmail.com");
    }

    #[test]
    fn one_account_syncing_never_empties_another_accounts_agenda() {
        let (store, _dir) = store();
        store
            .save_events(&[event("a", "me@work.com"), event("b", "me@gmail.com")])
            .expect("save events");

        let all = store
            .replace_account_events("me@work.com", vec![event("a2", "me@work.com")])
            .expect("replace");

        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|e| e.id == "b"), "the other account survived");
        assert!(all.iter().any(|e| e.id == "a2"));
        assert!(!all.iter().any(|e| e.id == "a"), "the stale copy is gone");
    }

    #[test]
    fn two_accounts_never_share_a_token_entry() {
        // A collision here would let disconnecting one account sign the other
        // one out, or worse, read its calendar.
        assert_ne!(
            token_username("me@work.com"),
            token_username("me@gmail.com")
        );
        assert_ne!(
            token_fallback("me@work.com"),
            token_fallback("me@gmail.com")
        );
        assert_eq!(token_username("ME@Work.com"), token_username("me@work.com"));
    }

    #[test]
    fn the_token_fallback_filename_is_safe_on_every_platform() {
        let name = token_fallback("first.last+tag@example.co.uk");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
            "got {name}"
        );
        assert!(name.ends_with(".bin"));
    }
}

/// A throwaway directory for tests.
#[cfg(test)]
mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vox-calendar-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).expect("a temp directory");
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
