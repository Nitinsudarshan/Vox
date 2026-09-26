//! Kanban cards — the todo model, and how one is written to the vault.
//!
//! Vox captures action items in several places and has never had anywhere
//! to put them: `KanbanCard` and its `kanban/` directory existed with
//! working save and list, and nothing but tests ever wrote one. This is
//! that model grown up enough to be the TODOs surface's store — the same
//! struct and the same directory, with the three things it was missing.
//!
//! - **Where a todo came from** (`source_kind` + `source_ref`), so every
//!   card can be traced back to the meeting turn, voice note or scribble
//!   that produced it.
//! - **Which PARA band it inherits**, never asked of the user. A todo is
//!   filed wherever its source note is filed; a todo from an unfiled source
//!   is uncategorised, which is shown as its own band rather than guessed.
//! - **When it was captured**, as distinct from when the record was
//!   written. Extracting yesterday's meeting today makes those different
//!   dates, and ordering by the wrong one puts stale work at the top.
//!
//! ## On-disk format
//!
//! Quoted `key: "value"` lines between `---` fences, which is what the
//! existing writer produced. New keys are appended and every one of them is
//! optional on read, so a card written before this module still loads with
//! its original fields intact — see `an_existing_card_still_loads`.

use serde::{Deserialize, Serialize};

use super::para::ParaBand;

/// Where a todo came from.
///
/// Not `Default`-able on purpose: a card whose provenance was never
/// recorded is `None`, and the UI says "source unknown" rather than
/// claiming it was typed by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoSourceKind {
    Meeting,
    VoiceNote,
    Scribble,
    Manual,
    Talkback,
    /// A page or AI conversation captured by the browser extension.
    ///
    /// Not in the original five: the brief expected `capture/web/context.rs`
    /// to be the meetings extractor, and it is not — it serves browser
    /// captures, and meetings have no structured action-item extraction at
    /// all. Labelling these as meetings would have been the one thing worse
    /// than an extra variant.
    WebCapture,
}

impl TodoSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Meeting => "meeting",
            Self::VoiceNote => "voice_note",
            Self::Scribble => "scribble",
            Self::Manual => "manual",
            Self::Talkback => "talkback",
            Self::WebCapture => "web_capture",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "meeting" => Some(Self::Meeting),
            "voice_note" | "voicenote" => Some(Self::VoiceNote),
            "scribble" => Some(Self::Scribble),
            "manual" => Some(Self::Manual),
            "talkback" => Some(Self::Talkback),
            "web_capture" | "webcapture" | "capture" => Some(Self::WebCapture),
            _ => None,
        }
    }
}

/// Enough to navigate back to exactly where a todo came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TodoSourceRef {
    /// The originating object: meeting id, voice note id, or scribble id.
    pub id: String,
    /// Which turn of a transcript, where the source has turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ordinal: Option<u32>,
    /// What to call the origin in the UI, so showing provenance does not
    /// require a second lookup per card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One action item an extractor found, before it becomes a card.
///
/// A named struct rather than a tuple because every one of these fields is
/// optional-looking at the call site and three of them are `Option`s —
/// positional arguments there are a swap waiting to happen.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedTodo {
    pub title: String,
    pub kind: TodoSourceKind,
    pub source_ref: TodoSourceRef,
    /// Inherited from the source, where the source has a band.
    pub para: Option<ParaBand>,
    /// When the originating moment happened, if known.
    pub captured_at: Option<String>,
}

pub const STATUS_TODO: &str = "todo";
pub const STATUS_IN_PROGRESS: &str = "in_progress";
pub const STATUS_DONE: &str = "done";

/// The three columns, in board order.
pub const KANBAN_STATUSES: [&str; 3] = [STATUS_TODO, STATUS_IN_PROGRESS, STATUS_DONE];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KanbanCard {
    pub id: String,
    pub title: String,
    pub assignee: String,
    pub status: String, // "todo", "in_progress", "done"
    pub priority: String,
    pub due_date: Option<String>,
    pub created_at: String,
    pub description: String,
    pub source_note_id: Option<String>,

    /// What kind of capture produced this todo. `None` for a card written
    /// before provenance was recorded.
    #[serde(default)]
    pub source_kind: Option<TodoSourceKind>,

    /// Where to go to see the todo in its original context.
    #[serde(default)]
    pub source_ref: Option<TodoSourceRef>,

    /// Inherited from the source note. Never set by hand — see the module
    /// comment.
    #[serde(default)]
    pub para: Option<ParaBand>,

    /// When the originating moment happened, where that differs from when
    /// the card was written.
    #[serde(default)]
    pub captured_at: Option<String>,
}

