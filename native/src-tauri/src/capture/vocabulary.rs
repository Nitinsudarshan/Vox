//! Domain vocabulary and prompt context construction for Whisper ASR.
//!
//! Whisper's attention decoder can be conditioned using `initial_prompt`. When primed
//! with domain-specific terms (names, acronyms, technical frameworks) and recent
//! conversational transcript context, recognition accuracy on specialized terminology
//! and cross-boundary utterances improves significantly.
//!
//! To prevent hallucination loops or prompt bloat:
//! 1. Vocabulary terms are deduplicated and length-budgeted.
//! 2. Previous segment context is screened to ensure only clean, non-repetitive text is forwarded.
//! 3. When a segment was a forced split, continuity is emphasized in prompt assembly.
//!
//! ## Why the order inside the prompt is not arbitrary
//!
//! Whisper reads `initial_prompt` as *text that came before this audio*, so
//! the last thing in it is the thing the decoder treats as most recent. Two
//! separate mechanisms then reward putting the real preceding speech at the
//! end and the keyword list in front of it:
//!
//! - **What the model weighs.** A comma-separated run of proper nouns in the
//!   position where the previous sentence belongs tells the decoder that the
//!   last thing said was a list of names — which is how a prompt meant to help
//!   with `NavGurukul` starts inserting it into audio that never contained it.
//! - **What survives truncation.** whisper.cpp caps the prompt at
//!   `n_text_ctx/2 - 1` tokens and keeps the *last* ones (`whisper_full_with_state`).
//!   So an over-budget prompt loses its vocabulary and keeps its context, which
//!   is the right way round — and the opposite of what a tail-last budget would
//!   have produced. The character budget below is a proxy for that token limit:
//!   Latin text runs well under one token per character and Devanagari well
//!   over, which is the same asymmetry [`crate::capture::stt::WhisperDecodingConfig::for_expensive_script`]
//!   exists for.

use serde::{Deserialize, Serialize};

/// Curated global vocabulary covering key project, community, organization,
/// and regional Hindi/English domain entities.
pub const GLOBAL_DOMAIN_VOCABULARY: &[&str] = &[
    "NavGurukul",
    "SOSC",
    "NGConnect",
    "GHAR",
    "Sama Saathi",
    "Ares",
    "Macquarie",
    "Kishore",
    "Abhishek",
    "Tauri",
    "Rust",
    "Whisper",
    "Supabase",
    "React",
    "TypeScript",
    "Ollama",
    "LanceDB",
];

/// Characters of `initial_prompt` a meeting segment is decoded with.
///
/// A budget rather than a limit: whisper's own cap is on tokens, and the
/// character-to-token ratio differs by an order of magnitude between Latin and
/// Devanagari, so no character count is the real boundary. This one is low
/// enough that a Latin prompt is nowhere near the cap and a Devanagari one
/// overshoots it only by losing vocabulary it can afford to lose — see the
/// module comment on what whisper drops when a prompt is too long.
pub const PROMPT_BUDGET_CHARS: usize = 220;

/// Budget below which the vocabulary list is left out entirely. A handful of
/// characters buys one truncated term, which primes nothing and still reads to
/// the decoder as speech.
const MIN_VOCABULARY_BUDGET: usize = 20;

/// Multi-tier vocabulary provider managing global, meeting-specific, and user dictionaries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DomainVocabulary {
    pub global_terms: Vec<String>,
    pub meeting_terms: Vec<String>,
    pub user_terms: Vec<String>,
}

impl DomainVocabulary {
    /// Creates a default domain vocabulary containing default global terms.
    pub fn new() -> Self {
        Self {
            global_terms: GLOBAL_DOMAIN_VOCABULARY
                .iter()
                .map(|s| s.to_string())
                .collect(),
            meeting_terms: Vec::new(),
            user_terms: Vec::new(),
        }
    }

    /// Extends vocabulary with meeting-specific keywords (e.g. from title, agenda, attendee names).
    pub fn with_meeting_terms(mut self, terms: &[String]) -> Self {
        for term in terms {
            let trimmed = term.trim();
            if !trimmed.is_empty() && !self.meeting_terms.iter().any(|t| t.eq_ignore_ascii_case(trimmed)) {
                self.meeting_terms.push(trimmed.to_string());
            }
        }
        self
    }

