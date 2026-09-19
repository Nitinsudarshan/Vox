//! The transcript everything downstream reads, and the line between evidence
//! and interpretation.
//!
//! ## Two layers, and why one file was not enough
//!
//! A [`TranscriptSegment`] is **raw ASR evidence**: what a decoder said about
//! one span of audio, when, on which channel, with what no-speech probability.
//! It is a measurement. Re-running the same model over the same audio should
//! reproduce it, and nothing that is not a decode should change it.
//!
//! A [`CanonicalSegment`] is **derived**: raw evidence ordered, joined where a
//! sentence was split, de-duplicated at the joins, labelled with whoever was
//! speaking, rendered in whichever language the reader asked for. It is an
//! interpretation, and every one of those steps is a decision that could be
//! made differently tomorrow.
//!
//! Collapsing the two is how a transcript becomes unauditable. The audit found
//! `transcript.json` written by five different producers — the live worker,
//! re-transcription, the English pass, romanization and speaker detection — so
//! "what did the model actually say" and "what does Vox believe this means"
//! were the same bytes.
//!
//! ## Provenance is the point
//!
//! Every canonical segment carries [`CanonicalSegment::sources`]: the raw
//! sequence numbers behind it. Two lines joined into one name both. A line
//! that is one raw segment names one. Nothing is derived from nothing, and
//! "why does the report say that" can always be walked back to a span of
//! audio.
//!
//! ## Gaps are stated, not smoothed over
//!
//! Sequence numbers are assigned before a decode, so a dropped or failed
//! segment leaves a hole in the numbering. The assembler reports those holes
//! as [`TranscriptGap`]s instead of closing ranks around them. A transcript
//! that silently omits ninety seconds reads as a complete record of a meeting
//! in which nobody spoke for ninety seconds, which is a different and worse
//! claim than "this part is missing".

use serde::{Deserialize, Serialize};

use super::model::{
    SegmentChannel, Speaker, TranscriptProvenance, TranscriptSegment,
};
use super::speakers::SpeakerAttribution;

/// Longest repeated phrase, in words, the join de-duplicator looks for.
///
/// Whisper conditions on its own output, so a span cut at the segmenter's
/// ceiling often re-decodes the last few words of the previous one. Beyond
/// about a dozen words a repeat is far more likely to be something the
/// speaker actually said twice.
const MAX_JOIN_OVERLAP_WORDS: usize = 12;

/// How close two spans must be, in seconds, to count as adjacent.
///
/// One segmenter frame. A ceiling cut is sample-exact, so anything looser
/// would start joining spans with a real pause between them.
const ADJACENCY_TOLERANCE_SECONDS: f64 = 0.021;

/// One line of the transcript as everything downstream should read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalSegment {
    /// Position in this transcript. Not a raw sequence number — see
    /// [`Self::sources`] for those.
    pub id: u64,
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Which capture channel carried it. Measured, not inferred.
    pub channel: SegmentChannel,
    /// Who Vox believes said it, where detection has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    /// What to call them — a user-given name, else the channel's own label.
    pub speaker_label: String,
    /// The language this rendering is in, where it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The raw sequence numbers this line was built from, in order.
    ///
    /// The whole provenance chain in one field: every claim a report makes
    /// walks back through here to a span of audio.
    pub sources: Vec<u64>,
    /// Whether more than one raw segment was joined into this line.
    pub merged: bool,
}

impl CanonicalSegment {
    pub fn duration_seconds(&self) -> f64 {
        (self.end_seconds - self.start_seconds).max(0.0)
    }
}

/// Speech that was recorded and never transcribed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptGap {
    /// Raw sequence numbers that were assigned and never produced a segment.
    pub missing_sequences: Vec<u64>,
    /// The canonical line before the gap, where there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_segment: Option<u64>,
    /// Where in the recording it falls, bounded by the lines either side.
    /// `None` at the very start or the very end, where only one side is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
}

