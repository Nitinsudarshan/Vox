//! Where meetings live on disk, and how they survive a crash.
//!
//! One directory per meeting under `<vault>/meetings/`, the same shape the
//! vault already uses for imported files and web captures:
//!
//! ```text
//! <vault>/meetings/<meeting_id>/
//! ├── meeting.json      metadata — the record the list surface reads
//! ├── transcript.json   Vec<TranscriptSegment>, ordered by sequence
//! ├── summary.json      the generated report and its cache fingerprint
//! ├── notes.md          whatever the user typed during the meeting
//! └── audio/
//!     ├── chunk_000000.wav   durable 30s checkpoints, during recording
//!     └── audio.wav          the merged recording, written on stop
//! ```
//!
//! Files rather than a database, because that is what the rest of this vault
//! is: a meeting stays greppable, syncable and readable without Vox running.
//! Meetily reaches for SQLite here and then leaves
//! `transcripts.meeting_id` — the column every one of its queries filters and
//! orders on — unindexed, so the database buys it schema without buying it
//! speed.
//!
//! Every write that matters goes to a temporary file and is renamed into
//! place. A half-written `transcript.json` is worse than a stale one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use crate::sync::MutexExt;

use super::model::{
    Meeting, MeetingListItem, MeetingState, MeetingSummary, SummaryStatus, TranscriptSegment,
};

/// Directory under the vault root that holds every meeting.
pub const MEETINGS_DIR: &str = "meetings";

const MEETING_FILE: &str = "meeting.json";
const TRANSCRIPT_FILE: &str = "transcript.json";
const SUMMARY_FILE: &str = "summary.json";
const NOTES_FILE: &str = "notes.md";
const SPEAKERS_FILE: &str = "speakers.json";
const AUDIO_DIR: &str = "audio";

/// Characters in the preview shown on a meeting list row.
const PREVIEW_CHARS: usize = 200;

#[derive(Debug, Error)]
pub enum MeetingStoreError {
    #[error("meeting storage IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("meeting record could not be read or written: {0}")]
    Serde(String),

    #[error("no meeting with id {0}")]
    NotFound(String),

    #[error("meeting id {0} is not a valid identifier")]
    InvalidId(String),
}

impl From<serde_json::Error> for MeetingStoreError {
    fn from(err: serde_json::Error) -> Self {
        MeetingStoreError::Serde(err.to_string())
    }
}

/// Reads and writes meetings under a vault root.
///
/// The root is behind a `Mutex` for the same reason [`crate::vault::VaultManager`]'s
/// is: the user can repoint their vault at runtime, and every method here only
/// ever needed `&self`.
pub struct MeetingStore {
    vault_dir: Mutex<PathBuf>,
}

