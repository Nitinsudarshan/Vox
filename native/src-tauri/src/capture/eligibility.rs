//! Deterministic decision layer separating acoustic VAD closure from Faithful cleanup eligibility.
//!
//! A segment can close acoustically (e.g. breath, ceiling cut, pause) without representing
//! a complete grammatical thought. Passing an incomplete clause to Faithful causes hallucinations
//! (as measured in Phase 3 where incomplete clauses led to hallucinated completions).

use serde::{Deserialize, Serialize};
use crate::meetings::segmenter::SegmentCloseReason;

/// Verdict on whether a transcript segment is complete and safe for Faithful cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CleanupEligibility {
    /// Safe for Faithful cleanup: completed grammatical utterance, stable boundary.
    Safe,
    /// Unsafe for Faithful: incomplete clause, trailing conjunction, abrupt cut.
    NotSafe { reason: String },
}

impl CleanupEligibility {
    pub fn is_safe(&self) -> bool {
        matches!(self, Self::Safe)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Safe => None,
            Self::NotSafe { reason } => Some(reason),
        }
    }
}

/// Evaluates whether a transcribed segment is safe to send to Faithful.
pub fn evaluate_cleanup_eligibility(
    raw_transcript: &str,
    close_reason: SegmentCloseReason,
    hangover_ms: u64,
) -> CleanupEligibility {
    let text = raw_transcript.trim();

    // 1. Non-empty check
    if text.is_empty() {
        return CleanupEligibility::NotSafe {
            reason: "empty transcript".to_string(),
        };
    }

    // 2. MaxDuration cut check (ceiling split)
    if matches!(close_reason, SegmentCloseReason::MaxDuration) {
        return CleanupEligibility::NotSafe {
            reason: "segment closed at max duration ceiling; sentence continues in next segment".to_string(),
        };
    }

    // 3. Word count check: minimum 3 words for standalone cleanup
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 3 {
        return CleanupEligibility::NotSafe {
            reason: format!("too few words ({}); incomplete phrase", words.len()),
        };
    }

    // 4. Acoustic pause check: if closed by pause, ensure hangover >= 300 ms
    if matches!(close_reason, SegmentCloseReason::Pause) && hangover_ms < 300 {
        return CleanupEligibility::NotSafe {
            reason: format!("acoustic pause hangover too short ({} ms < 300 ms)", hangover_ms),
        };
    }

    // 5. Trailing punctuation check
    let last_word = words.last().copied().unwrap_or("");
    let last_char = text.chars().last().unwrap_or(' ');

    if text.ends_with("...")
        || text.ends_with('…')
        || text.ends_with('-')
        || text.ends_with('—')
        || text.ends_with(',')
    {
        return CleanupEligibility::NotSafe {
            reason: "text ends with trailing pause, ellipsis, comma or dash".to_string(),
        };
    }

    // 6. Trailing dangling conjunctions / prepositions / auxiliaries
    let clean_last = last_word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();

    const DANGLING_WORDS: &[&str] = &[
        "and", "or", "but", "so", "because", "although", "though", "if", "when",
        "that", "which", "who", "whom", "whose", "where",
        "with", "to", "for", "as", "of", "in", "at", "by", "from", "into", "onto",
        "the", "a", "an", "their", "our", "my", "your", "his", "her", "its",
        "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did",
        "will", "would", "shall", "should", "can", "could", "may", "might", "must",
        "then", "also", "like", "such",
    ];

    if DANGLING_WORDS.contains(&clean_last.as_str()) {
        return CleanupEligibility::NotSafe {
            reason: format!("ends with trailing dangling word '{}'", clean_last),
        };
    }

    // 7. Positive terminal punctuation (., ?, !) provides strong confidence of completion
    if last_char == '.' || last_char == '?' || last_char == '!' {
        return CleanupEligibility::Safe;
    }

    // If there is no terminal punctuation, but it passed the dangling checks and has >= 4 words:
    if words.len() >= 4 {
        CleanupEligibility::Safe
    } else {
        CleanupEligibility::NotSafe {
            reason: "short phrase lacking terminal punctuation".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complete_sentence_is_safe() {
        let text = "We have deployed the new service to production.";
        let res = evaluate_cleanup_eligibility(text, SegmentCloseReason::Pause, 400);
        assert!(res.is_safe(), "complete sentence must be safe");
    }

    #[test]
    fn test_incomplete_clause_is_not_safe() {
        let text = "We decided to deploy the new service because";
        let res = evaluate_cleanup_eligibility(text, SegmentCloseReason::Pause, 400);
        assert!(!res.is_safe());
        assert!(res.reason().unwrap().contains("dangling word 'because'"));
    }

    #[test]
    fn test_trailing_comma_or_ellipsis_is_not_safe() {
        let text = "Looking at the dashboard,";
        let res = evaluate_cleanup_eligibility(text, SegmentCloseReason::Pause, 400);
        assert!(!res.is_safe());
        assert!(res.reason().unwrap().contains("comma or dash"));

        let text2 = "Looking at the dashboard...";
        let res2 = evaluate_cleanup_eligibility(text2, SegmentCloseReason::Pause, 400);
        assert!(!res2.is_safe());
        assert!(res2.reason().unwrap().contains("trailing pause"));
    }

    #[test]
    fn test_ceiling_cut_is_never_safe() {
        let text = "The meeting continued with everyone discussing the quarterly roadmap and targets.";
        let res = evaluate_cleanup_eligibility(text, SegmentCloseReason::MaxDuration, 0);
        assert!(!res.is_safe());
        assert!(res.reason().unwrap().contains("ceiling"));
    }

    #[test]
    fn test_short_utterance_is_not_safe() {
        let text = "Yes.";
        let res = evaluate_cleanup_eligibility(text, SegmentCloseReason::Pause, 400);
        assert!(!res.is_safe());
        assert!(res.reason().unwrap().contains("too few words"));
    }

    #[test]
    fn test_empty_transcript() {
        let res = evaluate_cleanup_eligibility("   ", SegmentCloseReason::Pause, 400);
        assert!(!res.is_safe());
        assert_eq!(res.reason().unwrap(), "empty transcript");
    }
}
