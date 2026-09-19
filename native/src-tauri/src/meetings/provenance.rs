//! Tracing a claim about a meeting back to the audio behind it.
//!
//! ## The question this exists to answer
//!
//! *Why did Vox think this was an action item?*
//!
//! Without an answer, meeting intelligence is a set of assertions with no
//! standing: a user who disagrees with one has nowhere to look, and a user who
//! agrees has no reason to. The chain that answers it is short and every link
//! already exists —
//!
//! ```text
//! claim  ─▶  canonical segment(s)  ─▶  raw ASR segment(s)  ─▶  span of audio
//! ```
//!
//! — and what was missing was a type that carries it.
//!
//! ## What it will not do
//!
//! Guess. [`Evidence::locate`] matches a claim's quoted text against the
//! transcript, and returns nothing when it cannot find it. A claim with no
//! evidence is reported as a claim with no evidence, not attached to whichever
//! line was most similar. An approximate provenance is worse than none: it
//! reads exactly like a real one, and the whole point is to be checkable.
//!
//! ## Nothing calls this yet
//!
//! The types and the matcher exist and are tested; **no shipped path produces
//! an [`Attributed<T>`]**. A report's action items, decisions and questions
//! come back from the model as plain text and are stored as plain text, so
//! today a user still cannot ask a claim where it came from.
//!
//! What a report *does* record is the transcript it was written from — which
//! pass produced it, how many lines, how many segments of recorded speech were
//! missing (`summary.json`, D-037). That is provenance for the report as a
//! whole, and it is live. Per-claim provenance is this module, and it is not.
//!
//! The remaining work is in the summary pipeline, not here: the structured
//! output would have to carry the quoted span each claim rests on, and
//! `MeetingSummary` would have to store the `Attributed` wrapper instead of
//! the bare claim.
//!
//! TODO(provenance): wire `Evidence::locate` into `summary::service` so that
//! the claims in a report carry evidence, and say which ones have none.

use serde::{Deserialize, Serialize};

use super::canonical::{CanonicalSegment, CanonicalTranscript};

/// Shortest quoted phrase that may anchor a claim.
///
/// Three words. Below that a match says almost nothing — "the release" occurs
/// in half the lines of a meeting about a release — and a citation that points
/// at the wrong line is worse than one that points nowhere.
const MIN_ANCHOR_WORDS: usize = 3;

/// Where a claim about a meeting came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub meeting_id: String,
    /// Canonical lines supporting the claim.
    pub segment_ids: Vec<u64>,
    /// Raw ASR sequences underneath those lines — the audio-facing end of the
    /// chain, and what a player needs to seek to.
    pub source_sequences: Vec<u64>,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl Evidence {
    pub fn duration_seconds(&self) -> f64 {
        (self.end_seconds - self.start_seconds).max(0.0)
    }

    /// Builds evidence from lines already identified.
    pub fn from_segments(meeting_id: &str, segments: &[&CanonicalSegment]) -> Option<Self> {
        if segments.is_empty() {
            return None;
        }
        let start = segments
            .iter()
            .map(|segment| segment.start_seconds)
            .fold(f64::INFINITY, f64::min);
        let end = segments
            .iter()
            .map(|segment| segment.end_seconds)
            .fold(f64::NEG_INFINITY, f64::max);
        Some(Self {
            meeting_id: meeting_id.to_string(),
            segment_ids: segments.iter().map(|segment| segment.id).collect(),
            source_sequences: segments
                .iter()
                .flat_map(|segment| segment.sources.iter().copied())
                .collect(),
            start_seconds: start,
            end_seconds: end,
        })
    }

    /// Finds the lines a quoted phrase came from.
    ///
    /// Returns `None` rather than a guess when the phrase is too short to
    /// anchor anything, or when no line contains it. A model asked to quote
    /// the transcript usually does; when it paraphrases instead, the honest
    /// answer is that this claim cannot be traced, and that is a fact worth
    /// showing a user rather than papering over.
    pub fn locate(transcript: &CanonicalTranscript, quote: &str) -> Option<Self> {
        let needle = normalize(quote);
        if needle.split(' ').filter(|w| !w.is_empty()).count() < MIN_ANCHOR_WORDS {
            return None;
        }
        let matches: Vec<&CanonicalSegment> = transcript
            .segments
            .iter()
            .filter(|segment| normalize(&segment.text).contains(&needle))
            .collect();
        Self::from_segments(&transcript.meeting_id, &matches)
    }
}

/// One thing a model said about a meeting, and what supports it.
///
/// Generic over the claim so the same shape serves a decision, an action, a
/// question and a knowledge-graph edge. They differ in what they assert and
/// not at all in how they are evidenced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attributed<T> {
    pub claim: T,
    /// `None` where the claim could not be traced to a line. Present and
    /// honest, rather than absent and assumed.
    #[serde(default = "none")]
    pub evidence: Option<Evidence>,
}

fn none<T>() -> Option<T> {
    None
}

impl<T> Attributed<T> {
    /// A claim with evidence located by its own quoted text.
    pub fn located(claim: T, transcript: &CanonicalTranscript, quote: &str) -> Self {
        Self {
            evidence: Evidence::locate(transcript, quote),
            claim,
        }
    }

    /// A claim nothing in the transcript supports.
    pub fn unsupported(claim: T) -> Self {
        Self {
            claim,
            evidence: None,
        }
    }

    pub fn is_traceable(&self) -> bool {
        self.evidence.is_some()
    }
}