impl MeetingStore {
    pub fn new(vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_dir: Mutex::new(vault_dir.into()),
        }
    }

    /// Repoints the store at a new vault root. Meetings at the old location
    /// stay where they are, matching how the rest of the vault behaves.
    pub fn set_vault_dir(&self, new_dir: impl Into<PathBuf>) {
        *self.vault_dir.lock_or_recover() = new_dir.into();
    }

    /// The `meetings/` directory under the current vault root.
    pub fn meetings_dir(&self) -> PathBuf {
        self.vault_dir.lock_or_recover().join(MEETINGS_DIR)
    }

    /// One meeting's directory. Rejects an id that could escape the vault.
    pub fn meeting_dir(&self, id: &str) -> Result<PathBuf, MeetingStoreError> {
        if !is_safe_id(id) {
            return Err(MeetingStoreError::InvalidId(id.to_string()));
        }
        Ok(self.meetings_dir().join(id))
    }

    /// Where a meeting's audio chunks and merged recording live.
    pub fn audio_dir(&self, id: &str) -> Result<PathBuf, MeetingStoreError> {
        Ok(self.meeting_dir(id)?.join(AUDIO_DIR))
    }

    /// Creates the directory tree for a new meeting and writes its record.
    pub fn create(&self, meeting: &Meeting) -> Result<PathBuf, MeetingStoreError> {
        let dir = self.meeting_dir(&meeting.id)?;
        fs::create_dir_all(dir.join(AUDIO_DIR))?;
        self.save_meeting(meeting)?;
        self.save_transcript(&meeting.id, &[])?;
        Ok(dir)
    }

    /// Writes a meeting record, stamping `updated_at`.
    pub fn save_meeting(&self, meeting: &Meeting) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(&meeting.id)?;
        fs::create_dir_all(&dir)?;
        let mut record = meeting.clone();
        record.touch();
        write_atomic(&dir.join(MEETING_FILE), &serde_json::to_vec_pretty(&record)?)
    }

    pub fn load_meeting(&self, id: &str) -> Result<Meeting, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(MEETING_FILE);
        if !path.exists() {
            return Err(MeetingStoreError::NotFound(id.to_string()));
        }
        Ok(serde_json::from_slice(&fs::read(&path)?)?)
    }

    /// Applies `edit` to a stored meeting and writes it back.
    ///
    /// The one mutation path, so "read, change one field, write" cannot
    /// accidentally drop a field another thread just set.
    pub fn update_meeting<F>(&self, id: &str, edit: F) -> Result<Meeting, MeetingStoreError>
    where
        F: FnOnce(&mut Meeting),
    {
        let mut meeting = self.load_meeting(id)?;
        edit(&mut meeting);
        self.save_meeting(&meeting)?;
        self.load_meeting(id)
    }

    /// Every meeting, newest first.
    ///
    /// A directory that fails to parse is skipped with a warning rather than
    /// failing the whole listing — one corrupt meeting must not hide the
    /// other fifty.
    pub fn list_meetings(&self) -> Result<Vec<Meeting>, MeetingStoreError> {
        let dir = self.meetings_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut meetings = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let record = entry.path().join(MEETING_FILE);
            if !record.exists() {
                continue;
            }
            match fs::read(&record).map_err(MeetingStoreError::from).and_then(|bytes| {
                serde_json::from_slice::<Meeting>(&bytes).map_err(MeetingStoreError::from)
            }) {
                Ok(meeting) => meetings.push(meeting),
                Err(err) => {
                    tracing::warn!("skipping unreadable meeting at {:?}: {}", record, err);
                }
            }
        }
        meetings.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(meetings)
    }

    /// The list surface's view: metadata plus whether a summary exists and a
    /// short transcript preview, so the UI never fetches a full transcript to
    /// render a row.
    pub fn list_for_display(&self) -> Result<Vec<MeetingListItem>, MeetingStoreError> {
        let meetings = self.list_meetings()?;
        let mut items = Vec::with_capacity(meetings.len());
        for meeting in meetings {
            let summary = self.load_summary(&meeting.id).ok().flatten();
            let preview = self
                .load_transcript(&meeting.id)
                .map(|segments| transcript_preview(&segments, PREVIEW_CHARS))
                .unwrap_or_default();
            items.push(MeetingListItem {
                has_summary: summary
                    .as_ref()
                    .is_some_and(|s| s.status == SummaryStatus::Completed),
                summary_status: summary.as_ref().map(|s| s.status),
                preview,
                meeting,
            });
        }
        Ok(items)
    }

    pub fn save_transcript(
        &self,
        id: &str,
        segments: &[TranscriptSegment],
    ) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join(TRANSCRIPT_FILE),
            &serde_json::to_vec_pretty(segments)?,
        )
    }

    /// A meeting's transcript, ordered by sequence.
    ///
    /// Sorting on read rather than trusting the file means a transcript
    /// written by an interrupted append is still in the right order.
    pub fn load_transcript(&self, id: &str) -> Result<Vec<TranscriptSegment>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(TRANSCRIPT_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut segments: Vec<TranscriptSegment> = serde_json::from_slice(&fs::read(&path)?)?;
        segments.sort_by_key(|segment| segment.sequence);
        segments.dedup_by_key(|segment| segment.sequence);
        Ok(segments)
    }

    /// Adds segments to a stored transcript, keeping it sequence-ordered and
    /// free of duplicates.
    ///
    /// Rewrites the whole file. That is the right trade at meeting scale — a
    /// three-hour meeting is a few hundred kilobytes of JSON — and it is what
    /// keeps the file atomically replaceable.
    pub fn append_segments(
        &self,
        id: &str,
        new_segments: &[TranscriptSegment],
    ) -> Result<usize, MeetingStoreError> {
        if new_segments.is_empty() {
            return Ok(self.load_transcript(id)?.len());
        }
        let mut segments = self.load_transcript(id)?;
        segments.extend_from_slice(new_segments);
        segments.sort_by_key(|segment| segment.sequence);
        segments.dedup_by_key(|segment| segment.sequence);
        let total = segments.len();
        self.save_transcript(id, &segments)?;
        Ok(total)
    }

    pub fn save_summary(&self, summary: &MeetingSummary) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(&summary.meeting_id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(SUMMARY_FILE), &serde_json::to_vec_pretty(summary)?)
    }

    pub fn load_summary(&self, id: &str) -> Result<Option<MeetingSummary>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(SUMMARY_FILE);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&fs::read(&path)?)?))
    }

    /// Free-text notes the user typed against a meeting.
    /// The speakers a detection run proposed, and the names the user gave them.
    ///
    /// Its own file rather than a field on `meeting.json`: a detection run
    /// rewrites this wholesale and must not be able to lose the meeting's
    /// metadata by failing halfway.
    pub fn save_speakers(
        &self,
        id: &str,
        speakers: &[crate::meetings::model::Speaker],
    ) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join(SPEAKERS_FILE),
            &serde_json::to_vec_pretty(speakers)?,
        )
    }

    /// A meeting's speakers, or none where detection has never run.
    ///
    /// A file that cannot be parsed reads as "no speakers" rather than as an
    /// error: the transcript is the product, and a meeting must still open
    /// when a detection run left a half-written file behind.
    pub fn load_speakers(
        &self,
        id: &str,
    ) -> Result<Vec<crate::meetings::model::Speaker>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(SPEAKERS_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_slice(&fs::read(&path)?).unwrap_or_default())
    }

    pub fn load_notes(&self, id: &str) -> Result<String, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(NOTES_FILE);
        if !path.exists() {
            return Ok(String::new());
        }
        Ok(fs::read_to_string(&path)?)
    }

    pub fn save_notes(&self, id: &str, notes: &str) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(NOTES_FILE), notes.as_bytes())
    }

    /// Deletes a meeting's entire directory, audio included.
    ///
    /// Meetily deletes the database rows and leaves `audio.mp4` on disk
    /// forever, so a user who deletes a sensitive meeting has not deleted the
    /// recording. Here the directory is the meeting; removing it removes all
    /// of it.
    pub fn delete_meeting(&self, id: &str) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        if !dir.exists() {
            return Err(MeetingStoreError::NotFound(id.to_string()));
        }
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Case-insensitive substring search across every transcript, returning
    /// each hit with surrounding context.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MeetingSearchHit>, MeetingStoreError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for meeting in self.list_meetings()? {
            let segments = self.load_transcript(&meeting.id).unwrap_or_default();
            for segment in &segments {
                if segment.text.to_lowercase().contains(&needle) {
                    hits.push(MeetingSearchHit {
                        meeting_id: meeting.id.clone(),
                        meeting_title: meeting.title.clone(),
                        sequence: segment.sequence,
                        start_seconds: segment.start_seconds,
                        excerpt: excerpt_around(&segment.text, &needle, 120),
                    });
                    if hits.len() >= limit {
                        return Ok(hits);
                    }
                }
            }
        }
        Ok(hits)
    }

    /// Finalizes meetings a crash left mid-recording.
    ///
    /// A meeting still marked `Recording` or `Paused` at launch cannot be
    /// resumed — its capture threads and its in-memory queue died with the
    /// process. What survives is its durable audio chunks and whatever
    /// transcript was flushed. This walks those, recomputes the true duration
    /// from the audio on disk, and marks the meeting `Completed` so it shows
    /// up as the (partial) record it is rather than as a recording that never
    /// ends.
    ///
    /// Returns the ids it recovered.
    pub fn recover_interrupted(&self) -> Result<Vec<String>, MeetingStoreError> {
        let mut recovered = Vec::new();
        for meeting in self.list_meetings()? {
            if let Ok(Some(mut summary)) = self.load_summary(&meeting.id) {
                if summary.is_running() {
                    summary.status = SummaryStatus::Failed;
                    summary.error = Some(
                        "Generation was interrupted when Vox closed or restarted."
                            .to_string(),
                    );
                    summary.markdown = summary.previous_markdown.take().or(summary.markdown);
                    summary.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    let _ = self.save_summary(&summary);
                }
            }

            if !meeting.state.is_interrupted() {
                continue;
            }
            let audio_dir = self.audio_dir(&meeting.id)?;
            let chunks = list_chunks(&audio_dir);
            let duration = chunks
                .iter()
                .filter_map(|path| wav_duration_seconds(path))
                .sum::<f64>();
            let segments = self.load_transcript(&meeting.id).unwrap_or_default();

            let id = meeting.id.clone();
            self.update_meeting(&id, |record| {
                record.state = MeetingState::Completed;
                // A duration measured from the audio that actually reached
                // disk, not the clock the dead process was keeping.
                if duration > 0.0 {
                    record.duration_seconds = duration;
                }
                record.segment_count = segments.len();
                record.error = Some(
                    "Recovered after Vox closed unexpectedly. Audio and transcript are complete \
                     up to the last saved checkpoint."
                        .to_string(),
                );
            })?;
            tracing::info!(
                "recovered interrupted meeting {} ({} chunks, {:.1}s)",
                id,
                chunks.len(),
                duration
            );
            recovered.push(id);
        }
        Ok(recovered)
    }
}

