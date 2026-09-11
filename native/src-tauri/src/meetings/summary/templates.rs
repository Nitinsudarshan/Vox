//! Report shapes, as data.
//!
//! A template says what sections a meeting report has and what each section is
//! for. Six ship with Vox, compiled into the binary; a JSON file dropped in
//! `<config>/meeting-templates/<id>.json` replaces one of them or adds a new
//! one, with no rebuild.
//!
//! This is meetily's one genuinely pluggable extension point and it is worth
//! copying — with its path-traversal bug fixed. Meetily joins a caller-supplied
//! `template_id` straight onto a directory path (`templates/loader.rs:41`), so
//! `../../…` reads any `.json` the user can. Ids here are validated before
//! they touch the filesystem.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Directory, under the app config dir, that user templates are read from.
pub const USER_TEMPLATE_DIR: &str = "meeting-templates";

/// The template used when nothing else is chosen.
pub const DEFAULT_TEMPLATE_ID: &str = "general";

/// Templates compiled into the binary.
///
/// Embedded rather than resolved from a Tauri resource directory so that a
/// broken install degrades to "no custom templates", never to "no templates".
const BUNDLED: &[(&str, &str)] = &[
    ("general", include_str!("../../../templates/meetings/general.json")),
    ("standup", include_str!("../../../templates/meetings/standup.json")),
    ("one-on-one", include_str!("../../../templates/meetings/one-on-one.json")),
    ("client-call", include_str!("../../../templates/meetings/client-call.json")),
    ("interview", include_str!("../../../templates/meetings/interview.json")),
    (
        "technical-review",
        include_str!("../../../templates/meetings/technical-review.json"),
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("no summary template with id {0}")]
    NotFound(String),

    #[error("template {id} is not valid: {reason}")]
    Invalid { id: String, reason: String },

    #[error("template id {0} is not a valid identifier")]
    InvalidId(String),
}

/// How a section's content should be laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionStyle {
    #[default]
    Bullets,
    /// `- [ ]` items, for things someone has to do.
    Checklist,
    /// Flowing paragraphs, for a summary or a narrative.
    Prose,
}