/// A meeting's transcript, assembled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalTranscript {
    pub meeting_id: String,
    pub built_at: String,
    /// What produced the raw evidence underneath. Carried rather than
    /// re-derived so a canonical transcript is self-describing: the model,
    /// the language and the pass travel with the text they produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TranscriptProvenance>,
    pub segments: Vec<CanonicalSegment>,
    /// Holes in the raw sequence — speech that exists in the recording and
    /// not here.
    #[serde(default)]
    pub gaps: Vec<TranscriptGap>,
    /// Raw segments that went in. `segments.len()` will be smaller wherever
    /// lines were joined.
    pub raw_segment_count: usize,
}

impl CanonicalTranscript {
    /// Whether any recorded speech is missing from this transcript.
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }

    /// Raw sequences that produced no line.
    pub fn missing_sequences(&self) -> Vec<u64> {
        self.gaps
            .iter()
            .flat_map(|gap| gap.missing_sequences.iter().copied())
            .collect()
    }
}

/// Which rendering of a line to prefer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rendering {
    /// The decode as it came out.
    #[default]
    Original,
    /// English where an English pass produced it, the original otherwise.
    ///
    /// Falls back rather than omitting: a line with no translation is still
    /// something somebody said.
    PreferTranslated,
    /// Latin script where romanization produced it, the original otherwise.
    PreferRomanized,
}

/// How to assemble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyOptions {
    pub rendering: Rendering,
    /// Join raw segments the segmenter cut at its ceiling.
    ///
    /// On by default: a sentence split across two lines because a decoder
    /// window ran out is an artifact of decoding, not of the conversation, and
    /// a summarizer reading it as two turns draws the wrong shape.
    pub merge_ceiling_cuts: bool,
    /// Strip a repeated phrase where two joined lines overlap.
    pub remove_join_overlap: bool,
}

impl Default for AssemblyOptions {
    fn default() -> Self {
        Self {
            rendering: Rendering::Original,
            merge_ceiling_cuts: true,
            remove_join_overlap: true,
        }
    }
}

/// Turns raw ASR evidence into the transcript everything downstream reads.
///
/// A free function rather than a struct with state: assembly is a pure
/// function of its inputs, and keeping it that way is what makes it testable
/// against a hand-written raw transcript and what makes the result
/// reproducible.
///
/// **The raw segments are borrowed and never modified.** That is the rule the
/// whole module exists to enforce.
pub fn assemble(
    meeting_id: &str,
    raw: &[TranscriptSegment],
    attribution: &SpeakerAttribution,
    speakers: &[Speaker],
    source: Option<TranscriptProvenance>,
    options: &AssemblyOptions,
) -> CanonicalTranscript {
    // Ordered and de-duplicated by sequence. The store already does this on
    // read; doing it again costs nothing and means the assembler is correct
    // for a caller that built a list some other way.
    let mut ordered: Vec<&TranscriptSegment> = raw.iter().collect();
    ordered.sort_by_key(|segment| segment.sequence);
    ordered.dedup_by_key(|segment| segment.sequence);

    let mut segments: Vec<CanonicalSegment> = Vec::with_capacity(ordered.len());
    for segment in &ordered {
        let text = render(segment, options.rendering);
        if text.trim().is_empty() {
            continue;
        }
        let speaker_id = attribution
            .speaker_for(segment.sequence)
            .map(str::to_string);
        let candidate = CanonicalSegment {
            id: segments.len() as u64,
            text,
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            channel: segment.channel,
            speaker_label: label_for(segment, speaker_id.as_deref(), speakers),
            speaker_id,
            language: language_of(segment, options.rendering),
            sources: vec![segment.sequence],
            merged: false,
        };

        match segments.last_mut() {
            Some(previous)
                if options.merge_ceiling_cuts
                    && continues(&ordered, previous, &candidate) =>
            {
                join(previous, candidate, options.remove_join_overlap);
            }
            _ => segments.push(candidate),
        }
    }

    // Renumber after merging, so ids are contiguous in the transcript a
    // reader sees. Raw sequences are unchanged and still in `sources`.
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.id = index as u64;
    }

    CanonicalTranscript {
        meeting_id: meeting_id.to_string(),
        built_at: chrono::Utc::now().to_rfc3339(),
        source,
        gaps: find_gaps(&ordered, &segments),
        raw_segment_count: ordered.len(),
        segments,
    }
}

