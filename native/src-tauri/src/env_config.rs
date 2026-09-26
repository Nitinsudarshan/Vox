//! Configuration read from the environment, under its `VOX_` name first and
//! its pre-rename `RELAY_` name second.
//!
//! The rename left three spellings of the Google client id in circulation —
//! `RELAY_` in the code, `GOOGLE_` in `.env.example`, `Vox_` in the setup
//! guide — so whichever one a developer followed, sign-in silently failed.
//! `VOX_` is now canonical everywhere and `RELAY_` keeps an existing machine
//! working.

/// A setting fixed at build time, e.g. `build_time!("GOOGLE_CLIENT_ID")`.
macro_rules! build_time {
    ($name:literal) => {
        option_env!(concat!("VOX_", $name))
            .or(option_env!(concat!("RELAY_", $name)))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
}
pub(crate) use build_time;

/// A setting read when the app runs: `VOX_<name>`, then `RELAY_<name>`.
pub fn runtime(name: &str) -> Option<String> {
    runtime_from(name, |key| std::env::var(key).ok())
}

/// [`runtime`], with the environment passed in so it can be tested without
/// mutating the process's own.
fn runtime_from(name: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["VOX_", "RELAY_"]
        .iter()
        .filter_map(|prefix| lookup(&format!("{prefix}{name}")))
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::runtime_from;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn the_vox_name_wins_over_the_legacy_one() {
        let lookup = env(&[("VOX_THING", "new"), ("RELAY_THING", "old")]);
        assert_eq!(runtime_from("THING", lookup).as_deref(), Some("new"));
    }

    #[test]
    fn the_legacy_name_still_works() {
        let lookup = env(&[("RELAY_THING", " old ")]);
        assert_eq!(runtime_from("THING", lookup).as_deref(), Some("old"));
    }

    #[test]
    fn blank_values_count_as_unset() {
        let lookup = env(&[("VOX_THING", "  "), ("RELAY_THING", "old")]);
        assert_eq!(runtime_from("THING", lookup).as_deref(), Some("old"));
        assert_eq!(runtime_from("OTHER", env(&[])), None);
    }
}
