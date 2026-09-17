//! Decisions, and how sure we are allowed to be about them.
//!
//! A decision is a `MemoryItem` of type `Decision` — the memory layer
//! already models provenance, confidence and supersession, and building a
//! second store for decisions would mean two answers to "what was decided"
//! and no way to tell which is current.
//!
//! What this module adds is the **confidence ladder**: a decision's
//! standing is derived from how it came to be known, never asserted by a
//! caller.
//!
//! | Provenance | Confidence | Where it comes from |
//! |---|---|---|
//! | Confirmed  | 0.98 | Stated, then confirmed again later |
//! | Captured   | 0.90 | The user stated it directly |
//! | Extracted  | 0.55 | Clear evidence in a transcript |
//! | Inferred   | 0.15 | A model's guess from scattered context |
//!
//! Captured deliberately stops at 0.90. The top of the scale is reserved
//! for a decision confirmed twice, so that "I said so" and "I said so, and
//! said so again" are not the same claim.
//!
//! ## The rule this module exists to enforce
//!
//! **A decision with no traceable source is never captured.** Provenance
//! is derived from the evidence attached to the memory, so a record with
//! no evidence cannot be dressed up as something the user said. The worst
//! failure available here is an invented "because" in the user's own
//! record of their project, and it is worse than an empty view.

use serde::{Deserialize, Serialize};

use super::model::{EpistemicState, MemoryItem, MemoryProvenance, MemoryStatus, MemoryType};
use super::store::MemoryStore;

/// How a decision came to be known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionProvenance {
    /// A model's guess from scattered context. Hidden unless asked for.
    Inferred,
    /// Clear evidence in a transcript, not yet confirmed by anyone.
    Extracted,
    /// The user stated it directly.
    Captured,
    /// Stated, and confirmed again on a separate occasion.
    Confirmed,
}

pub const CONFIDENCE_INFERRED: f32 = 0.15;
pub const CONFIDENCE_EXTRACTED: f32 = 0.55;
pub const CONFIDENCE_CAPTURED: f32 = 0.90;
pub const CONFIDENCE_CONFIRMED: f32 = 0.98;

/// `extracted_by` values that mean a person said it.
const BY_USER: &str = "user";
/// `extracted_by` values that mean something read it out of a transcript.
const EXTRACTOR_MARKERS: [&str; 3] = ["analysis", "deterministic_extractor", "llm"];

impl DecisionProvenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Extracted => "extracted",
            Self::Captured => "captured",
            Self::Confirmed => "confirmed",
        }
    }

    pub fn confidence(&self) -> f32 {
        match self {
            Self::Inferred => CONFIDENCE_INFERRED,
            Self::Extracted => CONFIDENCE_EXTRACTED,
            Self::Captured => CONFIDENCE_CAPTURED,
            Self::Confirmed => CONFIDENCE_CONFIRMED,
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "inferred" => Some(Self::Inferred),
            "extracted" => Some(Self::Extracted),
            "captured" => Some(Self::Captured),
            "confirmed" => Some(Self::Confirmed),
            _ => None,
        }
    }
}

/// One piece of evidence a decision rests on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionEvidence {
    /// The transcript, meeting, capture or note this came from.
    pub source_id: String,
    pub source_type: String,
    /// The words the decision was read from, or the user's own statement.
    pub evidence: String,
    pub extracted_by: String,
}

impl DecisionEvidence {
    /// Whether this actually points at something.
    ///
    /// An entry with no source and no quoted words is not traceable, and
    /// must not raise a decision's standing.
    fn is_traceable(&self) -> bool {
        !self.source_id.trim().is_empty() && !self.evidence.trim().is_empty()
    }
}

/// A decision as the Decision Tree renders it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: String,
    /// What the decision is about — the node it hangs from.
    pub subject: String,
    /// What was chosen.
    pub choice: String,
    /// Why, where a reason was given. Never invented when it was not.
    pub rationale: Option<String>,
    pub provenance: DecisionProvenance,
    pub confidence: f32,
    pub evidence: Vec<DecisionEvidence>,
    /// Set when a later decision replaced this one.
    pub superseded_by: Option<String>,
    /// Set when this decision replaced an earlier one.
    pub supersedes_id: Option<String>,
    pub superseded: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Derives standing from evidence, which is the whole point.