/// Whether `candidate` continues the line before it.
///
/// Three conditions, all necessary. The previous raw segment has to have been
/// cut at the segmenter's ceiling — otherwise the gap between them is a real
/// pause. They have to be adjacent in time, so a dropped segment between them
/// does not get papered over. And they have to be the same voice, because two
/// people talking in sequence is a turn change however the decoder windowed
/// it.
fn continues(
    ordered: &[&TranscriptSegment],
    previous: &CanonicalSegment,
    candidate: &CanonicalSegment,
) -> bool {
    let Some(last_source) = previous.sources.last() else {
        return false;
    };
    let cut_at_ceiling = ordered
        .iter()
        .find(|segment| segment.sequence == *last_source)
        .is_some_and(|segment| segment.cut_at_ceiling);

    cut_at_ceiling
        && (candidate.start_seconds - previous.end_seconds).abs() <= ADJACENCY_TOLERANCE_SECONDS
        && candidate.channel == previous.channel
        && candidate.speaker_id == previous.speaker_id
}

/// Appends `candidate` to `previous`, dropping a repeated phrase at the seam.
fn join(previous: &mut CanonicalSegment, candidate: CanonicalSegment, remove_overlap: bool) {
    let addition = if remove_overlap {
        strip_leading_overlap(&previous.text, &candidate.text)
    } else {
        candidate.text.clone()
    };
    if !addition.trim().is_empty() {
        if !previous.text.ends_with(' ') && !addition.starts_with(' ') {
            previous.text.push(' ');
        }
        previous.text.push_str(addition.trim_start());
    }
    previous.end_seconds = candidate.end_seconds;
    previous.sources.extend(candidate.sources);
    previous.merged = true;
}

/// Removes from `next` whatever repeats the tail of `previous`.
///
/// Whisper conditions on its own output, so a span cut mid-sentence often
/// re-decodes the last few words of the one before it. Longest match wins, and
/// the comparison ignores case and punctuation because the second decode
/// rarely punctuates the repeat the same way.
pub fn strip_leading_overlap(previous: &str, next: &str) -> String {
    let previous_words: Vec<&str> = previous.split_whitespace().collect();
    let next_words: Vec<&str> = next.split_whitespace().collect();
    let most = MAX_JOIN_OVERLAP_WORDS.min(previous_words.len()).min(next_words.len());

    for length in (1..=most).rev() {
        let tail = &previous_words[previous_words.len() - length..];
        let head = &next_words[..length];
        if tail
            .iter()
            .zip(head.iter())
            .all(|(a, b)| normalize_word(a) == normalize_word(b))
        {
            return next_words[length..].join(" ");
        }
    }
    next.to_string()
}

fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Sequence numbers that were assigned and never produced a line.
///
/// A gap is speech the recording has and the transcript does not. Reporting it
/// is the difference between "this part is missing" and a transcript that
/// reads as a complete record of a meeting where nobody spoke.
fn find_gaps(ordered: &[&TranscriptSegment], segments: &[CanonicalSegment]) -> Vec<TranscriptGap> {
    if ordered.is_empty() {
        return Vec::new();
    }
    let present: std::collections::BTreeSet<u64> =
        ordered.iter().map(|segment| segment.sequence).collect();
    let first = *present.iter().next().expect("non-empty");
    let last = *present.iter().next_back().expect("non-empty");

    let mut gaps = Vec::new();
    let mut run: Vec<u64> = Vec::new();
    for sequence in first..=last {
        if present.contains(&sequence) {
            if !run.is_empty() {
                gaps.push(gap_for(std::mem::take(&mut run), segments));
            }
        } else {
            run.push(sequence);
        }
    }
    if !run.is_empty() {
        gaps.push(gap_for(run, segments));
    }
    gaps
}

fn gap_for(missing: Vec<u64>, segments: &[CanonicalSegment]) -> TranscriptGap {
    let before = segments
        .iter()
        .rfind(|segment| segment.sources.iter().any(|s| *s < missing[0]));
    let after = segments
        .iter()
        .find(|segment| segment.sources.iter().any(|s| *s > *missing.last().unwrap()));
    TranscriptGap {
        after_segment: before.map(|segment| segment.id),
        start_seconds: before.map(|segment| segment.end_seconds),
        end_seconds: after.map(|segment| segment.start_seconds),
        missing_sequences: missing,
    }
}