/// Lowercased, punctuation stripped, whitespace collapsed.
///
/// A model quoting a transcript reproduces the words and rarely the
/// punctuation, so matching on the raw text would fail on a comma.
fn normalize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::canonical::{assemble, AssemblyOptions};
    use crate::meetings::model::{SegmentChannel, TranscriptSegment};
    use crate::meetings::speakers::SpeakerAttribution;

    fn raw(sequence: u64, text: &str, start: f64, end: f64) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: text.to_string(),
            start_seconds: start,
            end_seconds: end,
            channel: SegmentChannel::System,
            no_speech_prob: 0.02,
            recorded_at: "2026-09-19T10:00:00Z".into(),
            cut_at_ceiling: false,
            original_text: None,
            romanized_text: None,
            translated_text: None,
            corrections: Vec::new(),
        }
    }

    fn transcript() -> CanonicalTranscript {
        assemble(
            "meeting-1",
            &[
                raw(0, "Payal will send the deck on Thursday.", 10.0, 14.0),
                raw(1, "We should ship the release next week.", 20.0, 24.0),
                raw(2, "Anything else?", 30.0, 31.0),
            ],
            &SpeakerAttribution::default(),
            &[],
            None,
            &AssemblyOptions::default(),
        )
    }

    #[test]
    fn a_quoted_claim_traces_back_to_the_line_and_the_audio_behind_it() {
        // The whole chain in one assertion: claim → canonical line → raw
        // sequence → span of the recording.
        let evidence = Evidence::locate(&transcript(), "send the deck on Thursday")
            .expect("the transcript says this");
        assert_eq!(evidence.meeting_id, "meeting-1");
        assert_eq!(evidence.segment_ids, vec![0]);
        assert_eq!(evidence.source_sequences, vec![0]);
        assert_eq!(evidence.start_seconds, 10.0);
        assert_eq!(evidence.end_seconds, 14.0);
    }

    #[test]
    fn punctuation_a_model_did_not_reproduce_does_not_break_the_trace() {
        // A model quoting a transcript reproduces the words and rarely the
        // commas.
        assert!(Evidence::locate(&transcript(), "ship the release, next week").is_some());
        assert!(Evidence::locate(&transcript(), "SHIP THE RELEASE next week").is_some());
    }

    #[test]
    fn a_claim_the_transcript_does_not_support_gets_no_evidence() {
        // The failure this module exists to prevent: attaching a citation to
        // whichever line was most similar. An approximate provenance reads
        // exactly like a real one.
        assert!(Evidence::locate(&transcript(), "we agreed to cancel the project").is_none());
    }

    #[test]
    fn a_phrase_too_short_to_identify_anything_anchors_nothing() {
        // "the release" occurs in half the lines of a meeting about a release.
        assert!(Evidence::locate(&transcript(), "the release").is_none());
        assert!(Evidence::locate(&transcript(), "").is_none());
    }

    #[test]
    fn a_claim_spanning_two_lines_names_both_and_the_span_covering_them() {
        let transcript = transcript();
        let both: Vec<&CanonicalSegment> = transcript.segments.iter().take(2).collect();
        let evidence = Evidence::from_segments("meeting-1", &both).expect("two lines");
        assert_eq!(evidence.segment_ids, vec![0, 1]);
        assert_eq!(evidence.start_seconds, 10.0);
        assert_eq!(evidence.end_seconds, 24.0);
        assert_eq!(evidence.duration_seconds(), 14.0);
    }

    #[test]
    fn a_merged_line_names_every_raw_segment_underneath_it() {
        // A sentence the decoder's window cut in half is one canonical line
        // and two spans of audio, and a citation has to reach both.
        let mut first = raw(0, "we should ship the release", 0.0, 25.0);
        first.cut_at_ceiling = true;
        let second = raw(1, "on Thursday", 25.0, 27.0);
        let transcript = assemble(
            "meeting-1",
            &[first, second],
            &SpeakerAttribution::default(),
            &[],
            None,
            &AssemblyOptions::default(),
        );

        let evidence = Evidence::locate(&transcript, "ship the release on Thursday")
            .expect("the joined line says this");
        assert_eq!(evidence.segment_ids, vec![0], "one canonical line");
        assert_eq!(evidence.source_sequences, vec![0, 1], "two spans of audio");
    }

    #[test]
    fn an_untraceable_claim_says_so_rather_than_being_dropped() {
        // A claim nothing supports is still a claim the model made, and
        // hiding it would make the intelligence look better than it is.
        let claim = Attributed::located(
            "Ship the release".to_string(),
            &transcript(),
            "we agreed to cancel the project",
        );
        assert!(!claim.is_traceable());
        assert_eq!(claim.claim, "Ship the release");

        let supported = Attributed::located(
            "Payal sends the deck".to_string(),
            &transcript(),
            "send the deck on Thursday",
        );
        assert!(supported.is_traceable());
    }

    #[test]
    fn evidence_round_trips_so_it_can_be_stored_beside_what_it_supports() {
        let claim = Attributed::unsupported("a thing".to_string());
        let json = serde_json::to_string(&claim).expect("serialize");
        let back: Attributed<String> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(claim, back);

        let located = Attributed::located(
            "a thing".to_string(),
            &transcript(),
            "send the deck on Thursday",
        );
        let json = serde_json::to_string(&located).expect("serialize");
        let back: Attributed<String> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(located, back);
    }
}
