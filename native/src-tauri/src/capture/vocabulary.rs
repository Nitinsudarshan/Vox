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
    /// - `max_prompt_chars`: Whisper cpp initial prompt budget (~200 chars to avoid prompt eviction).
    /// - `prev_transcript`: Clean text from preceding segment, if valid and non-hallucinated.
    /// - `is_continuation`: True if current segment immediately continues a forced split.
    pub fn build_prompt(
        &self,
        prev_transcript: Option<&str>,
        is_continuation: bool,
        max_prompt_chars: usize,
    ) -> Option<String> {
        let max_chars = max_prompt_chars.clamp(60, 450);
        let mut parts = Vec::new();

        // 1. If this is a direct continuation of a forced split, prioritize the tail of the previous sentence.
        if let Some(prev) = prev_transcript {
            let cleaned = prev.trim();
            if !cleaned.is_empty() {
                // Extract the last 15-20 words or 120 chars to provide acoustic & semantic continuity
                let tail = extract_recent_tail(cleaned, if is_continuation { 140 } else { 90 });
                if !tail.is_empty() {
                    parts.push(tail);
                }
            }
        }

        // 2. Add domain vocabulary terms up to the remaining character budget.
        let current_len: usize = parts.iter().map(|p| p.len() + 2).sum();
        let vocab_budget = max_chars.saturating_sub(current_len);

        if vocab_budget > 20 {
            let mut vocab_chunk = Vec::new();
            let mut vocab_chars = 0;
            for term in self.active_terms() {
                if vocab_chars + term.len() + 2 > vocab_budget {
                    break;
                }
                vocab_chars += term.len() + 2;
                vocab_chunk.push(term);
            }
            if !vocab_chunk.is_empty() {
                parts.push(vocab_chunk.join(", "));
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(". "))
        }
    }
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
            Some("We were discussing the budget for the upcoming quarter"),
            false,
            200,
        );
        assert!(prompt.is_some());
        let p = prompt.unwrap();
        assert!(p.contains("budget for the upcoming quarter") || p.contains("Sprint Planning"));
    }

    #[test]
    fn test_forced_split_prompt_continuation() {
        let vocab = DomainVocabulary::new();
        let prev = "Actually mujhe lagta hai we should finalize this by Friday because agar numbers kal tak aa jaate";
        let prompt = vocab.build_prompt(Some(prev), true, 220).unwrap();
        assert!(prompt.contains("numbers kal tak aa jaate"));
    }

    #[test]
    fn test_hindi_devanagari_tail_extraction() {
        let hindi_text = "नमस्ते आप सभी का आज की इस महत्वपूर्ण समीक्षा बैठक में हार्दिक स्वागत है।";
        let tail = extract_recent_tail(hindi_text, 15);
        assert!(!tail.is_empty());
        assert!(hindi_text.ends_with(&tail));
    }
}