/// One transcript match, with enough context to render a result row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeetingSearchHit {
    pub meeting_id: String,
    pub meeting_title: String,
    pub sequence: u64,
    pub start_seconds: f64,
    pub excerpt: String,
}

/// Whether an id is safe to join onto the vault path.
///
/// Ids are generated, so this only ever rejects a caller that made one up —
/// but a command boundary takes ids from the frontend, and meetily's template
/// loader shows what the missing check costs: `template_id` is joined into a
/// path with no traversal guard (`templates/loader.rs:41`), so `../../…`
/// reaches any file the user can read.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Writes bytes to `path` through a temporary file in the same directory.
///
/// Same-directory, because a rename across filesystems is a copy and stops
/// being atomic.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), MeetingStoreError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension(format!(
        "tmp{}",
        std::process::id()
    ));
    fs::write(&tmp, bytes)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err.into())
        }
    }
}

/// A meeting's durable audio checkpoints, in capture order.
pub fn list_chunks(audio_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(audio_dir) else {
        return Vec::new();
    };
    let mut chunks: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("chunk_") && name.ends_with(".wav"))
        })
        .collect();
    chunks.sort();
    chunks
}

/// How long a 16-bit PCM WAV runs, read from its header rather than decoded.
fn wav_duration_seconds(path: &Path) -> Option<f64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let frames = reader.len() as f64 / spec.channels.max(1) as f64;
    Some(frames / spec.sample_rate.max(1) as f64)
}