impl KanbanCard {
    /// A todo typed on the TODOs page.
    ///
    /// Manual capture has no upstream note, so it inherits no band and
    /// lands in Uncategorised until its source does — which for a typed
    /// todo is never. That is the honest outcome of "PARA is inherited,
    /// never asked for".
    pub fn new_manual(title: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: format!("card_{}", uuid::Uuid::new_v4()),
            title: title.trim().to_string(),
            assignee: String::new(),
            status: STATUS_TODO.to_string(),
            priority: "medium".to_string(),
            due_date: None,
            created_at: now.clone(),
            description: String::new(),
            source_note_id: None,
            source_kind: Some(TodoSourceKind::Manual),
            source_ref: None,
            para: None,
            captured_at: Some(now),
        }
    }

    /// A todo extracted from something already in the vault.
    ///
    /// `para` is the band of the source note, passed in by the caller that
    /// knows it rather than looked up here — the extraction paths already
    /// hold the note they are reading.
    pub fn from_source(
        title: &str,
        kind: TodoSourceKind,
        source_ref: TodoSourceRef,
        para: Option<ParaBand>,
        captured_at: Option<String>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: format!("card_{}", uuid::Uuid::new_v4()),
            title: title.trim().to_string(),
            assignee: String::new(),
            status: STATUS_TODO.to_string(),
            priority: "medium".to_string(),
            due_date: None,
            created_at: now.clone(),
            description: String::new(),
            source_note_id: Some(source_ref.id.clone()),
            source_kind: Some(kind),
            source_ref: Some(source_ref),
            para,
            captured_at: Some(captured_at.unwrap_or(now)),
        }
    }

    /// When this todo arrived, for date ordering.
    ///
    /// Falls back to `created_at` so a card written before `captured_at`
    /// existed still sorts sensibly rather than sinking to the bottom.
    pub fn captured_or_created(&self) -> &str {
        self.captured_at.as_deref().unwrap_or(&self.created_at)
    }

    /// The card as it is stored.
    ///
    /// Every field added after the original writer is emitted only when it
    /// has a value, so a manual todo's file stays as small as the ones the
    /// old code wrote and a diff of the vault shows what actually changed.
    pub fn format_markdown(&self) -> String {
        let mut frontmatter = format!(
            "id: \"{}\"\ntitle: \"{}\"\nassignee: \"{}\"\nstatus: \"{}\"\npriority: \"{}\"\ndue_date: \"{}\"\ncreated_at: \"{}\"\nsource_note_id: \"{}\"",
            escape(&self.id),
            escape(&self.title),
            escape(&self.assignee),
            escape(&self.status),
            escape(&self.priority),
            escape(self.due_date.as_deref().unwrap_or("")),
            escape(&self.created_at),
            escape(self.source_note_id.as_deref().unwrap_or("")),
        );

        if let Some(kind) = self.source_kind {
            frontmatter.push_str(&format!("\nsource_kind: \"{}\"", kind.as_str()));
        }
        if let Some(band) = self.para {
            frontmatter.push_str(&format!("\npara: \"{}\"", band.as_str()));
        }
        if let Some(captured) = &self.captured_at {
            frontmatter.push_str(&format!("\ncaptured_at: \"{}\"", escape(captured)));
        }
        if let Some(source_ref) = &self.source_ref {
            // Compact JSON on one line: `serde_json` escapes newlines and
            // quotes inside strings, so a reference cannot break out of its
            // line and corrupt the key below it.
            if let Ok(encoded) = serde_json::to_string(source_ref) {
                frontmatter.push_str(&format!("\nsource_ref: {}", encoded));
            }
        }

        format!("---\n{}\n---\n\n{}", frontmatter, self.description)
    }

    /// Reads a card back.
    ///
    /// Every field introduced after the first version is optional, which is
    /// what keeps a vault written by the previous code readable.
    pub fn parse_markdown(content: &str) -> Option<Self> {
        let (frontmatter, body) = super::frontmatter::split(content)?;
        let description = body.trim().to_string();

        let mut id = String::new();
        let mut title = String::new();
        let mut assignee = String::new();
        let mut status = STATUS_TODO.to_string();
        let mut priority = "medium".to_string();
        let mut due_date = None;
        let mut created_at = String::new();
        let mut source_note_id = None;
        let mut source_kind = None;
        let mut source_ref = None;
        let mut para = None;
        let mut captured_at = None;

        for line in frontmatter.lines() {
            let line = line.trim();
            let Some((key, raw)) = line.split_once(':') else {
                continue;
            };
            let value = unescape(raw.trim().trim_matches('"'));

            match key.trim() {
                "id" => id = value,
                "title" => title = value,
                "assignee" => assignee = value,
                "status" => status = value,
                "priority" => priority = value,
                "due_date" => due_date = non_empty(value),
                "created_at" => created_at = value,
                "source_note_id" => source_note_id = non_empty(value),
                "source_kind" => source_kind = TodoSourceKind::from_str_opt(&value),
                "para" => para = ParaBand::from_str_opt(&value),
                "captured_at" => captured_at = non_empty(value),
                "source_ref" => {
                    // Re-read the untrimmed remainder: the JSON object was
                    // not quoted, and trimming quotes would have eaten the
                    // first key's opening quote.
                    source_ref = serde_json::from_str::<TodoSourceRef>(raw.trim()).ok();
                }
                _ => {}
            }
        }

        if id.is_empty() || title.is_empty() {
            return None;
        }

        Some(KanbanCard {
            id,
            title,
            assignee,
            status,
            priority,
            due_date,
            created_at,
            description,
            source_note_id,
            source_kind,
            source_ref,
            para,
            captured_at,
        })
    }
}