    /// Extends vocabulary with user custom dictionary terms.
    pub fn with_user_terms(mut self, terms: &[String]) -> Self {
        for term in terms {
            let trimmed = term.trim();
            if !trimmed.is_empty() && !self.user_terms.iter().any(|t| t.eq_ignore_ascii_case(trimmed)) {
                self.user_terms.push(trimmed.to_string());
            }
        }
        self
    }

    /// Returns a deduplicated list of active vocabulary terms prioritized by:
    /// Meeting-specific > User-custom > Global domain terms.
    pub fn active_terms(&self) -> Vec<String> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for term in self.meeting_terms.iter().chain(self.user_terms.iter()).chain(self.global_terms.iter()) {
            let lower = term.to_lowercase();
            if !seen.contains(&lower) {
                seen.insert(lower);
                result.push(term.clone());
            }
        }
        result
    }

    /// Constructs an effective `initial_prompt` for a speech segment.
    ///
    /// The result reads `<user prompt>. <vocabulary>. <what was just said>` —
    /// see the module comment for why the preceding speech goes last and not
    /// first.
    ///
    /// - `user_prompt`: the free-text prompt the user configured in settings,
    ///   if any. Included rather than replaced: it is the most specific thing
    ///   anyone has said about this audio, and a meeting silently dropping it
    ///   while dictation honoured it is the bug this argument exists to fix.
    /// - `prev_transcript`: clean text from the preceding segment, if it was
    ///   kept by the quality gate. A rejected segment passes `None` here, which
    ///   is what stops a hallucination priming its successor.
    /// - `is_continuation`: true when the previous segment was cut at the
    ///   segmenter's ceiling, so this one continues the same sentence and a
    ///   longer tail is worth its budget.
    /// - `max_prompt_chars`: the character budget, clamped to a sane range.
    ///   See [`PROMPT_BUDGET_CHARS`].
    pub fn build_prompt(
        &self,
        user_prompt: Option<&str>,
        prev_transcript: Option<&str>,
        is_continuation: bool,
        max_prompt_chars: usize,
    ) -> Option<String> {
        let max_chars = max_prompt_chars.clamp(60, 450);

        // The tail is costed first because it is the part worth the most, not
        // because it is emitted first — it is emitted last. Claiming its budget
        // up front is what stops a long vocabulary list from crowding out the
        // sentence this segment is continuing.
        let tail = prev_transcript
            .map(str::trim)
            .filter(|prev| !prev.is_empty())
            .map(|prev| extract_recent_tail(prev, if is_continuation { 140 } else { 90 }))
            .filter(|tail| !tail.is_empty());
        let mut budget = max_chars.saturating_sub(tail.as_ref().map_or(0, |t| charge(t)));

        let mut lead: Vec<String> = Vec::new();

        if let Some(user) = user_prompt.map(str::trim).filter(|p| !p.is_empty()) {
            // All of it or none of it: half a sentence the user wrote is worse
            // conditioning than the sentence being absent.
            if charge(user) <= budget {
                budget -= charge(user);
                lead.push(user.to_string());
            }
        }

        if budget > MIN_VOCABULARY_BUDGET {
            let mut vocab_chunk = Vec::new();
            let mut vocab_chars = 0;
            for term in self.active_terms() {
                if vocab_chars + charge(&term) > budget {
                    break;
                }
                vocab_chars += charge(&term);
                vocab_chunk.push(term);
            }
            if !vocab_chunk.is_empty() {
                lead.push(vocab_chunk.join(", "));
            }
        }

        let mut parts = lead;
        parts.extend(tail);

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(". "))
        }
    }
}

/// What one prompt part costs against the budget: its own characters plus the
/// `". "` that will separate it from the next.
fn charge(part: &str) -> usize {
    part.chars().count() + 2
}

