//! PARA classification for vault objects.
//!
//! PARA (Projects / Areas / Resources / Archive) is the one piece of
//! structure the Rings view and the TODO surface both read position from, so
//! it is stored explicitly rather than guessed. A scribble carries the band
//! it was filed under; anything derived from that scribble — a todo, a graph
//! node — inherits it. An object with no band is *uncategorised*, which is a
//! state the UI shows as its own region, never a band picked on its behalf.

use serde::{Deserialize, Deserializer, Serialize};

/// The four PARA bands, innermost (most active) first.
///
/// The declaration order is the ring order: Projects sits at the centre and
/// Archive at the rim, so `ALL` can drive both the layout and any band list
/// without a second ordering table to keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParaBand {
    /// Work with an end state: it finishes.
    Projects,
    /// Standing responsibilities: they continue.
    Areas,
    /// Reference material kept because it is useful later.
    Resources,
    /// Finished or dormant, kept for the record.
    Archive,
}

impl ParaBand {
    /// Every band, innermost first.
    pub const ALL: [ParaBand; 4] = [
        ParaBand::Projects,
        ParaBand::Areas,
        ParaBand::Resources,
        ParaBand::Archive,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Projects => "projects",
            Self::Areas => "areas",
            Self::Resources => "resources",
            Self::Archive => "archive",
        }
    }

    /// Parses a stored band, accepting the singular spellings a human is
    /// likely to type into frontmatter by hand.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "projects" | "project" | "p" => Some(Self::Projects),
            "areas" | "area" | "a" => Some(Self::Areas),
            "resources" | "resource" | "r" => Some(Self::Resources),
            "archive" | "archives" => Some(Self::Archive),
            _ => None,
        }
    }
}

/// Deserializes a PARA band, treating anything unrecognised as *absent*.
///
/// The strict derive would fail the whole frontmatter document on one
/// unknown string, and frontmatter failure means the scribble vanishes from
/// the vault listing entirely. A mis-typed band must cost its own value and
/// nothing else, so an unreadable band deserializes to `None` — the same
/// uncategorised state as a scribble that was never filed.
pub fn deserialize_band_lenient<'de, D>(deserializer: D) -> Result<Option<ParaBand>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match raw {
        Some(serde_json::Value::String(s)) => ParaBand::from_str_opt(&s),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Holder {
        #[serde(default, deserialize_with = "deserialize_band_lenient")]
        para: Option<ParaBand>,
    }

    #[test]
    fn rings_order_runs_projects_to_archive() {
        assert_eq!(
            ParaBand::ALL.map(|b| b.as_str()),
            ["projects", "areas", "resources", "archive"]
        );
    }

    #[test]
    fn parses_the_spellings_a_human_types() {
        assert_eq!(ParaBand::from_str_opt("Projects"), Some(ParaBand::Projects));
        assert_eq!(ParaBand::from_str_opt(" area "), Some(ParaBand::Areas));
        assert_eq!(
            ParaBand::from_str_opt("RESOURCE"),
            Some(ParaBand::Resources)
        );
        assert_eq!(ParaBand::from_str_opt("archives"), Some(ParaBand::Archive));
        assert_eq!(ParaBand::from_str_opt("someday"), None);
    }

    #[test]
    fn round_trips_through_json_as_its_stored_spelling() {
        let holder = Holder {
            para: Some(ParaBand::Resources),
        };
        let json = serde_json::to_string(&holder).expect("serializes");
        assert_eq!(json, r#"{"para":"resources"}"#);
        assert_eq!(
            serde_json::from_str::<Holder>(&json).expect("parses"),
            holder
        );
    }

    /// The reason the lenient deserializer exists: a junk band must not take
    /// the surrounding document down with it.
    #[test]
    fn an_unknown_band_reads_as_uncategorised_rather_than_failing() {
        let parsed: Holder = serde_json::from_str(r#"{"para":"someday-maybe"}"#)
            .expect("an unknown band must not fail the document");
        assert_eq!(parsed.para, None);

        let wrong_type: Holder =
            serde_json::from_str(r#"{"para":7}"#).expect("a wrong-typed band must not fail either");
        assert_eq!(wrong_type.para, None);

        let missing: Holder = serde_json::from_str("{}").expect("an absent band is uncategorised");
        assert_eq!(missing.para, None);
    }
}