/// Keeps a value on its own line and inside its own quotes.
///
/// The original writer interpolated raw strings into a quoted line, which
/// was safe only while every value was developer-authored. Todo titles come
/// from speech and from typing, so a literal quote or newline has to be
/// prevented from ending the value early.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly what the previous writer produced, byte for byte.
    const LEGACY_CARD: &str = "---\nid: \"card_legacy_1\"\ntitle: \"Ship the parser\"\nassignee: \"Nitin\"\nstatus: \"in_progress\"\npriority: \"high\"\ndue_date: \"2026-03-01\"\ncreated_at: \"2026-01-04T10:00:00Z\"\nsource_note_id: \"note_42\"\n---\n\nThe body of the card.";

    /// The back-compat acceptance. Fails if deserialisation errors, or if
    /// any field the old writer stored is dropped.
    #[test]
    fn an_existing_card_still_loads() {
        let card = KanbanCard::parse_markdown(LEGACY_CARD).expect("a legacy card must still parse");

        assert_eq!(card.id, "card_legacy_1");
        assert_eq!(card.title, "Ship the parser");
        assert_eq!(card.assignee, "Nitin");
        assert_eq!(card.status, "in_progress");
        assert_eq!(card.priority, "high");
        assert_eq!(card.due_date.as_deref(), Some("2026-03-01"));
        assert_eq!(card.created_at, "2026-01-04T10:00:00Z");
        assert_eq!(card.source_note_id.as_deref(), Some("note_42"));
        assert_eq!(card.description, "The body of the card.");

        // The new fields are absent rather than invented.
        assert_eq!(card.source_kind, None);
        assert_eq!(card.source_ref, None);
        assert_eq!(card.para, None);
        assert_eq!(card.captured_at, None);

        // And an absent capture time still orders by something real.
        assert_eq!(card.captured_or_created(), "2026-01-04T10:00:00Z");
    }

    /// A legacy card must survive being loaded and written back — an
    /// upgrade that silently drops `assignee` would pass the read test
    /// above and still lose data.
    #[test]
    fn a_legacy_card_survives_a_round_trip_through_the_new_writer() {
        let card = KanbanCard::parse_markdown(LEGACY_CARD).expect("parses");
        let rewritten = KanbanCard::parse_markdown(&card.format_markdown()).expect("re-parses");
        assert_eq!(rewritten, card);
    }

    #[test]
    fn a_card_with_provenance_round_trips() {
        let card = KanbanCard::from_source(
            "Send the pricing deck",
            TodoSourceKind::Meeting,
            TodoSourceRef {
                id: "meeting_7".to_string(),
                turn_ordinal: Some(42),
                label: Some("Pricing sync".to_string()),
            },
            Some(ParaBand::Projects),
            Some("2026-02-01T09:30:00Z".to_string()),
        );

        let parsed = KanbanCard::parse_markdown(&card.format_markdown()).expect("parses");

        assert_eq!(parsed, card);
        assert_eq!(parsed.source_kind, Some(TodoSourceKind::Meeting));
        assert_eq!(parsed.para, Some(ParaBand::Projects));
        assert_eq!(parsed.captured_at.as_deref(), Some("2026-02-01T09:30:00Z"));
        let source_ref = parsed
            .source_ref
            .expect("provenance survives the round trip");
        assert_eq!(source_ref.id, "meeting_7");
        assert_eq!(source_ref.turn_ordinal, Some(42));
        assert_eq!(source_ref.label.as_deref(), Some("Pricing sync"));
    }

    /// A `---` inside a title or label used to end the frontmatter there,
    /// costing the card its status and provenance.
    #[test]
    fn triple_dashes_in_a_title_or_label_keep_the_card_whole() {
        let mut card = KanbanCard::from_source(
            "Q3 --- ship the deck",
            TodoSourceKind::Meeting,
            TodoSourceRef {
                id: "meeting_8".to_string(),
                turn_ordinal: Some(3),
                label: Some("Sync --- pricing".to_string()),
            },
            None,
            None,
        );
        card.status = "in_progress".to_string();
        card.description = "First\n---\nSecond".to_string();

        let parsed = KanbanCard::parse_markdown(&card.format_markdown()).expect("parses");

        assert_eq!(parsed, card);
    }

    /// Titles come from speech and from typing. Fails if a quote or a
    /// newline in a title ends the value early and corrupts the next key.
    #[test]
    fn a_title_cannot_break_out_of_its_own_line() {
        let mut card = KanbanCard::new_manual("Ask \"why\"\nthen decide: now");
        card.assignee = "Nitin".to_string();

        let parsed = KanbanCard::parse_markdown(&card.format_markdown()).expect("parses");

        assert_eq!(parsed.title, "Ask \"why\"\nthen decide: now");
        assert_eq!(parsed.assignee, "Nitin", "the next key must survive intact");
        assert_eq!(parsed.status, STATUS_TODO);
    }

    /// Provenance labels come from meeting titles, which are arbitrary too.
    #[test]
    fn a_quote_in_a_source_label_does_not_corrupt_the_reference() {
        let card = KanbanCard::from_source(
            "Follow up",
            TodoSourceKind::VoiceNote,
            TodoSourceRef {
                id: "note_1".to_string(),
                turn_ordinal: None,
                label: Some("The \"quarterly\" review".to_string()),
            },
            None,
            None,
        );

        let parsed = KanbanCard::parse_markdown(&card.format_markdown()).expect("parses");
        assert_eq!(
            parsed.source_ref.and_then(|r| r.label).as_deref(),
            Some("The \"quarterly\" review")
        );
    }

    #[test]
    fn a_manual_todo_records_that_it_was_typed_and_inherits_no_band() {
        let card = KanbanCard::new_manual("  Buy milk  ");

        assert_eq!(card.title, "Buy milk");
        assert_eq!(card.source_kind, Some(TodoSourceKind::Manual));
        assert_eq!(card.status, STATUS_TODO);
        // Nothing upstream to inherit from, so uncategorised — not guessed.
        assert_eq!(card.para, None);
        assert!(card.captured_at.is_some());
    }

    #[test]
    fn a_card_with_no_id_or_title_is_not_a_card() {
        assert!(KanbanCard::parse_markdown("---\nid: \"\"\ntitle: \"x\"\n---\n").is_none());
        assert!(KanbanCard::parse_markdown("---\nid: \"a\"\ntitle: \"\"\n---\n").is_none());
        assert!(KanbanCard::parse_markdown("no frontmatter here").is_none());
    }

    #[test]
    fn an_unknown_source_kind_reads_as_unrecorded_rather_than_failing() {
        let raw = "---\nid: \"c1\"\ntitle: \"t\"\nsource_kind: \"telepathy\"\n---\n\nbody";
        let card = KanbanCard::parse_markdown(raw).expect("the card still loads");
        assert_eq!(card.source_kind, None);
    }

    #[test]
    fn a_web_capture_kind_round_trips_under_its_stored_spelling() {
        assert_eq!(TodoSourceKind::WebCapture.as_str(), "web_capture");
        assert_eq!(
            TodoSourceKind::from_str_opt("web_capture"),
            Some(TodoSourceKind::WebCapture)
        );
    }

    #[test]
    fn the_board_has_three_columns_in_order() {
        assert_eq!(KANBAN_STATUSES, ["todo", "in_progress", "done"]);
    }
}