///
/// Only traceable evidence counts. A record whose every entry names no
/// source and quotes nothing falls to `Inferred` no matter what wrote it —
/// there is nothing to check, so there is nothing to believe.
fn derive_provenance(evidence: &[DecisionEvidence]) -> DecisionProvenance {
    let traceable: Vec<&DecisionEvidence> = evidence.iter().filter(|e| e.is_traceable()).collect();

    if traceable.is_empty() {
        return DecisionProvenance::Inferred;
    }

    let user_statements = traceable
        .iter()
        .filter(|e| e.extracted_by.eq_ignore_ascii_case(BY_USER))
        .count();

    if user_statements >= 2 {
        // Said once, then said again on a separate occasion. This is the
        // only way to the top of the scale.
        return DecisionProvenance::Confirmed;
    }
    if user_statements == 1 {
        return DecisionProvenance::Captured;
    }
    if traceable.iter().any(|e| {
        EXTRACTOR_MARKERS
            .iter()
            .any(|m| e.extracted_by.eq_ignore_ascii_case(m))
    }) {
        return DecisionProvenance::Extracted;
    }

    DecisionProvenance::Inferred
}

fn evidence_of(item: &MemoryItem) -> Vec<DecisionEvidence> {
    item.provenance
        .iter()
        .map(|p| DecisionEvidence {
            source_id: p.source_id.clone(),
            source_type: p.source_type.clone(),
            evidence: p.evidence.clone(),
            extracted_by: p.extracted_by.clone(),
        })
        .collect()
}

