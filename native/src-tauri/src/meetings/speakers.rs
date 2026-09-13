//! Working out who was speaking, and giving the user what they need to say so.
//!
//! # The shape of the answer
//!
//! Vox already knows, for free and exactly, which *side* of a call a line came
//! from: the microphone and the system loopback are two streams, so "you" and
//! "the far end" is measurement, not inference (see
//! [`crate::meetings::model::SegmentChannel`]). What it cannot know that way is
//! which of three people on the far end was talking.
//!
//! This module answers that by fingerprinting each turn
//! ([`crate::meetings::voiceprint`]) and clustering the fingerprints. The
//! technique is honest-but-limited — MFCC statistics, not a trained speaker
//! encoder — so the output is framed as a proposal: each group gets a span of
//! audio where that voice is talking alone, the UI plays it, and the user puts
//! a name to it. That flow is correct whether the grouping was right or wrong,
//! and it survives the embeddings getting better later.
//!
//! # What is never overwritten
//!
//! A name the user typed. Re-running detection after a re-transcription
//! renumbers Vox's own placeholders freely and carries every user-supplied
//! name across by position in the speaking order, because the alternative —
//! asking someone to name the same four people again because they improved
//! their transcript — is how a feature stops being used.

use serde::{Deserialize, Serialize};

use super::model::{Speaker, SegmentChannel, TranscriptSegment};
use super::voiceprint::{self, TurnPrint};

/// Everything a detection run needs to know that is not the audio.
#[derive(Debug, Clone)]
pub struct DetectionSettings {
    /// Cosine distance beyond which two turns are taken to be two people.
    pub split_distance: f32,
    /// Most groups to propose.
    pub max_speakers: usize,
}

impl Default for DetectionSettings {
    fn default() -> Self {
        Self {
            split_distance: voiceprint::DEFAULT_SPLIT_DISTANCE,
            max_speakers: voiceprint::MAX_SPEAKERS,
        }
    }
}

/// What a detection run found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpeakerReport {
    pub speakers: Vec<Speaker>,
    /// Lines that were attributed to somebody.
    pub attributed: usize,
    /// Lines left unattributed — too short to fingerprint, or silent.
    pub unattributed: usize,
}

/// Groups turns by speaker and writes the attribution onto the transcript.
///
/// `prints` carries a fingerprint for every line long enough to have one. A
/// line with no fingerprint keeps `speaker_id: None`: attributing it would
/// mean guessing from the audio's length rather than its content, and an
/// unattributed line reads as unknown while a wrongly attributed one reads as
/// known and wrong.
///
/// The two capture channels are clustered **separately**. The local microphone
/// is one person by construction, and letting a quiet remote voice merge into
/// it would claim the user said something they did not.
pub fn assign_speakers(
    segments: &mut [TranscriptSegment],
    prints: &[TurnPrint],
    settings: &DetectionSettings,
    previous: &[Speaker],
) -> SpeakerReport {
    for segment in segments.iter_mut() {
        segment.speaker_id = None;
    }

    let mut groups: Vec<(SegmentChannel, Vec<u64>)> = Vec::new();

    for channel in [
        SegmentChannel::Microphone,
        SegmentChannel::System,
        SegmentChannel::Mixed,
    ] {
        let channel_prints: Vec<TurnPrint> = prints
            .iter()
            .filter(|print| {
                segments
                    .iter()
                    .any(|s| s.sequence == print.sequence && s.channel == channel)
            })
            .cloned()
            .collect();
        if channel_prints.is_empty() {
            continue;
        }

        // The local microphone carries one person. Clustering it would only
        // ever split a single speaker, never find a second one.
        let labels = if channel == SegmentChannel::Microphone {
            vec![0usize; channel_prints.len()]
        } else {
            voiceprint::cluster(&channel_prints, settings.split_distance, settings.max_speakers)
        };

        let distinct = labels.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        for label in 0..distinct {
            let members: Vec<u64> = labels
                .iter()
                .zip(&channel_prints)
                .filter(|(assigned, _)| **assigned == label)
                .map(|(_, print)| print.sequence)
                .collect();
            if !members.is_empty() {
                groups.push((channel, members));
            }
        }
    }

    // Ordered by who spoke first, so "Speaker 1" means the same thing to the
    // user as it does to the transcript they are reading.
    groups.sort_by_key(|(_, members)| members.iter().copied().min().unwrap_or(u64::MAX));

    let mut speakers = Vec::new();
    for (index, (channel, members)) in groups.iter().enumerate() {
        let id = format!("speaker-{}", index + 1);
        let lines: Vec<&TranscriptSegment> = segments
            .iter()
            .filter(|s| members.contains(&s.sequence))
            .collect();
        let speaking_seconds = lines.iter().map(|s| s.duration_seconds()).sum();
        let (sample_start, sample_end) = sample_span(&lines);

        speakers.push(Speaker {
            label: default_label(*channel, index),
            id,
            named_by_user: false,
            channel: *channel,
            sample_start_seconds: sample_start,
            sample_end_seconds: sample_end,
            segment_count: lines.len(),
            speaking_seconds,
        });
    }

    let speakers = carry_over_names(speakers, previous);

    for (speaker, (_, members)) in speakers.iter().zip(&groups) {
        for segment in segments.iter_mut() {
            if members.contains(&segment.sequence) {
                segment.speaker_id = Some(speaker.id.clone());
            }
        }
    }

    let attributed = segments.iter().filter(|s| s.speaker_id.is_some()).count();
    SpeakerReport {
        speakers,
        attributed,
        unattributed: segments.len() - attributed,
    }
}

