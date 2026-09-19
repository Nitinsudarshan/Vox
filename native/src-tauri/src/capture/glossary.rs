//! Words the user has told Vox about, and what happens when one of them is
//! probably what a decoder was reaching for.
//!
//! ## Evidence, not find-and-replace
//!
//! A glossary is a claim about vocabulary — "this meeting contains the word
//! Bengaluru" — and the temptation is to treat it as a claim about text:
//! rewrite anything close enough and move on. Vox did exactly that. Any token
//! within one edit of a glossary term was rewritten in place, before the
//! segment was ever written to disk, with no record of what had been changed
//! or from what.
//!
//! That is a transcription source wearing a vocabulary list's clothes. If the
//! decoder heard "banglore" and the glossary says "Bengaluru", something
//! probably went right. If it heard "Bangor" — a real place — and the
//! glossary says "Bengaluru", something went badly wrong, and under blind
//! replacement the transcript says Bengaluru and nothing says otherwise.
//!
//! So a correction here is **recorded**: what the decoder said, what it was
//! changed to, which glossary term did it, and why. The line can be read back
//! as it was decoded, and a wrong correction is visible rather than
//! indistinguishable from a correct transcription.
//!
//! ## Categories, because the risks differ
//!
//! An acronym is nearly always right to normalize — `api` to `API` changes
//! no words. A person's name is the riskiest thing in the list, because names
//! are short, numerous and easy to confuse with ordinary words. Keeping the
//! category means a later change can be made per category instead of to
//! everything at once.
//!
//! ## What this is not
//!
//! Not a spell-checker. The near-miss rule is one edit, so "banglore" — four
//! edits from "Bengaluru", and obvious to a human — is left exactly as the
//! decoder said it. A rule loose enough to catch that is loose enough to
//! rewrite words nobody meant, and the failure it would cause is silent. The
//! narrower rule leaves more errors in place and puts none in.

use serde::{Deserialize, Serialize};

/// Minimum token length before a term is matched by edit distance rather than
/// exactly.
///
/// Short tokens are close to too many things. At four characters a single
/// edit reaches a quarter of the dictionary, and a glossary that rewrites
/// "form" to "Ford" is worse than no glossary.
const MIN_FUZZY_LEN: usize = 6;

/// What kind of thing a term is.
///
/// Not decoration: the categories carry different risk. Rewriting an acronym's
/// casing changes no words. Rewriting something to a person's name might
/// change who a meeting says made a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermCategory {
    Person,
    Organization,
    Product,
    Technical,
    Acronym,
    /// Anything the user added without saying what it is.
    #[default]
    Custom,
}

impl TermCategory {
    pub fn key(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Product => "product",
            Self::Technical => "technical",
            Self::Acronym => "acronym",
            Self::Custom => "custom",
        }
    }

    /// Whether a near miss may be corrected to this term, or only an exact
    /// match.
    ///
    /// Names are excluded. They are short, there are many of them, and they
    /// collide with ordinary words — "Marc" and "mark", "Bill" and "bill",
    /// "Rose" and "rose". Getting one wrong changes who a meeting says
    /// committed to something, which is the single most consequential thing a
    /// meeting transcript asserts.
    pub fn allows_near_miss(self) -> bool {
        !matches!(self, Self::Person)
    }
}

/// One thing the user has told Vox about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryTerm {
    /// The spelling a transcript should use.
    pub term: String,
    #[serde(default)]
    pub category: TermCategory,
}

impl GlossaryTerm {
    pub fn new(term: impl Into<String>, category: TermCategory) -> Self {
        Self {
            term: term.into(),
            category,
        }
    }

    /// A bare string from the existing settings list, with no category given.
    pub fn untyped(term: impl Into<String>) -> Self {
        Self::new(term, TermCategory::Custom)
    }
}

/// Why a word was changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CorrectionReason {
    /// The word matched the term exactly apart from casing.
    Casing,
    /// The word was within one edit of the term.
    NearMiss,
}

/// One word the glossary changed, and the evidence for it.
///
/// Persisted with the segment, so the line can be read back as the decoder
/// produced it and a wrong correction is visible rather than silently
/// authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermCorrection {
    /// What the decoder actually said.
    pub from: String,
    /// What it was changed to.
    pub to: String,
    /// The glossary term responsible.
    pub term: String,
    pub category: TermCategory,
    pub reason: CorrectionReason,
}

/// The user's vocabulary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Glossary {
    #[serde(default)]
    pub terms: Vec<GlossaryTerm>,
}

impl Glossary {
    pub fn new(terms: Vec<GlossaryTerm>) -> Self {
        Self { terms }
    }