fn rationale_of(item: &MemoryItem) -> Option<String> {
    item.metadata
        .as_ref()
        .and_then(|m| m.get("rationale"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl DecisionRecord {
    /// Reads a memory item as a decision.
    ///
    /// Returns `None` for anything that is not one, so a caller cannot
    /// accidentally render a preference or a fact as a decision.
    pub fn from_memory(item: &MemoryItem) -> Option<Self> {
        if item.memory_type != MemoryType::Decision {
            return None;
        }
        let evidence = evidence_of(item);
        let provenance = derive_provenance(&evidence);

        Some(Self {
            id: item.id.clone(),
            subject: item.subject.clone(),
            choice: item.content.clone(),
            rationale: rationale_of(item),
            provenance,
            // Derived, never read back from the stored number: a
            // hand-edited `confidence` in the index must not be able to
            // promote a guess into something that draws as solid.
            confidence: provenance.confidence(),
            evidence,
            superseded_by: item.superseded_by.clone(),
            supersedes_id: item.supersedes_id.clone(),
            superseded: item.status == MemoryStatus::Superseded
                || item.epistemic_state == EpistemicState::NoLongerCurrent,
            created_at: item.created_at.clone(),
            updated_at: item.updated_at.clone(),
        })
    }
}

/// What a caller must supply to record a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewDecision {
    pub subject: String,
    pub choice: String,
    #[serde(default)]
    pub rationale: Option<String>,
    /// How this decision came to be known. The confidence follows from it;
    /// callers do not get to name a number.
    pub provenance: DecisionProvenance,
    pub source_id: String,
    pub source_type: String,
    /// The words this rests on: the user's statement, or the transcript
    /// line it was read from.
    pub evidence: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("A decision needs a subject and a choice")]
    Incomplete,

    #[error("A {level} decision needs a traceable source and the words it rests on")]
    Untraceable { level: &'static str },

    #[error("Decision {0} not found")]
    NotFound(String),

    #[error("Memory store error: {0}")]
    Store(String),
}

/// The `extracted_by` marker for a given standing.
fn marker_for(provenance: DecisionProvenance) -> &'static str {
    match provenance {
        DecisionProvenance::Captured | DecisionProvenance::Confirmed => BY_USER,
        DecisionProvenance::Extracted => "analysis",
        DecisionProvenance::Inferred => "inference",
    }
}

/// Records a decision.
///
/// Anything claiming to be captured or extracted must come with a source
/// and the words it rests on; without both, the call is refused rather
/// than quietly downgraded. A silent downgrade would put a faint line on
/// the tree where the caller believed it had put a solid one, and nobody
/// would find out.
pub fn record_decision(
    store: &MemoryStore,
    input: NewDecision,
) -> Result<DecisionRecord, DecisionError> {
    if input.subject.trim().is_empty() || input.choice.trim().is_empty() {
        return Err(DecisionError::Incomplete);
    }

    let traceable = !input.source_id.trim().is_empty() && !input.evidence.trim().is_empty();
    if !traceable && input.provenance != DecisionProvenance::Inferred {
        return Err(DecisionError::Untraceable {
            level: input.provenance.as_str(),
        });
    }

    let provenance = MemoryProvenance {
        source_id: input.source_id.trim().to_string(),
        source_type: input.source_type.trim().to_string(),
        evidence: input.evidence.trim().to_string(),
        confidence: input.provenance.confidence(),
        extracted_by: marker_for(input.provenance).to_string(),
    };

    let mut item = MemoryItem::new(
        MemoryType::Decision,
        input.subject.trim(),
        input.choice.trim(),
        provenance,
    );
    // An inferred decision is a guess, and the memory layer already has a
    // word for that: it is not something we hold as current truth.
    if input.provenance == DecisionProvenance::Inferred {
        item.epistemic_state = EpistemicState::Unverified;
    }
    item.metadata = Some(serde_json::json!({
        "rationale": input.rationale.unwrap_or_default(),
    }));

    let stored = store.create_memory(item).map_err(DecisionError::Store)?;

    DecisionRecord::from_memory(&stored).ok_or(DecisionError::Incomplete)
}

/// Confirms a decision, promoting it one rung.
///
/// This is a real write, not a UI state: it appends a user-attributed
/// provenance entry, which is what `derive_provenance` reads. An extracted
/// decision becomes captured; a captured one becomes confirmed. Confirming
/// again after that changes nothing, because there is no rung above.
pub fn confirm_decision(
    store: &MemoryStore,
    id: &str,
    source_id: &str,
    evidence: &str,
) -> Result<DecisionRecord, DecisionError> {
    if source_id.trim().is_empty() || evidence.trim().is_empty() {
        return Err(DecisionError::Untraceable { level: "confirmed" });
    }

    let existing = store
        .get_memory(id)
        .ok_or_else(|| DecisionError::NotFound(id.to_string()))?;
    if existing.memory_type != MemoryType::Decision {
        return Err(DecisionError::NotFound(id.to_string()));
    }

    let updated = store
        .update_memory(
            id,
            &existing.content,
            Some(MemoryProvenance {
                source_id: source_id.trim().to_string(),
                source_type: "confirmation".to_string(),
                evidence: evidence.trim().to_string(),
                confidence: CONFIDENCE_CAPTURED,
                extracted_by: BY_USER.to_string(),
            }),
        )
        .map_err(DecisionError::Store)?;

    let record = DecisionRecord::from_memory(&updated).ok_or(DecisionError::Incomplete)?;

    // Confirming settles the question, so a previously unverified guess
    // stops being one.
    if updated.epistemic_state != EpistemicState::Current {
        let _ = store.update_memory(id, &updated.content, None);
    }

    Ok(record)
}

/// Replaces a decision with a later one, keeping both.
///
/// The old decision is not deleted: it is marked superseded and points at
/// what replaced it, so the tree can show that the question was reopened
/// and what the answer became. That is the memory layer's own supersession
/// and this does not reimplement it.
pub fn supersede_decision(
    store: &MemoryStore,
    old_id: &str,
    input: NewDecision,
) -> Result<(DecisionRecord, DecisionRecord), DecisionError> {
    if input.choice.trim().is_empty() {
        return Err(DecisionError::Incomplete);
    }
    if input.source_id.trim().is_empty() || input.evidence.trim().is_empty() {
        return Err(DecisionError::Untraceable {
            level: input.provenance.as_str(),
        });
    }

    let (old, new) = store
        .supersede_memory(
            old_id,
            input.choice.trim(),
            MemoryProvenance {
                source_id: input.source_id.trim().to_string(),
                source_type: input.source_type.trim().to_string(),
                evidence: input.evidence.trim().to_string(),
                confidence: input.provenance.confidence(),
                extracted_by: marker_for(input.provenance).to_string(),
            },
        )
        .map_err(DecisionError::Store)?;

    let old_record = DecisionRecord::from_memory(&old).ok_or(DecisionError::Incomplete)?;
    let new_record = DecisionRecord::from_memory(&new).ok_or(DecisionError::Incomplete)?;
    Ok((old_record, new_record))
}

/// Every decision, superseded ones included.
///
/// The tree keeps what was reversed — that is most of what makes it worth
/// reading — so this cannot use `list_active`, which filters superseded
/// records out. Soft-deleted records are excluded: those the user asked to
/// be rid of.
pub fn list_decisions(store: &MemoryStore) -> Vec<DecisionRecord> {
    let mut records: Vec<DecisionRecord> = store
        .list_all()
        .iter()
        .filter(|m| m.memory_type == MemoryType::Decision)
        .filter(|m| m.status != MemoryStatus::Deleted)
        .filter_map(DecisionRecord::from_memory)
        .collect();

    // Oldest first, so a supersession chain reads in the order it happened.
    records.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (MemoryStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("vox_decisions_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        (MemoryStore::new(&dir), dir)
    }

    fn captured(subject: &str, choice: &str) -> NewDecision {
        NewDecision {
            subject: subject.to_string(),
            choice: choice.to_string(),
            rationale: Some("Because the alternative needed a server.".to_string()),
            provenance: DecisionProvenance::Captured,
            source_id: "note_1".to_string(),
            source_type: "scribble".to_string(),
            evidence: "We're going with LanceDB.".to_string(),
        }
    }

    /// The ladder, exactly as specified. Fails if any rung moves.
    #[test]
    fn the_confidence_ladder_is_the_one_in_the_brief() {
        assert_eq!(DecisionProvenance::Captured.confidence(), 0.90);
        assert_eq!(DecisionProvenance::Extracted.confidence(), 0.55);
        assert_eq!(DecisionProvenance::Inferred.confidence(), 0.15);
    }

    /// Captured must stop short of the top: the top is for a decision
    /// confirmed twice. Fails if captured is ever 1.0.
    #[test]
    fn captured_is_not_certainty() {
        assert!(DecisionProvenance::Captured.confidence() < 1.0);
        assert!(
            DecisionProvenance::Confirmed.confidence() > DecisionProvenance::Captured.confidence()
        );
    }

    #[test]
    fn a_captured_decision_records_its_standing_and_its_reason() {
        let (store, dir) = store();
        let record = record_decision(&store, captured("Retrieval", "Use LanceDB")).unwrap();

        assert_eq!(record.provenance, DecisionProvenance::Captured);
        assert_eq!(record.confidence, 0.90);
        assert_eq!(
            record.rationale.as_deref(),
            Some("Because the alternative needed a server.")
        );
        assert_eq!(record.evidence.len(), 1);
        assert_eq!(record.evidence[0].source_id, "note_1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The hard rule. Fails if a decision with nothing behind it can be
    /// written as captured — an invented "because" in the user's own
    /// record is the worst thing available here.
    #[test]
    fn a_decision_with_no_traceable_source_cannot_be_captured() {
        let (store, dir) = store();

        let no_source = NewDecision {
            source_id: "  ".to_string(),
            ..captured("Retrieval", "Use LanceDB")
        };
        assert!(matches!(
            record_decision(&store, no_source),
            Err(DecisionError::Untraceable { .. })
        ));

        let no_evidence = NewDecision {
            evidence: String::new(),
            ..captured("Retrieval", "Use LanceDB")
        };
        assert!(matches!(
            record_decision(&store, no_evidence),
            Err(DecisionError::Untraceable { .. })
        ));

        // Refused, not downgraded: nothing was written.
        assert!(list_decisions(&store).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A guess is allowed to have no source — that is what makes it a
    /// guess — but it must land on the bottom rung.
    #[test]
    fn an_unsourced_guess_is_allowed_but_stays_a_guess() {
        let (store, dir) = store();

        let guess = NewDecision {
            provenance: DecisionProvenance::Inferred,
            source_id: String::new(),
            evidence: String::new(),
            rationale: None,
            ..captured("Retrieval", "Probably LanceDB")
        };
        let record = record_decision(&store, guess).unwrap();

        assert_eq!(record.provenance, DecisionProvenance::Inferred);
        assert_eq!(record.confidence, 0.15);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Standing is derived from evidence, so editing the stored number by
    /// hand must not promote anything. Fails if a tampered index can make
    /// a guess draw as solid.
    #[test]
    fn a_hand_edited_confidence_cannot_promote_a_guess() {
        let mut item = MemoryItem::new(
            MemoryType::Decision,
            "Retrieval",
            "Use LanceDB",
            MemoryProvenance {
                source_id: String::new(),
                source_type: "guess".to_string(),
                evidence: String::new(),
                confidence: 0.99,
                extracted_by: "inference".to_string(),
            },
        );
        item.confidence = 0.99;

        let record = DecisionRecord::from_memory(&item).unwrap();
        assert_eq!(record.provenance, DecisionProvenance::Inferred);
        assert_eq!(record.confidence, 0.15);
    }

    /// Confirming is a real write. Fails if the promotion is only visible
    /// in the returned value and not in what a later read sees.
    #[test]
    fn confirming_an_extracted_decision_promotes_it_on_disk() {
        let (store, dir) = store();

        let extracted = NewDecision {
            provenance: DecisionProvenance::Extracted,
            source_id: "meeting_3".to_string(),
            source_type: "meeting".to_string(),
            evidence: "I think we said LanceDB.".to_string(),
            ..captured("Retrieval", "Use LanceDB")
        };
        let record = record_decision(&store, extracted).unwrap();
        assert_eq!(record.provenance, DecisionProvenance::Extracted);

        let promoted = confirm_decision(&store, &record.id, "note_9", "Yes, LanceDB.").unwrap();
        assert_eq!(promoted.provenance, DecisionProvenance::Captured);
        assert_eq!(promoted.confidence, 0.90);

        // The promotion survives a fresh read of the store.
        let reread = list_decisions(&store);
        assert_eq!(reread.len(), 1);
        assert_eq!(reread[0].provenance, DecisionProvenance::Captured);
        assert_eq!(reread[0].evidence.len(), 2, "both sources are kept");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Said once, then again: the only route to the top of the scale.
    #[test]
    fn confirming_a_captured_decision_twice_reaches_the_top_rung() {
        let (store, dir) = store();
        let record = record_decision(&store, captured("Retrieval", "Use LanceDB")).unwrap();

        let confirmed = confirm_decision(&store, &record.id, "note_2", "Still LanceDB.").unwrap();
        assert_eq!(confirmed.provenance, DecisionProvenance::Confirmed);
        assert_eq!(confirmed.confidence, 0.98);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirming_without_a_source_is_refused() {
        let (store, dir) = store();
        let record = record_decision(&store, captured("Retrieval", "Use LanceDB")).unwrap();

        assert!(matches!(
            confirm_decision(&store, &record.id, "", "sure"),
            Err(DecisionError::Untraceable { .. })
        ));
        assert!(matches!(
            confirm_decision(&store, "mem_nope", "note_2", "sure"),
            Err(DecisionError::NotFound(_))
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reversed decision is kept and points at what replaced it. Fails
    /// if superseding deletes the original — the record of having changed
    /// your mind is most of what the tree is for.
    #[test]
    fn a_superseded_decision_is_kept_and_linked_to_its_replacement() {
        let (store, dir) = store();
        let first = record_decision(&store, captured("Retrieval", "Use LanceDB")).unwrap();

        let (old, new) = supersede_decision(
            &store,
            &first.id,
            NewDecision {
                choice: "Use SQLite FTS".to_string(),
                rationale: Some("LanceDB pulled in too much.".to_string()),
                source_id: "note_5".to_string(),
                evidence: "Actually, let's drop LanceDB.".to_string(),
                ..captured("Retrieval", "Use SQLite FTS")
            },
        )
        .unwrap();

        assert!(old.superseded);
        assert_eq!(old.superseded_by.as_deref(), Some(new.id.as_str()));
        assert_eq!(new.supersedes_id.as_deref(), Some(old.id.as_str()));
        assert!(!new.superseded);

        // Both are still listed — the tree draws the old one struck
        // through rather than losing it.
        let all = list_decisions(&store);
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0].id, old.id,
            "oldest first, so the chain reads in order"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_decision_needs_a_subject_and_a_choice() {
        let (store, dir) = store();

        assert!(matches!(
            record_decision(
                &store,
                NewDecision {
                    choice: "  ".to_string(),
                    ..captured("S", "C")
                }
            ),
            Err(DecisionError::Incomplete)
        ));
        assert!(matches!(
            record_decision(
                &store,
                NewDecision {
                    subject: String::new(),
                    ..captured("S", "C")
                }
            ),
            Err(DecisionError::Incomplete)
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only decisions. Fails if a preference or a fact can be rendered as
    /// something that was decided.
    #[test]
    fn a_memory_that_is_not_a_decision_is_not_read_as_one() {
        let item = MemoryItem::new(
            MemoryType::Preference,
            "Editor",
            "Prefers dark mode",
            MemoryProvenance {
                source_id: "note_1".to_string(),
                source_type: "scribble".to_string(),
                evidence: "I like dark mode".to_string(),
                confidence: 1.0,
                extracted_by: "user".to_string(),
            },
        );
        assert!(DecisionRecord::from_memory(&item).is_none());
    }
}