/// The text for a rendering, falling back to the original rather than to
/// nothing: a line with no translation is still something somebody said.
fn render(segment: &TranscriptSegment, rendering: Rendering) -> String {
    match rendering {
        Rendering::Original => segment.text.clone(),
        Rendering::PreferTranslated => segment
            .translated_text
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| segment.text.clone()),
        Rendering::PreferRomanized => segment
            .romanized_text
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| segment.text.clone()),
    }
}

/// The language of the rendering actually chosen, not of the meeting.
///
/// A translated line is English whatever the meeting was, and saying so is
/// what stops a downstream reader treating a mixed transcript as monolingual.
fn language_of(segment: &TranscriptSegment, rendering: Rendering) -> Option<String> {
    match rendering {
        Rendering::PreferTranslated
            if segment
                .translated_text
                .as_ref()
                .is_some_and(|text| !text.trim().is_empty()) =>
        {
            Some("en".to_string())
        }
        _ => None,
    }
}

/// What to call whoever said this line.
///
/// A user-given name wins. Otherwise the channel's own label, which is
/// measured rather than inferred and is never wrong about which side of the
/// call someone was on.
fn label_for(
    segment: &TranscriptSegment,
    speaker_id: Option<&str>,
    speakers: &[Speaker],
) -> String {
    speaker_id
        .and_then(|id| speakers.iter().find(|speaker| speaker.id == id))
        .map(|speaker| speaker.label.clone())
        .unwrap_or_else(|| segment.channel.label().to_string())
}

/// Renders a canonical transcript for a reader or a model.
///
/// One function, so the text a summarizer sees and the text a person sees come
/// from the same place. A gap is marked rather than elided: a model that reads
/// straight past ninety missing seconds will summarize a conversation that did
/// not happen.
pub fn render_transcript(transcript: &CanonicalTranscript) -> String {
    let mut out = String::new();
    let mut last_label: Option<&str> = None;
    let mut gaps = transcript.gaps.iter().peekable();

    for segment in &transcript.segments {
        while let Some(gap) = gaps.peek() {
            if gap.after_segment == Some(segment.id.saturating_sub(1)) && segment.id > 0 {
                out.push_str(&format!(
                    "[{} segment(s) of speech could not be transcribed]\n",
                    gap.missing_sequences.len()
                ));
                gaps.next();
            } else {
                break;
            }
        }
        if last_label != Some(segment.speaker_label.as_str()) {
            out.push_str(&format!("\n{}:\n", segment.speaker_label));
            last_label = Some(segment.speaker_label.as_str());
        }
        out.push_str(&format!(
            "[{}] {}\n",
            format_timestamp(segment.start_seconds),
            segment.text.trim()
        ));
    }
    out
}