/// Helper to extract the trailing tail of a transcript string without cutting mid-word or mid-char.
fn extract_recent_tail(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    let skip_chars = char_count - max_chars;
    let start_byte = text
        .char_indices()
        .nth(skip_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(0);

    let slice = &text[start_byte..];
    if let Some((space_idx, _)) = slice.char_indices().find(|&(_, c)| c.is_whitespace()) {
        let after_space = slice[space_idx..]
            .char_indices()
            .find(|&(_, c)| !c.is_whitespace())
            .map(|(idx, _)| space_idx + idx);

        if let Some(clean_start) = after_space {
            if clean_start < slice.len() {
                return slice[clean_start..].to_string();
            }
        }
    }

    slice.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_vocabulary_defaults() {
        let vocab = DomainVocabulary::new();
        let terms = vocab.active_terms();
        assert!(terms.contains(&"NavGurukul".to_string()));
        assert!(terms.contains(&"SOSC".to_string()));
        assert!(terms.contains(&"Tauri".to_string()));
    }

    #[test]
    fn test_vocabulary_prompt_construction() {
        let vocab = DomainVocabulary::new()
            .with_meeting_terms(&["Sprint Planning".to_string(), "Project Ares".to_string()]);

        let prompt = vocab.build_prompt(
            None,
            Some("We were discussing the budget for the upcoming quarter"),
            false,
            200,
        );
        assert!(prompt.is_some());
        let p = prompt.unwrap();
        assert!(p.contains("budget for the upcoming quarter"));
        assert!(p.contains("Sprint Planning"));
    }

    #[test]
    fn test_forced_split_prompt_continuation() {
        let vocab = DomainVocabulary::new();
        let prev = "Actually mujhe lagta hai we should finalize this by Friday because agar numbers kal tak aa jaate";
        let prompt = vocab
            .build_prompt(None, Some(prev), true, PROMPT_BUDGET_CHARS)
            .unwrap();
        assert!(prompt.contains("numbers kal tak aa jaate"));
    }

    /// The decoder reads the end of the prompt as the most recent speech, and
    /// whisper truncates a long prompt from the front. Both want the sentence
    /// last and the keyword list first.
    #[test]
    fn preceding_speech_is_the_last_thing_in_the_prompt() {
        let vocab = DomainVocabulary::new();
        let prev = "so the migration lands on Thursday";
        let prompt = vocab
            .build_prompt(None, Some(prev), false, PROMPT_BUDGET_CHARS)
            .unwrap();

        assert!(prompt.ends_with(prev), "prompt was {prompt:?}");
        let vocab_at = prompt.find("NavGurukul").expect("vocabulary is present");
        let speech_at = prompt.find(prev).expect("preceding speech is present");
        assert!(vocab_at < speech_at, "prompt was {prompt:?}");
    }

    /// A meeting used to overwrite the prompt the user configured in settings,
    /// while dictation honoured it. Both now carry it.
    #[test]
    fn a_configured_prompt_is_kept_alongside_the_vocabulary() {
        let vocab = DomainVocabulary::new();
        let prompt = vocab
            .build_prompt(
                Some("Kubernetes, Grafana, Prometheus"),
                Some("we rolled the release back"),
                false,
                PROMPT_BUDGET_CHARS,
            )
            .unwrap();

        assert!(prompt.starts_with("Kubernetes, Grafana, Prometheus"));
        assert!(prompt.ends_with("we rolled the release back"));
    }

    /// The tail claims its budget before the vocabulary does, so a long list
    /// cannot crowd out the sentence this segment continues.
    #[test]
    fn the_tail_is_never_crowded_out_by_vocabulary() {
        let many: Vec<String> = (0..80).map(|i| format!("Term{i:03}")).collect();
        let vocab = DomainVocabulary::new().with_meeting_terms(&many);
        let prev = "and that is why the numbers moved";

        let prompt = vocab
            .build_prompt(None, Some(prev), false, PROMPT_BUDGET_CHARS)
            .unwrap();

        assert!(prompt.ends_with(prev), "prompt was {prompt:?}");
        assert!(prompt.chars().count() <= PROMPT_BUDGET_CHARS);
    }

    /// A rejected segment passes `None`, and then nothing of it reaches the
    /// next decode — which is the whole hallucination quarantine.
    #[test]
    fn no_preceding_text_still_primes_the_vocabulary() {
        let vocab = DomainVocabulary::new();
        let prompt = vocab
            .build_prompt(None, None, false, PROMPT_BUDGET_CHARS)
            .unwrap();
        assert!(prompt.contains("NavGurukul"));
    }

    /// Nothing to say, nothing said: an empty vocabulary with no context must
    /// not produce an empty prompt string, which whisper would still tokenize.
    #[test]
    fn an_empty_vocabulary_with_no_context_produces_no_prompt() {
        let vocab = DomainVocabulary::default();
        assert!(vocab
            .build_prompt(None, Some("   "), false, PROMPT_BUDGET_CHARS)
            .is_none());
    }

    #[test]
    fn test_hindi_devanagari_tail_extraction() {
        let hindi_text = "नमस्ते आप सभी का आज की इस महत्वपूर्ण समीक्षा बैठक में हार्दिक स्वागत है।";
        let tail = extract_recent_tail(hindi_text, 15);
        assert!(!tail.is_empty());
        assert!(hindi_text.ends_with(&tail));
    }
}