    /// Builds from the plain list of strings settings already store.
    ///
    /// Every term arrives as [`TermCategory::Custom`], because that is the
    /// truth — the existing setting records no category and inventing one
    /// would make the riskiest rule (names) apply to words nobody said were
    /// names.
    pub fn from_settings(terms: &[String]) -> Self {
        Self::new(
            terms
                .iter()
                .map(|term| term.trim())
                .filter(|term| !term.is_empty())
                .map(GlossaryTerm::untyped)
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Applies the glossary, returning the corrected text and what it changed.
    ///
    /// Word by word, never across a whole line: a phrase-level rewrite cannot
    /// say which word it was reaching for, and a correction that cannot name
    /// its own evidence is the thing this module exists to avoid.
    pub fn apply(&self, text: &str) -> (String, Vec<TermCorrection>) {
        if self.terms.is_empty() {
            return (text.to_string(), Vec::new());
        }
        let mut corrections = Vec::new();
        let corrected = text
            .split(' ')
            .map(|word| match self.correct_word(word) {
                Some((replacement, correction)) => {
                    corrections.push(correction);
                    replacement
                }
                None => word.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        (corrected, corrections)
    }

    /// The replacement for one word, if the glossary has a claim on it.
    fn correct_word(&self, word: &str) -> Option<(String, TermCorrection)> {
        let core = trimmed_core(word);
        if core.is_empty() {
            return None;
        }
        let key = core.to_lowercase();

        // Exact-but-for-casing first, and across every category: it changes no
        // words, so there is nothing to be careful about.
        if let Some(term) = self
            .terms
            .iter()
            .find(|candidate| candidate.term.to_lowercase() == key)
        {
            if term.term == core {
                // Already right. Recording a correction here would fill the
                // evidence with non-events.
                return None;
            }
            return Some((
                replace_core(word, &term.term),
                TermCorrection {
                    from: core.to_string(),
                    to: term.term.clone(),
                    term: term.term.clone(),
                    category: term.category,
                    reason: CorrectionReason::Casing,
                },
            ));
        }

        if key.chars().count() < MIN_FUZZY_LEN {
            return None;
        }
        let term = self.terms.iter().find(|candidate| {
            candidate.category.allows_near_miss()
                && is_within_one_edit(&candidate.term.to_lowercase(), &key)
        })?;
        Some((
            replace_core(word, &term.term),
            TermCorrection {
                from: core.to_string(),
                to: term.term.clone(),
                term: term.term.clone(),
                category: term.category,
                reason: CorrectionReason::NearMiss,
            },
        ))
    }
}

/// Undoes a line's corrections, giving back what the decoder said.
///
/// Exists so "preserve the raw ASR text" is a property of the data rather than
/// a second copy of every line: the corrections *are* the difference, and
/// applying them backwards reconstructs the original exactly.
pub fn uncorrect(text: &str, corrections: &[TermCorrection]) -> String {
    if corrections.is_empty() {
        return text.to_string();
    }
    text.split(' ')
        .map(|word| {
            let core = trimmed_core(word);
            match corrections
                .iter()
                .find(|correction| correction.to == core)
            {
                Some(correction) => replace_core(word, &correction.from),
                None => word.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The word without its surrounding punctuation.
fn trimmed_core(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .to_string()
}

/// Swaps a word's core, keeping whatever punctuation surrounded it.
fn replace_core(word: &str, replacement: &str) -> String {
    let core = trimmed_core(word);
    if core.is_empty() {
        return word.to_string();
    }
    match word.find(&core) {
        Some(at) => format!("{}{}{}", &word[..at], replacement, &word[at + core.len()..]),
        None => word.to_string(),
    }
}

/// Whether two strings differ by at most one insertion, deletion or
/// substitution.
fn is_within_one_edit(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (long, short) = if a.len() >= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }

    let mut edits = 0usize;
    let (mut i, mut j) = (0usize, 0usize);
    while i < long.len() && j < short.len() {
        if long[i] == short[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if long.len() == short.len() {
            i += 1;
            j += 1;
        } else {
            i += 1;
        }
    }
    edits + (long.len() - i) + (short.len() - j) <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        Glossary::new(vec![
            GlossaryTerm::new("Bengaluru", TermCategory::Organization),
            GlossaryTerm::new("Supabase", TermCategory::Product),
            GlossaryTerm::new("API", TermCategory::Acronym),
            GlossaryTerm::new("Payal", TermCategory::Person),
        ])
    }

    #[test]
    fn a_correction_records_what_was_changed_and_why() {
        // The whole point: a correction that cannot name its own evidence is
        // indistinguishable from a transcription.
        let (text, corrections) = glossary().apply("we shipped to bengalru");
        assert_eq!(text, "we shipped to Bengaluru");
        assert_eq!(corrections.len(), 1);

        let correction = &corrections[0];
        assert_eq!(correction.from, "bengalru");
        assert_eq!(correction.to, "Bengaluru");
        assert_eq!(correction.category, TermCategory::Organization);
        assert_eq!(correction.reason, CorrectionReason::NearMiss);
    }

    #[test]
    fn the_decoders_own_words_can_always_be_read_back() {
        // "Preserve the raw ASR text" as a property of the data rather than a
        // second copy of every line.
        let spoken = "we shipped supabse to bengalru";
        let (text, corrections) = glossary().apply(spoken);
        assert_ne!(text, spoken);
        assert_eq!(uncorrect(&text, &corrections), spoken);
    }

    #[test]
    fn a_near_miss_is_never_corrected_to_a_persons_name() {
        // Names are short, numerous, and collide with ordinary words. Getting
        // one wrong changes who a meeting says committed to something.
        let (text, corrections) = glossary().apply("the payall was late");
        assert_eq!(text, "the payall was late");
        assert!(corrections.is_empty());
    }

    #[test]
    fn a_persons_name_is_still_fixed_when_only_the_casing_is_wrong() {
        // Casing changes no words, so there is nothing to be careful about.
        let (text, corrections) = glossary().apply("payal will send it");
        assert_eq!(text, "Payal will send it");
        assert_eq!(corrections[0].reason, CorrectionReason::Casing);
        assert_eq!(corrections[0].category, TermCategory::Person);
    }

    #[test]
    fn a_common_misspelling_further_than_one_edit_is_left_alone() {
        // "banglore" is four edits from "Bengaluru" and a human would fix it
        // instantly. The glossary will not, and that is the intended trade: a
        // rule loose enough to catch it is loose enough to rewrite words
        // nobody meant. This is a vocabulary list, not a spell-checker.
        let (text, corrections) = glossary().apply("we shipped to banglore");
        assert_eq!(text, "we shipped to banglore");
        assert!(corrections.is_empty());
    }

    #[test]
    fn a_short_word_is_never_matched_by_edit_distance() {
        // At four characters a single edit reaches a quarter of the
        // dictionary, and a glossary that rewrites "form" to "Ford" is worse
        // than no glossary.
        let terms = Glossary::new(vec![GlossaryTerm::new("Ford", TermCategory::Organization)]);
        let (text, corrections) = terms.apply("fill in the form");
        assert_eq!(text, "fill in the form");
        assert!(corrections.is_empty());
    }

    #[test]
    fn a_word_that_is_already_correct_records_no_correction() {
        // Otherwise the evidence fills with non-events and a real correction
        // is impossible to find.
        let (text, corrections) = glossary().apply("Bengaluru and Supabase");
        assert_eq!(text, "Bengaluru and Supabase");
        assert!(corrections.is_empty());
    }

    #[test]
    fn punctuation_around_a_corrected_word_survives() {
        let (text, _) = glossary().apply("we use supabase, and the api.");
        assert_eq!(text, "we use Supabase, and the API.");
    }

    #[test]
    fn an_empty_glossary_changes_nothing_and_allocates_no_evidence() {
        let (text, corrections) = Glossary::default().apply("anything at all");
        assert_eq!(text, "anything at all");
        assert!(corrections.is_empty());
    }

    #[test]
    fn terms_from_the_existing_setting_arrive_uncategorised_rather_than_guessed() {
        // The setting records no category, and inventing one would apply the
        // riskiest rule to words nobody said were names.
        let glossary = Glossary::from_settings(&["Tauri".into(), "  ".into(), "Vox".into()]);
        assert_eq!(glossary.terms.len(), 2);
        assert!(glossary
            .terms
            .iter()
            .all(|term| term.category == TermCategory::Custom));
    }

    #[test]
    fn one_edit_is_one_edit() {
        assert!(is_within_one_edit("bengaluru", "bengalru"));
        assert!(is_within_one_edit("bengaluru", "bengaluruu"));
        assert!(is_within_one_edit("bengaluru", "bengalura"));
        assert!(is_within_one_edit("bengaluru", "bengaluru"));
        assert!(!is_within_one_edit("bengaluru", "bangalore"));
        assert!(!is_within_one_edit("bengaluru", "xy"));
    }

    #[test]
    fn a_correction_round_trips_through_json_so_it_can_be_stored() {
        let (_, corrections) = glossary().apply("banglore");
        let json = serde_json::to_string(&corrections).expect("serialize");
        let back: Vec<TermCorrection> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(corrections, back);
    }
}