fn format_timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(sequence: u64, text: &str, start: f64, end: f64) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: text.to_string(),
            start_seconds: start,
            end_seconds: end,
            channel: SegmentChannel::Microphone,
            no_speech_prob: 0.05,
            recorded_at: "2026-09-19T10:00:00Z".into(),
            cut_at_ceiling: false,
            original_text: None,
            romanized_text: None,
            translated_text: None,
            corrections: Vec::new(),
        }
    }

    fn build(raw_segments: &[TranscriptSegment]) -> CanonicalTranscript {
        assemble(
            "m",
            raw_segments,
            &SpeakerAttribution::default(),
            &[],
            None,
            &AssemblyOptions::default(),
        )
    }

    /// Attribution for a set of (raw sequence, speaker id) pairs.
    fn attributed(pairs: &[(u64, &str)]) -> SpeakerAttribution {
        SpeakerAttribution {
            by_sequence: pairs
                .iter()
                .map(|(sequence, id)| (*sequence, (*id).to_string()))
                .collect(),
        }
    }

    #[test]
    fn assembling_never_touches_the_raw_evidence() {
        // The rule the whole module exists for.
        let original = vec![raw(0, "hello", 0.0, 1.0), raw(1, "world", 2.0, 3.0)];
        let before = original.clone();
        let _ = build(&original);
        assert_eq!(original, before);
    }

    #[test]
    fn every_line_names_the_raw_segments_behind_it() {
        let transcript = build(&[raw(4, "hello", 0.0, 1.0), raw(5, "world", 2.0, 3.0)]);
        assert_eq!(transcript.segments[0].sources, vec![4]);
        assert_eq!(transcript.segments[1].sources, vec![5]);
        // Canonical ids are contiguous; raw sequences are whatever they were.
        assert_eq!(transcript.segments[0].id, 0);
        assert_eq!(transcript.segments[1].id, 1);
    }

    #[test]
    fn segments_are_ordered_by_sequence_whatever_order_they_arrive_in() {
        let transcript = build(&[
            raw(2, "third", 4.0, 5.0),
            raw(0, "first", 0.0, 1.0),
            raw(1, "second", 2.0, 3.0),
        ]);
        let texts: Vec<&str> = transcript
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn a_duplicate_sequence_produces_one_line() {
        let transcript = build(&[raw(0, "hello", 0.0, 1.0), raw(0, "hello", 0.0, 1.0)]);
        assert_eq!(transcript.segments.len(), 1);
    }

    #[test]
    fn a_sentence_cut_at_the_ceiling_is_put_back_together() {
        // The decoder's window ran out mid-sentence. That is an artifact of
        // decoding, and a summarizer reading it as two turns draws the wrong
        // shape of conversation.
        let mut first = raw(0, "we should ship the release", 0.0, 25.0);
        first.cut_at_ceiling = true;
        let second = raw(1, "on Thursday", 25.0, 27.0);

        let transcript = build(&[first, second]);
        assert_eq!(transcript.segments.len(), 1);
        assert_eq!(transcript.segments[0].text, "we should ship the release on Thursday");
        assert_eq!(transcript.segments[0].sources, vec![0, 1]);
        assert!(transcript.segments[0].merged);
        assert_eq!(transcript.segments[0].end_seconds, 27.0);
        assert_eq!(transcript.raw_segment_count, 2);
    }

    #[test]
    fn a_phrase_repeated_across_a_join_is_not_transcribed_twice() {
        // Whisper conditions on its own output, so the span after a ceiling
        // cut often re-decodes the tail of the one before it.
        let mut first = raw(0, "and then we agreed to ship it", 0.0, 25.0);
        first.cut_at_ceiling = true;
        let second = raw(1, "to ship it on Thursday", 25.0, 28.0);

        let transcript = build(&[first, second]);
        assert_eq!(
            transcript.segments[0].text,
            "and then we agreed to ship it on Thursday"
        );
    }

    #[test]
    fn overlap_removal_matches_past_casing_and_punctuation() {
        // The second decode rarely punctuates the repeat the same way.
        assert_eq!(strip_leading_overlap("we should ship it.", "Ship it, on Thursday"), "on Thursday");
        assert_eq!(strip_leading_overlap("hello there", "entirely new"), "entirely new");
        assert_eq!(strip_leading_overlap("", "anything"), "anything");
    }

    #[test]
    fn a_real_pause_between_spans_is_not_joined_even_after_a_ceiling_cut() {
        // A ceiling cut is sample-exact. A gap means a segment went missing
        // between them, and papering over it would hide the loss.
        let mut first = raw(0, "we should ship", 0.0, 25.0);
        first.cut_at_ceiling = true;
        let second = raw(1, "on Thursday", 40.0, 42.0);

        let transcript = build(&[first, second]);
        assert_eq!(transcript.segments.len(), 2, "not adjacent, so not one sentence");
    }

    #[test]
    fn a_turn_change_is_never_merged_even_across_a_ceiling_cut() {
        let mut first = raw(0, "we should ship", 0.0, 25.0);
        first.cut_at_ceiling = true;
        let mut second = raw(1, "I disagree", 25.0, 27.0);
        second.channel = SegmentChannel::System;

        let transcript = build(&[first, second]);
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0].speaker_label, "You");
        assert_eq!(transcript.segments[1].speaker_label, "Others");
    }

    #[test]
    fn two_different_speakers_are_not_merged_even_on_one_channel() {
        let mut first = raw(0, "we should ship", 0.0, 25.0);
        first.cut_at_ceiling = true;
        let second = raw(1, "I disagree", 25.0, 27.0);
        let transcript = assemble(
            "m",
            &[first, second],
            &attributed(&[(0, "speaker-1"), (1, "speaker-2")]),
            &[],
            None,
            &AssemblyOptions::default(),
        );
        assert_eq!(transcript.segments.len(), 2);
    }

    #[test]
    fn a_dropped_segment_leaves_a_gap_that_is_reported() {
        // Sequences are assigned before decoding, so a hole in the numbering
        // is speech the recording has and the transcript does not.
        let transcript = build(&[
            raw(0, "before", 0.0, 2.0),
            raw(3, "after", 30.0, 32.0),
        ]);
        assert!(!transcript.is_complete());
        assert_eq!(transcript.missing_sequences(), vec![1, 2]);

        let gap = &transcript.gaps[0];
        assert_eq!(gap.after_segment, Some(0));
        assert_eq!(gap.start_seconds, Some(2.0));
        assert_eq!(gap.end_seconds, Some(30.0));
    }

    #[test]
    fn an_unbroken_transcript_reports_no_gaps() {
        let transcript = build(&[raw(0, "a", 0.0, 1.0), raw(1, "b", 1.0, 2.0)]);
        assert!(transcript.is_complete());
        assert!(transcript.missing_sequences().is_empty());
    }

    #[test]
    fn a_rendered_transcript_marks_the_gap_rather_than_reading_past_it() {
        // A model that reads straight past ninety missing seconds will
        // summarize a conversation that did not happen.
        let transcript = build(&[raw(0, "before", 0.0, 2.0), raw(4, "after", 90.0, 92.0)]);
        let rendered = render_transcript(&transcript);
        assert!(rendered.contains("could not be transcribed"));
        assert!(rendered.contains("before"));
        assert!(rendered.contains("after"));
    }

    #[test]
    fn a_user_given_name_replaces_the_channel_label() {
        let segment = raw(0, "hello", 0.0, 1.0);
        let speakers = vec![Speaker {
            id: "speaker-1".into(),
            label: "Payal".into(),
            named_by_user: true,
            channel: SegmentChannel::System,
            sample_start_seconds: 0.0,
            sample_end_seconds: 4.0,
            segment_count: 1,
            speaking_seconds: 1.0,
        }];
        let transcript = assemble(
            "m",
            &[segment],
            &attributed(&[(0, "speaker-1")]),
            &speakers,
            None,
            &AssemblyOptions::default(),
        );
        assert_eq!(transcript.segments[0].speaker_label, "Payal");
        assert_eq!(transcript.segments[0].speaker_id.as_deref(), Some("speaker-1"));
    }

    #[test]
    fn an_unattributed_line_falls_back_to_the_channel_which_is_measured() {
        let transcript = build(&[raw(0, "hello", 0.0, 1.0)]);
        assert_eq!(transcript.segments[0].speaker_label, "You");
        assert!(transcript.segments[0].speaker_id.is_none());
    }

    #[test]
    fn a_translated_rendering_says_it_is_english() {
        let mut segment = raw(0, "हम कल भेजेंगे", 0.0, 2.0);
        segment.translated_text = Some("we will send it tomorrow".into());
        let transcript = assemble(
            "m",
            &[segment],
            &SpeakerAttribution::default(),
            &[],
            None,
            &AssemblyOptions {
                rendering: Rendering::PreferTranslated,
                ..AssemblyOptions::default()
            },
        );
        assert_eq!(transcript.segments[0].text, "we will send it tomorrow");
        assert_eq!(transcript.segments[0].language.as_deref(), Some("en"));
    }

    #[test]
    fn a_line_with_no_translation_keeps_its_own_words_rather_than_vanishing() {
        let transcript = assemble(
            "m",
            &[raw(0, "still something somebody said", 0.0, 2.0)],
            &SpeakerAttribution::default(),
            &[],
            None,
            &AssemblyOptions {
                rendering: Rendering::PreferTranslated,
                ..AssemblyOptions::default()
            },
        );
        assert_eq!(transcript.segments[0].text, "still something somebody said");
        assert_eq!(transcript.segments[0].language, None);
    }

    #[test]
    fn an_empty_decode_produces_no_line_and_no_gap() {
        // A screened-out hallucination is not missing speech — there was
        // nothing there. It is discarded before it reaches a sequence.
        let transcript = build(&[raw(0, "real", 0.0, 1.0), raw(1, "   ", 1.0, 2.0)]);
        assert_eq!(transcript.segments.len(), 1);
    }

    #[test]
    fn a_retranscription_produces_a_transcript_that_says_what_made_it() {
        let source = TranscriptProvenance {
            pass: super::super::model::TranscriptionPass::Final,
            engine: "whisper".into(),
            model: "ggml-large-v3-turbo.bin".into(),
            language: Some("hi".into()),
            profile: "BeamSearch".into(),
            completed_at: "2026-09-19T10:00:00Z".into(),
        };
        let transcript = assemble(
            "m",
            &[raw(0, "hello", 0.0, 1.0)],
            &SpeakerAttribution::default(),
            &[],
            Some(source.clone()),
            &AssemblyOptions::default(),
        );
        assert_eq!(transcript.source, Some(source));
    }

    #[test]
    fn an_empty_transcript_assembles_to_an_empty_one_rather_than_failing() {
        let transcript = build(&[]);
        assert!(transcript.segments.is_empty());
        assert!(transcript.is_complete());
        assert_eq!(transcript.raw_segment_count, 0);
    }

    #[test]
    fn a_corrected_line_can_still_be_read_as_the_decoder_said_it() {
        // "Preserve the raw ASR text" as a property of the data. The
        // corrections are the difference, so applying them backwards gives
        // back exactly what came out of the model.
        use crate::capture::glossary::{uncorrect, CorrectionReason, TermCategory, TermCorrection};

        let mut segment = raw(0, "we use Supabase", 0.0, 2.0);
        segment.corrections = vec![TermCorrection {
            from: "supabse".into(),
            to: "Supabase".into(),
            term: "Supabase".into(),
            category: TermCategory::Product,
            reason: CorrectionReason::NearMiss,
        }];
        assert_eq!(
            uncorrect(&segment.text, &segment.corrections),
            "we use supabse"
        );
    }

    #[test]
    fn a_change_to_the_raw_evidence_propagates_to_what_downstream_reads() {
        // The contract in one direction: intelligence is downstream of the
        // transcript, so re-decoding a line has to change what a report would
        // be written from.
        let before = build(&[raw(0, "we will ship on Thursday", 0.0, 3.0)]);
        let after = build(&[raw(0, "we will ship on Tuesday", 0.0, 3.0)]);

        assert_ne!(before.segments[0].text, after.segments[0].text);
        assert_ne!(render_transcript(&before), render_transcript(&after));
    }

    #[test]
    fn renaming_a_speaker_changes_the_rendering_and_not_the_evidence() {
        // The contract in the other direction, and the one that is easy to
        // get wrong: a name is the user's word about a voice, not a claim
        // about what was said.
        let segments = [raw(0, "we will ship on Thursday", 0.0, 3.0)];
        let before = segments.to_vec();
        let attribution = attributed(&[(0, "speaker-1")]);

        let speaker = |label: &str| {
            vec![Speaker {
                id: "speaker-1".into(),
                label: label.into(),
                named_by_user: true,
                channel: SegmentChannel::Microphone,
                sample_start_seconds: 0.0,
                sample_end_seconds: 3.0,
                segment_count: 1,
                speaking_seconds: 3.0,
            }]
        };

        let anonymous = assemble(
            "m",
            &segments,
            &attribution,
            &speaker("Speaker 1"),
            None,
            &AssemblyOptions::default(),
        );
        let named = assemble(
            "m",
            &segments,
            &attribution,
            &speaker("Payal"),
            None,
            &AssemblyOptions::default(),
        );

        assert_eq!(segments.to_vec(), before, "the raw evidence is untouched");
        assert_eq!(
            anonymous.segments[0].text, named.segments[0].text,
            "and so are the words"
        );
        assert_eq!(anonymous.segments[0].speaker_label, "Speaker 1");
        assert_eq!(named.segments[0].speaker_label, "Payal");
        assert!(render_transcript(&named).contains("Payal"));
    }

    #[test]
    fn a_canonical_transcript_round_trips_through_json() {
        let transcript = build(&[raw(0, "hello", 0.0, 1.0)]);
        let json = serde_json::to_string(&transcript).expect("serialize");
        let back: CanonicalTranscript = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(transcript, back);
    }
}