impl SectionStyle {
    /// The instruction appended to a section's own, telling the model what
    /// shape to write in.
    fn format_rule(self) -> &'static str {
        match self {
            SectionStyle::Bullets => "Write this section as `- ` bullet points, one point per line.",
            SectionStyle::Checklist => {
                "Write this section as GitHub-style checklist items, each line starting with \
                 `- [ ] `, and put the owner in bold at the end as `— **Name**`."
            }
            SectionStyle::Prose => {
                "Write this section as plain paragraphs. No bullet points, no headings."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateSection {
    pub heading: String,
    /// What belongs in this section, in the author's words. Goes into the
    /// prompt verbatim.
    pub instruction: String,
    #[serde(default)]
    pub style: SectionStyle,
    /// A required section is always written, with an explicit "None" when the
    /// transcript has nothing for it; an optional one is omitted instead.
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub sections: Vec<TemplateSection>,
    /// Whether this template came from the user's own directory.
    #[serde(default, skip_deserializing)]
    pub custom: bool,
}

impl Template {
    /// Checks the invariants the prompt builder relies on.
    pub fn validate(&self) -> Result<(), TemplateError> {
        if !is_valid_template_id(&self.id) {
            return Err(TemplateError::InvalidId(self.id.clone()));
        }
        if self.name.trim().is_empty() {
            return Err(TemplateError::Invalid {
                id: self.id.clone(),
                reason: "name is empty".into(),
            });
        }
        if self.sections.is_empty() {
            return Err(TemplateError::Invalid {
                id: self.id.clone(),
                reason: "a template needs at least one section".into(),
            });
        }
        for section in &self.sections {
            if section.heading.trim().is_empty() {
                return Err(TemplateError::Invalid {
                    id: self.id.clone(),
                    reason: "a section has an empty heading".into(),
                });
            }
            if section.instruction.trim().is_empty() {
                return Err(TemplateError::Invalid {
                    id: self.id.clone(),
                    reason: format!("section '{}' has no instruction", section.heading),
                });
            }
        }
        Ok(())
    }

    /// The markdown skeleton the model is asked to fill.
    pub fn skeleton(&self) -> String {
        let mut out = String::from("# <a short, specific title for this meeting>\n");
        for section in &self.sections {
            out.push_str(&format!("\n## {}\n<content>\n", section.heading));
        }
        out
    }

    /// Per-section instructions, numbered, for the system prompt.
    pub fn section_instructions(&self) -> String {
        let mut out = String::new();
        for (index, section) in self.sections.iter().enumerate() {
            out.push_str(&format!(
                "{}. ## {}\n   {}\n   {}\n",
                index + 1,
                section.heading,
                section.instruction.trim(),
                section.style.format_rule()
            ));
            if !section.required {
                out.push_str(
                    "   If the transcript contains nothing for this section, omit the heading \
                     entirely.\n",
                );
            } else {
                out.push_str(
                    "   If the transcript contains nothing for this section, keep the heading and \
                     write `None recorded.`\n",
                );
            }
        }
        out
    }

    /// A stable string that changes whenever the template's content changes.
    ///
    /// Part of a summary's cache fingerprint: editing a template must
    /// invalidate the report it produced.
    pub fn content_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.id.as_bytes());
        for section in &self.sections {
            hasher.update(section.heading.as_bytes());
            hasher.update(section.instruction.as_bytes());
            hasher.update(format!("{:?}{}", section.style, section.required).as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}

/// Reads templates from the binary and from the user's template directory.
#[derive(Debug, Clone)]
pub struct TemplateLibrary {
    user_dir: Option<PathBuf>,
}

impl TemplateLibrary {
    /// A library that also reads `<config_dir>/meeting-templates`.
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        Self {
            user_dir: Some(config_dir.as_ref().join(USER_TEMPLATE_DIR)),
        }
    }

    /// A library with only the bundled templates.
    pub fn bundled_only() -> Self {
        Self { user_dir: None }
    }

    /// Every available template, user overrides applied, sorted by name.
    ///
    /// A user file whose id matches a bundled one replaces it; an unparseable
    /// one is skipped with a warning rather than hiding the whole library.
    pub fn list(&self) -> Vec<Template> {
        let mut by_id: BTreeMap<String, Template> = BTreeMap::new();
        for (id, json) in BUNDLED {
            match serde_json::from_str::<Template>(json) {
                Ok(template) => {
                    by_id.insert((*id).to_string(), template);
                }
                Err(err) => {
                    // A bundled template that does not parse is a build-time
                    // mistake; say so loudly and keep the others.
                    tracing::error!("bundled meeting template {} is malformed: {}", id, err);
                }
            }
        }
        for template in self.read_user_templates() {
            by_id.insert(template.id.clone(), template);
        }
        let mut templates: Vec<Template> = by_id.into_values().collect();
        templates.sort_by(|a, b| a.name.cmp(&b.name));
        templates
    }

    /// One template by id.
    pub fn get(&self, id: &str) -> Result<Template, TemplateError> {
        if !is_valid_template_id(id) {
            return Err(TemplateError::InvalidId(id.to_string()));
        }
        self.list()
            .into_iter()
            .find(|template| template.id == id)
            .ok_or_else(|| TemplateError::NotFound(id.to_string()))
    }

    /// The requested template, or the default when it no longer exists.
    ///
    /// A meeting can outlive the template it was summarised with — the user
    /// deleted their custom one — and that must not make the meeting
    /// un-regeneratable.
    pub fn get_or_default(&self, id: Option<&str>) -> Template {
        let requested = id.unwrap_or(DEFAULT_TEMPLATE_ID);
        match self.get(requested) {
            Ok(template) => template,
            Err(err) => {
                tracing::warn!("meeting template {}: {} — using the default", requested, err);
                self.get(DEFAULT_TEMPLATE_ID)
                    .unwrap_or_else(|_| fallback_template())
            }
        }
    }

    /// Where user templates are read from, if anywhere.
    pub fn user_dir(&self) -> Option<&Path> {
        self.user_dir.as_deref()
    }

    fn read_user_templates(&self) -> Vec<Template> {
        let Some(dir) = &self.user_dir else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut templates = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match serde_json::from_str::<Template>(&text) {
                Ok(mut template) => {
                    template.custom = true;
                    match template.validate() {
                        Ok(()) => templates.push(template),
                        Err(err) => {
                            tracing::warn!("ignoring meeting template {:?}: {}", path, err);
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!("ignoring unparseable meeting template {:?}: {}", path, err);
                }
            }
        }
        templates
    }
}

/// Whether an id is safe to compare and safe as a filename.
///
/// The check meetily's loader is missing.
pub fn is_valid_template_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The last-resort template, used only if the bundled JSON fails to parse.
///
/// Exists so that summarization has no unreachable state: there is always
/// something to fill.
fn fallback_template() -> Template {
    Template {
        id: DEFAULT_TEMPLATE_ID.to_string(),
        name: "General Meeting".to_string(),
        description: "Summary, decisions and action items.".to_string(),
        sections: vec![
            TemplateSection {
                heading: "Summary".into(),
                instruction: "What this meeting was about and what came out of it.".into(),
                style: SectionStyle::Prose,
                required: true,
            },
            TemplateSection {
                heading: "Decisions".into(),
                instruction: "Decisions actually reached.".into(),
                style: SectionStyle::Bullets,
                required: true,
            },
            TemplateSection {
                heading: "Action Items".into(),
                instruction: "Commitments made, with owners.".into(),
                style: SectionStyle::Checklist,
                required: true,
            },
        ],
        custom: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_template_parses_and_is_valid() {
        for (id, json) in BUNDLED {
            let template: Template = serde_json::from_str(json)
                .unwrap_or_else(|err| panic!("bundled template {id} does not parse: {err}"));
            assert_eq!(&template.id, id, "template id must match its filename");
            template
                .validate()
                .unwrap_or_else(|err| panic!("bundled template {id} is invalid: {err}"));
        }
    }

    #[test]
    fn the_bundled_library_offers_six_templates_including_the_default() {
        let library = TemplateLibrary::bundled_only();
        let templates = library.list();
        assert_eq!(templates.len(), 6);
        assert!(library.get(DEFAULT_TEMPLATE_ID).is_ok());
        assert!(templates.iter().all(|t| !t.custom));
    }

    #[test]
    fn a_template_id_that_climbs_out_of_the_directory_is_refused() {
        let library = TemplateLibrary::bundled_only();
        for bad in ["../../etc/passwd", "..", "a/b", "General", "gen eral", ""] {
            assert!(
                matches!(library.get(bad), Err(TemplateError::InvalidId(_))),
                "{bad:?} should be refused as an id"
            );
        }
    }

    #[test]
    fn an_unknown_but_well_formed_id_reads_as_missing_rather_than_unsafe() {
        let library = TemplateLibrary::bundled_only();
        assert!(matches!(
            library.get("no-such-template"),
            Err(TemplateError::NotFound(_))
        ));
    }

    #[test]
    fn a_missing_template_falls_back_to_the_default_instead_of_failing() {
        let library = TemplateLibrary::bundled_only();
        let template = library.get_or_default(Some("deleted-by-the-user"));
        assert_eq!(template.id, DEFAULT_TEMPLATE_ID);
        assert_eq!(library.get_or_default(None).id, DEFAULT_TEMPLATE_ID);
    }

    #[test]
    fn a_user_template_replaces_the_bundled_one_with_the_same_id() {
        let dir = std::env::temp_dir().join(format!(
            "vox-templates-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(USER_TEMPLATE_DIR)).unwrap();
        std::fs::write(
            dir.join(USER_TEMPLATE_DIR).join("general.json"),
            r#"{"id":"general","name":"My General","sections":[
                {"heading":"Only Section","instruction":"Everything."}]}"#,
        )
        .unwrap();

        let library = TemplateLibrary::new(&dir);
        let template = library.get("general").expect("override");

        assert_eq!(template.name, "My General");
        assert_eq!(template.sections.len(), 1);
        assert!(template.custom);
        assert_eq!(library.list().len(), 6, "an override replaces, not adds");
    }

    #[test]
    fn a_new_user_template_is_added_alongside_the_bundled_ones() {
        let dir = std::env::temp_dir().join(format!(
            "vox-templates-new-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(USER_TEMPLATE_DIR)).unwrap();
        std::fs::write(
            dir.join(USER_TEMPLATE_DIR).join("board.json"),
            r#"{"id":"board","name":"Board Meeting","sections":[
                {"heading":"Motions","instruction":"Motions and their outcome."}]}"#,
        )
        .unwrap();

        assert_eq!(TemplateLibrary::new(&dir).list().len(), 7);
    }

    #[test]
    fn an_invalid_user_template_is_skipped_and_does_not_hide_the_library() {
        let dir = std::env::temp_dir().join(format!(
            "vox-templates-bad-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(USER_TEMPLATE_DIR)).unwrap();
        std::fs::write(dir.join(USER_TEMPLATE_DIR).join("broken.json"), "{ nope").unwrap();
        std::fs::write(
            dir.join(USER_TEMPLATE_DIR).join("empty.json"),
            r#"{"id":"empty","name":"Empty","sections":[]}"#,
        )
        .unwrap();

        let library = TemplateLibrary::new(&dir);
        assert_eq!(library.list().len(), 6);
        assert!(library.get("empty").is_err());
    }

    #[test]
    fn a_missing_user_directory_is_not_an_error() {
        let library = TemplateLibrary::new("/definitely/not/a/real/config/dir");
        assert_eq!(library.list().len(), 6);
    }

    #[test]
    fn the_skeleton_lists_every_section_under_a_title() {
        let template = TemplateLibrary::bundled_only().get("standup").unwrap();
        let skeleton = template.skeleton();
        assert!(skeleton.starts_with("# "));
        for section in &template.sections {
            assert!(
                skeleton.contains(&format!("## {}", section.heading)),
                "skeleton is missing {}",
                section.heading
            );
        }
    }

    #[test]
    fn section_instructions_carry_the_style_and_the_empty_case() {
        let template = TemplateLibrary::bundled_only().get("general").unwrap();
        let instructions = template.section_instructions();
        assert!(instructions.contains("- [ ] "), "checklist rule is missing");
        assert!(instructions.contains("None recorded."), "required-empty rule is missing");
        assert!(
            instructions.contains("omit the heading entirely"),
            "optional-section rule is missing"
        );
    }

    #[test]
    fn editing_a_template_changes_its_hash() {
        let mut template = TemplateLibrary::bundled_only().get("general").unwrap();
        let before = template.content_hash();
        assert_eq!(before, template.content_hash(), "hashing must be stable");
        template.sections[0].instruction.push_str(" Be brief.");
        assert_ne!(before, template.content_hash());
    }

    #[test]
    fn a_template_missing_a_section_instruction_is_rejected() {
        let template: Template = serde_json::from_str(
            r#"{"id":"x","name":"X","sections":[{"heading":"H","instruction":"  "}]}"#,
        )
        .unwrap();
        assert!(matches!(template.validate(), Err(TemplateError::Invalid { .. })));
    }
}