/// The placeholder name a group starts with.
///
/// The microphone channel is named rather than numbered because it is the one
/// speaker Vox knows something about: it is whoever is running Vox.
fn default_label(channel: SegmentChannel, index: usize) -> String {
    match channel {
        SegmentChannel::Microphone => "You".to_string(),
        _ => format!("Speaker {}", index + 1),
    }
}

/// The span to play back when the user asks "who is this?".
///
/// The longest line the speaker has, clipped to [`SAMPLE_MAX_SECONDS`]. Longest
/// because a long line is a line where somebody talked without interruption,
/// which is exactly the recording you want in order to recognise a voice; the
/// clip keeps it to something a person will actually sit through.
///
/// Nothing is extracted to disk. The span points into the recording the
/// meeting already has, so a meeting with eight speakers costs eight pairs of
/// floats rather than eight audio files.
fn sample_span(lines: &[&TranscriptSegment]) -> (f64, f64) {
    let Some(longest) = lines
        .iter()
        .max_by(|a, b| a.duration_seconds().total_cmp(&b.duration_seconds()))
    else {
        return (0.0, 0.0);
    };
    let start = longest.start_seconds;
    let end = longest
        .end_seconds
        .min(start + SAMPLE_MAX_SECONDS)
        .max(start);
    (start, end)
}

/// Longest voice sample offered for identification.
pub const SAMPLE_MAX_SECONDS: f64 = 10.0;

/// Carries names the user typed across a re-run.
///
/// Matched by position in the speaking order, which is what the clusterer
/// sorts by. Not by cluster id, which is an artefact of merge order, and not
/// by fingerprint, which a new transcript's segments will not reproduce
/// exactly. Position is stable for the case that matters — the same meeting,
/// re-transcribed — and when it is wrong the cost is a rename, which is the
/// same cost as not carrying names at all.
fn carry_over_names(mut speakers: Vec<Speaker>, previous: &[Speaker]) -> Vec<Speaker> {
    for (speaker, old) in speakers.iter_mut().zip(previous) {
        if old.named_by_user && !old.label.trim().is_empty() {
            speaker.label = old.label.clone();
            speaker.named_by_user = true;
        }
    }
    speakers
}