/// The first `max_chars` of a transcript, cut on a character boundary.
fn transcript_preview(segments: &[TranscriptSegment], max_chars: usize) -> String {
    let mut preview = String::new();
    for segment in segments {
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(segment.text.trim());
        if preview.chars().count() >= max_chars {
            break;
        }
    }
    let truncated: String = preview.chars().take(max_chars).collect();
    if truncated.chars().count() < preview.chars().count() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// `context` characters of text either side of the first match of `needle`.
fn excerpt_around(text: &str, needle_lower: &str, context: usize) -> String {
    let lowered = text.to_lowercase();
    let Some(byte_index) = lowered.find(needle_lower) else {
        return text.chars().take(context * 2).collect();
    };
    // Byte offsets from the lowercased copy can land mid-character in the
    // original for any non-ASCII text, so convert to a character index before
    // slicing anything.
    let char_index = lowered[..byte_index].chars().count();
    let start = char_index.saturating_sub(context);
    let chars: Vec<char> = text.chars().collect();
    let end = (char_index + needle_lower.chars().count() + context).min(chars.len());
    let mut excerpt = String::new();
    if start > 0 {
        excerpt.push('…');
    }
    excerpt.extend(&chars[start..end]);
    if end < chars.len() {
        excerpt.push('…');
    }
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::model::{MeetingSource, SegmentChannel};

    fn temp_vault(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-meetings-{}-{}-{:?}",
            name,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp vault");
        dir
    }

    fn segment(sequence: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: text.to_string(),
            start_seconds: sequence as f64,
            end_seconds: sequence as f64 + 1.0,
            channel: SegmentChannel::Mixed,
            no_speech_prob: 0.01,
            recorded_at: "2026-01-01T00:00:00Z".into(),
            original_text: None,
            romanized_text: None,
            translated_text: None,
            speaker_id: None,
        }
    }

    #[test]
    fn a_created_meeting_round_trips_through_disk() {
        let vault = temp_vault("create");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-a".into(), "Weekly".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");

        let loaded = store.load_meeting("meeting-a").expect("load");
        assert_eq!(loaded.title, "Weekly");
        assert_eq!(loaded.state, MeetingState::Recording);
        assert!(store.audio_dir("meeting-a").unwrap().exists());
    }

    #[test]
    fn appended_segments_come_back_ordered_and_deduplicated() {
        let vault = temp_vault("append");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-b".into(), "Sync".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");

        store.append_segments("meeting-b", &[segment(3, "third")]).unwrap();
        store.append_segments("meeting-b", &[segment(1, "first")]).unwrap();
        // The same sequence arriving twice is the pipeline retrying, not a
        // second thing that was said.
        let total = store
            .append_segments("meeting-b", &[segment(3, "third again"), segment(2, "second")])
            .unwrap();

        assert_eq!(total, 3);
        let transcript = store.load_transcript("meeting-b").unwrap();
        let sequences: Vec<u64> = transcript.iter().map(|s| s.sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3]);
        assert_eq!(transcript[2].text, "third");
    }

    #[test]
    fn an_id_that_climbs_out_of_the_vault_is_refused() {
        let vault = temp_vault("traversal");
        let store = MeetingStore::new(&vault);
        for bad in ["../escape", "..", "a/b", "meeting id", ""] {
            assert!(
                matches!(store.meeting_dir(bad), Err(MeetingStoreError::InvalidId(_))),
                "{bad:?} should be refused"
            );
        }
        assert!(store.meeting_dir("meeting-abc_123").is_ok());
    }

    #[test]
    fn deleting_a_meeting_takes_its_audio_with_it() {
        let vault = temp_vault("delete");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-c".into(), "Gone".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        let audio = store.audio_dir("meeting-c").unwrap().join("audio.wav");
        fs::write(&audio, b"not really a wav").unwrap();

        store.delete_meeting("meeting-c").expect("delete");

        assert!(!audio.exists());
        assert!(!store.meeting_dir("meeting-c").unwrap().exists());
        assert!(matches!(
            store.delete_meeting("meeting-c"),
            Err(MeetingStoreError::NotFound(_))
        ));
    }

    #[test]
    fn recovery_completes_an_interrupted_meeting_and_leaves_finished_ones_alone() {
        let vault = temp_vault("recover");
        let store = MeetingStore::new(&vault);

        let crashed = Meeting::new("meeting-d".into(), "Crashed".into(), MeetingSource::Recorded);
        store.create(&crashed).expect("create");
        store.append_segments("meeting-d", &[segment(1, "we were saying")]).unwrap();

        let mut finished = Meeting::new("meeting-e".into(), "Fine".into(), MeetingSource::Recorded);
        finished.state = MeetingState::Completed;
        finished.duration_seconds = 42.0;
        store.create(&finished).expect("create");
        store.save_meeting(&finished).expect("save completed");

        let recovered = store.recover_interrupted().expect("recover");

        assert_eq!(recovered, vec!["meeting-d".to_string()]);
        let d = store.load_meeting("meeting-d").unwrap();
        assert_eq!(d.state, MeetingState::Completed);
        assert_eq!(d.segment_count, 1);
        assert!(d.error.is_some(), "recovery should say the meeting was interrupted");
        let e = store.load_meeting("meeting-e").unwrap();
        assert_eq!(e.duration_seconds, 42.0);
        assert!(e.error.is_none());
    }

    #[test]
    fn recovery_recomputes_duration_from_the_audio_that_reached_disk() {
        let vault = temp_vault("recover-duration");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-f".into(), "Crashed".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");

        // Two seconds of 16 kHz mono, written as one checkpoint.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let chunk = store.audio_dir("meeting-f").unwrap().join("chunk_000000.wav");
        let mut writer = hound::WavWriter::create(&chunk, spec).unwrap();
        for _ in 0..32_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();

        store.recover_interrupted().expect("recover");

        let recovered = store.load_meeting("meeting-f").unwrap();
        assert!((recovered.duration_seconds - 2.0).abs() < 0.01);
    }

    #[test]
    fn search_finds_a_phrase_and_returns_it_in_context() {
        let vault = temp_vault("search");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-g".into(), "Planning".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        store
            .append_segments(
                "meeting-g",
                &[segment(1, "we should ship the migration before the holiday freeze")],
            )
            .unwrap();

        let hits = store.search("MIGRATION", 10).expect("search");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, "meeting-g");
        assert!(hits[0].excerpt.contains("migration"));
        assert!(store.search("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn search_context_does_not_split_a_multibyte_character() {
        // The needle's byte offset comes from a lowercased copy; slicing the
        // original on it would panic the moment anything non-ASCII precedes
        // the match.
        let excerpt = excerpt_around("मुझे migration चाहिए", "migration", 5);
        assert!(excerpt.contains("migration"));
    }

    #[test]
    fn a_listing_skips_an_unreadable_meeting_instead_of_failing() {
        let vault = temp_vault("corrupt");
        let store = MeetingStore::new(&vault);
        let good = Meeting::new("meeting-h".into(), "Good".into(), MeetingSource::Recorded);
        store.create(&good).expect("create");

        let bad_dir = store.meetings_dir().join("meeting-i");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join(MEETING_FILE), b"{ not json").unwrap();

        let meetings = store.list_meetings().expect("list");
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].id, "meeting-h");
    }

    #[test]
    fn a_display_listing_reports_summary_state_and_a_preview() {
        let vault = temp_vault("display");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-j".into(), "Retro".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        store.append_segments("meeting-j", &[segment(1, "what went well")]).unwrap();
        let mut summary = MeetingSummary::pending("meeting-j", "default");
        summary.status = SummaryStatus::Completed;
        summary.markdown = Some("# Retro".into());
        store.save_summary(&summary).unwrap();

        let items = store.list_for_display().expect("list");

        assert_eq!(items.len(), 1);
        assert!(items[0].has_summary);
        assert_eq!(items[0].summary_status, Some(SummaryStatus::Completed));
        assert_eq!(items[0].preview, "what went well");
    }

    #[test]
    fn a_preview_is_cut_on_a_character_boundary() {
        let segments = vec![segment(1, &"नमस्ते ".repeat(60))];
        let preview = transcript_preview(&segments, 20);
        assert!(preview.chars().count() <= 21, "20 chars plus the ellipsis");
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn an_atomic_write_leaves_no_temporary_file_behind() {
        let vault = temp_vault("atomic");
        let target = vault.join("thing.json");
        write_atomic(&target, b"{}").expect("write");
        assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
        let leftovers: Vec<_> = fs::read_dir(&vault)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary file was not renamed away");
    }

    #[test]
    fn notes_default_to_empty_rather_than_missing() {
        let vault = temp_vault("notes");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-k".into(), "Notes".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        assert_eq!(store.load_notes("meeting-k").unwrap(), "");
        store.save_notes("meeting-k", "- follow up with ops").unwrap();
        assert_eq!(store.load_notes("meeting-k").unwrap(), "- follow up with ops");
    }
}