/// The name to show against a transcript line.
pub fn label_for(segment: &TranscriptSegment, speakers: &[Speaker]) -> String {
    segment
        .speaker_id
        .as_deref()
        .and_then(|id| speakers.iter().find(|s| s.id == id))
        .map(|speaker| speaker.label.clone())
        // No attribution falls back to the channel, which is measured and
        // therefore always available: "Others" is less than a name and more
        // than nothing.
        .unwrap_or_else(|| segment.channel.label().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(sequence: u64, channel: SegmentChannel, start: f64, end: f64) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: format!("line {sequence}"),
            start_seconds: start,
            end_seconds: end,
            channel,
            no_speech_prob: 0.01,
            recorded_at: "2026-01-01T00:00:00Z".into(),
            original_text: None,
            romanized_text: None,
            translated_text: None,
            speaker_id: None,
        }
    }

    fn print(sequence: u64, vector: &[f32]) -> TurnPrint {
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
        TurnPrint {
            sequence,
            print: vector.iter().map(|v| v / norm).collect(),
        }
    }

    #[test]
    fn two_remote_voices_become_two_speakers() {
        let mut segments = vec![
            segment(0, SegmentChannel::System, 0.0, 3.0),
            segment(1, SegmentChannel::System, 3.0, 6.0),
            segment(2, SegmentChannel::System, 6.0, 9.0),
        ];
        let prints = vec![
            print(0, &[1.0, 0.0, 0.0]),
            print(1, &[0.0, 1.0, 0.0]),
            print(2, &[0.98, 0.02, 0.0]),
        ];
        let report = assign_speakers(&mut segments, &prints, &DetectionSettings::default(), &[]);

        assert_eq!(report.speakers.len(), 2);
        assert_eq!(report.attributed, 3);
        assert_eq!(segments[0].speaker_id, segments[2].speaker_id);
        assert_ne!(segments[0].speaker_id, segments[1].speaker_id);
    }

    #[test]
    fn the_local_microphone_is_never_split_into_several_people() {
        // One microphone carries one person. Clustering it can only ever
        // invent a second speaker for the same voice.
        let mut segments = vec![
            segment(0, SegmentChannel::Microphone, 0.0, 3.0),
            segment(1, SegmentChannel::Microphone, 3.0, 6.0),
        ];
        let prints = vec![print(0, &[1.0, 0.0, 0.0]), print(1, &[0.0, 1.0, 0.0])];
        let report = assign_speakers(&mut segments, &prints, &DetectionSettings::default(), &[]);

        assert_eq!(report.speakers.len(), 1);
        assert_eq!(report.speakers[0].label, "You");
        assert_eq!(segments[0].speaker_id, segments[1].speaker_id);
    }

    #[test]
    fn the_two_channels_are_never_merged_into_one_person() {
        // The failure this prevents: a quiet remote voice clustering with the
        // local one, so the transcript says the user said something they did
        // not.
        let mut segments = vec![
            segment(0, SegmentChannel::Microphone, 0.0, 3.0),
            segment(1, SegmentChannel::System, 3.0, 6.0),
        ];
        // Identical fingerprints, which is the worst case for this.
        let prints = vec![print(0, &[1.0, 0.0]), print(1, &[1.0, 0.0])];
        let report = assign_speakers(&mut segments, &prints, &DetectionSettings::default(), &[]);

        assert_eq!(report.speakers.len(), 2);
        assert_ne!(segments[0].speaker_id, segments[1].speaker_id);
    }

    #[test]
    fn a_line_with_no_fingerprint_is_left_unattributed() {
        let mut segments = vec![
            segment(0, SegmentChannel::System, 0.0, 3.0),
            segment(1, SegmentChannel::System, 3.0, 3.3),
        ];
        // Only the first line was long enough to fingerprint.
        let report = assign_speakers(
            &mut segments,
            &[print(0, &[1.0, 0.0])],
            &DetectionSettings::default(),
            &[],
        );

        assert_eq!(report.attributed, 1);
        assert_eq!(report.unattributed, 1);
        assert!(segments[1].speaker_id.is_none());
    }

    #[test]
    fn an_unattributed_line_still_reads_as_the_channel_it_came_from() {
        let segment = segment(0, SegmentChannel::System, 0.0, 1.0);
        assert_eq!(label_for(&segment, &[]), "Others");
    }

    #[test]
    fn an_attributed_line_reads_as_the_name_the_user_gave() {
        let mut segment = segment(0, SegmentChannel::System, 0.0, 1.0);
        segment.speaker_id = Some("speaker-1".into());
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
        assert_eq!(label_for(&segment, &speakers), "Payal");
    }

    #[test]
    fn the_sample_is_the_longest_uninterrupted_line_clipped_to_something_playable() {
        let long = segment(1, SegmentChannel::System, 30.0, 75.0);
        let short = segment(0, SegmentChannel::System, 0.0, 2.0);
        let (start, end) = sample_span(&[&short, &long]);
        assert_eq!(start, 30.0);
        assert_eq!(end, 30.0 + SAMPLE_MAX_SECONDS);
    }

    #[test]
    fn a_sample_shorter_than_the_clip_is_not_stretched_past_the_line() {
        let line = segment(0, SegmentChannel::System, 5.0, 8.0);
        assert_eq!(sample_span(&[&line]), (5.0, 8.0));
    }

    #[test]
    fn re_running_detection_keeps_the_names_the_user_typed() {
        let previous = vec![
            Speaker {
                id: "speaker-1".into(),
                label: "Payal".into(),
                named_by_user: true,
                channel: SegmentChannel::System,
                sample_start_seconds: 0.0,
                sample_end_seconds: 4.0,
                segment_count: 2,
                speaking_seconds: 6.0,
            },
            Speaker {
                id: "speaker-2".into(),
                label: "Speaker 2".into(),
                named_by_user: false,
                channel: SegmentChannel::System,
                sample_start_seconds: 4.0,
                sample_end_seconds: 8.0,
                segment_count: 1,
                speaking_seconds: 3.0,
            },
        ];
        let mut segments = vec![
            segment(0, SegmentChannel::System, 0.0, 3.0),
            segment(1, SegmentChannel::System, 3.0, 6.0),
        ];
        let prints = vec![print(0, &[1.0, 0.0]), print(1, &[0.0, 1.0])];
        let report = assign_speakers(&mut segments, &prints, &DetectionSettings::default(), &previous);

        assert_eq!(report.speakers[0].label, "Payal", "a typed name survives");
        assert!(report.speakers[0].named_by_user);
        assert_eq!(
            report.speakers[1].label, "Speaker 2",
            "a placeholder is Vox's to renumber"
        );
        assert!(!report.speakers[1].named_by_user);
    }

    #[test]
    fn detection_with_no_fingerprints_at_all_finds_nobody_and_breaks_nothing() {
        let mut segments = vec![segment(0, SegmentChannel::System, 0.0, 1.0)];
        let report = assign_speakers(&mut segments, &[], &DetectionSettings::default(), &[]);
        assert!(report.speakers.is_empty());
        assert_eq!(report.attributed, 0);
        assert_eq!(report.unattributed, 1);
    }

    #[test]
    fn a_second_run_clears_attributions_it_no_longer_believes() {
        let mut segments = vec![segment(0, SegmentChannel::System, 0.0, 3.0)];
        segments[0].speaker_id = Some("speaker-9".into());
        let report = assign_speakers(&mut segments, &[], &DetectionSettings::default(), &[]);
        assert!(
            segments[0].speaker_id.is_none(),
            "a stale attribution must not outlive the run that made it"
        );
        assert_eq!(report.attributed, 0);
    }

    #[test]
    fn speakers_are_numbered_by_who_spoke_first() {
        let mut segments = vec![
            segment(0, SegmentChannel::System, 0.0, 3.0),
            segment(1, SegmentChannel::System, 3.0, 6.0),
        ];
        let prints = vec![print(0, &[0.0, 1.0]), print(1, &[1.0, 0.0])];
        let report = assign_speakers(&mut segments, &prints, &DetectionSettings::default(), &[]);
        assert_eq!(segments[0].speaker_id.as_deref(), Some("speaker-1"));
        assert_eq!(report.speakers[0].id, "speaker-1");
        assert_eq!(report.speakers[0].sample_start_seconds, 0.0);
    }
}
